//! Sign grants the badge issues and honours: an agent's key may sign git commits as a named
//! author, a fixed number of times, until a time or until revoked.
//!
//! [`issue`] makes a grant after the holder allowed it on the screen: the seat key signs it, a
//! counter for it starts at zero in the chip, and its record goes to flash. [`evaluate_request`]
//! decides whether a request under a grant is signed without asking, asked about as an
//! ordinary signature ([`Fallback::Prompt`]), or refused ([`Fallback::Refuse`]);
//! [`sign_under_grant`] counts the use and signs. [`sweep`] runs at boot and removes what is
//! spent, expired or half there.
//!
//! On a badge with a chip the count is the chip's counter for the grant's slot and nothing
//! else: a flash image restored over the record cannot refund a use. Each grant's counter
//! starts at its `base`, the chip-wide issue counter ([`ISSUE_SLOT`]) at issue shifted up 32
//! bits, so a grant's uses are its counter less its base. The base is signed into the grant
//! (the `cahoots-base` claim) and read from the verified grant, never from flash: a record
//! whose base is not the grant's is refused as unknown. A later grant in the same slot has a
//! base at least 2^32 higher, so an older grant reads its slot's counter as far past any
//! budget and is refused, and never spends the later grant's count. The issue counter is kept
//! through a wipe, so no base is given twice on one chip. A badge without a chip counts in the
//! record (`used_soft`), and its grants have base 0.
//!
//! The badge has a timer, not a clock: [`Clock`] keeps the latest time a laptop told it, plus
//! the uptime since, and never goes back. A laptop can make a grant end early, never late.
//!
//! The record is CBOR: `[known_time, [grant, ...]]`, each grant an array of the
//! [`GrantRecord`] fields in order. The decoder re-encodes and refuses anything that differs.

use crate::apps::seat_of;
use crate::board::{Board, BoardError, Circle};
use crate::chip::Malformed;
use crate::ssh::{self, Signed};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_chip::{ChipOps, KeyId, KeyKind};
use cahoots_core::circle_id::CircleId;
use cahoots_core::seat::SeatPublic;
use cahoots_custodian::wire::{ErrorCode, GrantRequestFields, UnderGrantFields};
use cahoots_hal::Entropy;
use cahoots_objects::authz::{
    self, Action, Context, EvalRequest, Reason, Resource, SignGrant, Subject,
};
use cahoots_objects::cwt::Claims;
use cahoots_objects::grant::key_id;
use minicbor::{Decode, Encode};
use sha2::{Digest, Sha256};

/// The most grants a badge holds, live or revoked.
pub const GRANTS_MAX: usize = 8;
/// The most signatures one grant allows.
pub const BUDGET_MAX: u32 = 1000;
/// The stored time is rewritten when it is more than this many seconds behind.
pub const TIME_WRITE_STEP: u64 = 3600;
/// The longest agent name, in bytes. With [`AUTHOR_MAX`] and [`LOCATION_MAX`] it keeps a full
/// record within one 4 KiB flash sector.
pub const AGENT_MAX: usize = 64;
/// The longest author, in bytes.
pub const AUTHOR_MAX: usize = 128;
/// The longest location, in bytes.
pub const LOCATION_MAX: usize = 128;
/// The chip slot of the issue counter: one chip-wide counter, moved by one at every issue,
/// which gives each grant's counter a base no earlier grant in the same slot had. Never a
/// grant's slot.
pub const ISSUE_SLOT: u8 = 255;
const _: () = assert!(GRANTS_MAX <= ISSUE_SLOT as usize);

/// One grant the badge issued.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct GrantRecord {
    /// The slot of its counter in the chip, `0..GRANTS_MAX`.
    #[n(0)]
    pub slot: u8,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub cti: [u8; 16],
    #[cbor(n(2), with = "minicbor::bytes")]
    pub circle_id: CircleId,
    #[n(3)]
    pub seat: u8,
    /// The id of the agent's key.
    #[cbor(n(4), with = "minicbor::bytes")]
    pub key_id: [u8; 32],
    #[n(5)]
    pub agent: String,
    #[n(6)]
    pub action: String,
    #[n(7)]
    pub author: String,
    #[n(8)]
    pub locations: Vec<String>,
    #[n(9)]
    pub budget: u32,
    #[n(10)]
    pub exp: Option<u64>,
    /// Revoked on the badge: the record stays so its `cti` stays refused.
    #[n(11)]
    pub revoked: bool,
    /// Uses so far, on a badge without a chip. A badge with one counts in the chip.
    #[n(12)]
    pub used_soft: u64,
    /// What the grant's chip counter was set to at issue: the issue number shifted up 32 bits.
    /// 0 without a chip. A copy of the grant's signed base: a record that differs from the
    /// grant is refused, and the count is read against the grant's.
    #[n(13)]
    pub base: u64,
}

/// Every grant the badge holds, and the latest time it was told.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct GrantsRecord {
    #[n(0)]
    pub known_time: u64,
    #[n(1)]
    pub grants: Vec<GrantRecord>,
}

impl GrantsRecord {
    pub fn encode(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("encoding into a Vec cannot fail")
    }

    /// Strict: exactly one canonical encoding, at most [`GRANTS_MAX`] grants, each in its own
    /// slot below [`GRANTS_MAX`].
    pub fn decode(bytes: &[u8]) -> Result<GrantsRecord, Malformed> {
        let r: GrantsRecord = minicbor::decode(bytes).map_err(|_| Malformed)?;
        let slots_ok = r.grants.iter().enumerate().all(|(i, g)| {
            usize::from(g.slot) < GRANTS_MAX && r.grants[..i].iter().all(|h| h.slot != g.slot)
        });
        if r.encode() != bytes || r.grants.len() > GRANTS_MAX || !slots_ok {
            return Err(Malformed);
        }
        Ok(r)
    }

    /// The lowest slot no grant uses, if there is one.
    pub fn next_free_slot(&self) -> Option<u8> {
        (0..GRANTS_MAX as u8).find(|&s| self.grants.iter().all(|g| g.slot != s))
    }

    /// How many grants are not revoked.
    pub fn live(&self) -> usize {
        self.grants.iter().filter(|g| !g.revoked).count()
    }
}

/// The badge's best time: `known_time = max(stored, told + (uptime - told_at_uptime))`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// The time in the stored record.
    pub stored: u64,
    /// The latest time a laptop told the badge, or the stored time at boot.
    pub told: u64,
    /// The uptime when `told` was learned.
    pub told_at_uptime: u64,
}

impl Clock {
    /// The clock at boot: the stored time, counting from `uptime`.
    pub fn new(stored: u64, uptime: u64) -> Clock {
        Clock {
            stored,
            told: stored,
            told_at_uptime: uptime,
        }
    }

    pub fn known_time(&self, uptime: u64) -> u64 {
        let since = uptime.saturating_sub(self.told_at_uptime);
        self.stored.max(self.told.saturating_add(since))
    }

    /// A laptop says it is `time`. A time earlier than the badge's is ignored.
    pub fn raise(&mut self, time: u64, uptime: u64) {
        if time > self.known_time(uptime) {
            self.told = time;
            self.told_at_uptime = uptime;
        }
    }

    /// The stored time is more than [`TIME_WRITE_STEP`] behind.
    pub fn stale(&self, uptime: u64) -> bool {
        self.known_time(uptime) - self.stored > TIME_WRITE_STEP
    }
}

/// What the grant functions need of a board. Every [`Board`] is one.
pub trait GrantBoard {
    fn chip(&mut self) -> Option<&mut dyn ChipOps>;
    fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError>;
    fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError>;
    /// Fill `buf` from the board's random source; false if it failed its health tests.
    fn fill_random(&mut self, buf: &mut [u8]) -> bool;
    fn log(&mut self, line: &str);
    fn uptime_secs(&self) -> u64;
}

impl<B: Board> GrantBoard for B {
    fn chip(&mut self) -> Option<&mut dyn ChipOps> {
        Board::chip(self)
    }
    fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError> {
        Board::grants_load(self)
    }
    fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError> {
        Board::grants_store(self, r)
    }
    fn fill_random(&mut self, buf: &mut [u8]) -> bool {
        self.entropy().fill(buf).is_ok()
    }
    fn log(&mut self, line: &str) {
        Board::log(self, line)
    }
    fn uptime_secs(&self) -> u64 {
        Board::uptime_secs(self)
    }
}

/// Why a request under a grant is not signed at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// Ask the holder, as for an ordinary signature. The code is what the log says it was.
    Prompt(ErrorCode),
    /// Refuse with this code, asking nobody.
    Refuse(ErrorCode),
}

/// A request the grant allows: the grant, its record's place in the record, the uses before
/// this one, and the commit's subject.
#[derive(Clone, Debug)]
pub struct Allowed {
    pub grant: SignGrant,
    pub index: usize,
    pub used: u64,
    pub subject: String,
}

/// A signature made under a grant, for the board to show and log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantUse {
    pub cti: [u8; 16],
    pub agent: String,
    pub subject: String,
    pub remaining: u32,
    pub budget: u32,
}

/// The first four bytes of a grant's id, in hex, for the log.
pub fn short(cti: &[u8; 16]) -> String {
    cti[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn counter_id(slot: u8) -> KeyId {
    KeyId::new(KeyKind::Grant, slot)
}

/// Move the issue counter on by one, making it at 0 first if the chip has none, and return
/// its new value: this issue's number, never given before on this chip.
fn next_issue(chip: &mut dyn ChipOps) -> Result<u64, ErrorCode> {
    let id = counter_id(ISSUE_SLOT);
    if !chip.exists(id).map_err(|_| ErrorCode::ChipError)? {
        chip.put_counter(id, 0).map_err(|_| ErrorCode::ChipError)?;
    }
    chip.counter_next(id).map_err(|_| ErrorCode::ChipError)
}

/// A base the badge gives on a chip: a nonzero issue number shifted up 32 bits. Only one such
/// base lies within a budget below any counter value, so the chip alone decides the count
/// whatever the flash says.
fn base_ok(base: u64) -> bool {
    base != 0 && base & 0xFFFF_FFFF == 0
}

/// The uses a chip counter reading `raw` stands for under a grant with `base` and `budget`,
/// or `None` when the counter cannot be this grant's: below its base, or past its budget
/// (a later grant in the same slot, whose base is at least 2^32 higher).
fn uses(raw: u64, base: u64, budget: u32) -> Option<u64> {
    raw.checked_sub(base).filter(|&n| n <= u64::from(budget))
}

/// The stored record, empty if there is none. An unreadable one is `Storage`, logged.
fn load<G: GrantBoard + ?Sized>(board: &mut G) -> Result<GrantsRecord, ErrorCode> {
    match board.grants_load() {
        Ok(r) => Ok(r.unwrap_or_default()),
        Err(_) => {
            board.log("grants record: unreadable");
            Err(ErrorCode::Storage)
        }
    }
}

/// Store `record` with the clock's time in it.
fn store<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &mut Clock,
    record: &mut GrantsRecord,
) -> Result<(), ErrorCode> {
    let now = clock.known_time(board.uptime_secs());
    record.known_time = record.known_time.max(now);
    match board.grants_store(record) {
        Ok(()) => {
            clock.stored = clock.stored.max(record.known_time);
            Ok(())
        }
        Err(_) => {
            board.log("grants record: write failed");
            Err(ErrorCode::Storage)
        }
    }
}

/// Delete a grant's counter. One that is not there is not an error.
fn delete_counter<G: GrantBoard + ?Sized>(board: &mut G, slot: u8) {
    if let Some(chip) = board.chip() {
        let _ = chip.delete(counter_id(slot));
    }
}

/// The checks a grant request passes before the holder is asked, and the grant it would make
/// (with a zero `cti`). A request that fails is `Denied`, its reason logged: an action other
/// than `git-commit`, a budget of 0 or over [`BUDGET_MAX`], no expiry without `standing` or an
/// expiry with it, a field too long to keep, an agent key that is not a P-256 point, or a
/// badge that already holds [`GRANTS_MAX`] live grants.
pub fn check_request<G: GrantBoard + ?Sized>(
    board: &mut G,
    c: &Circle,
    req: &GrantRequestFields,
) -> Result<SignGrant, ErrorCode> {
    let record = load(board)?;
    let cnf = SeatPublic::parse(&req.cnf);
    let why = if req.action != Action::GitCommit.name() {
        Some("only git commits can be granted")
    } else if req.budget == 0 || req.budget > BUDGET_MAX {
        Some("the budget is not 1 to 1000")
    } else if req.exp.is_none() && !req.standing {
        Some("no expiry, and not asked for until revoked")
    } else if req.exp.is_some() && req.standing {
        Some("until revoked, with an expiry")
    } else if req.agent.len() > AGENT_MAX
        || req.author.len() > AUTHOR_MAX
        || req
            .location
            .as_ref()
            .is_some_and(|l| l.len() > LOCATION_MAX)
    {
        Some("a field is too long")
    } else if !crate::screens::grant_request_fits(&req.agent, &req.author) {
        Some("the agent or the author would not show whole on the screen")
    } else if cnf.is_err() {
        Some("the agent's key is not a P-256 key")
    } else if record.live() >= GRANTS_MAX {
        Some("the badge holds as many grants as it can")
    } else {
        None
    };
    let refuse = |board: &mut G, why: &str| {
        board.log(&format!("grant request refused: {why}"));
        Err(ErrorCode::Denied)
    };
    if let Some(why) = why {
        return refuse(board, why);
    }
    let grant = SignGrant {
        circle_id: c.circle_id,
        seat: c.seat,
        grantee: cnf.map_err(|_| ErrorCode::Denied)?,
        cti: [0; 16],
        actions: alloc::vec![Action::GitCommit],
        author: req.author.clone(),
        locations: req.location.iter().cloned().collect(),
        agent: req.agent.clone(),
        budget: req.budget,
        exp: req.exp,
        base: 0,
    };
    // The grant must read back as itself, or nobody could check it.
    let back = Claims::decode(&grant.claims().encode()).and_then(|c| SignGrant::from_claims(&c));
    if !matches!(back, Ok(g) if g == grant) {
        return refuse(board, "the grant would not read back");
    }
    Ok(grant)
}

/// One live grant as the menu's grants page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEntry {
    pub cti: [u8; 16],
    pub agent: String,
    /// `"37 of 50 \u{b7} until 29 Nov 2026"`, or `"\u{2026} \u{b7} until revoked"`.
    pub line: String,
}

/// The live grants of `circle`, oldest first: not revoked, not expired by the badge's known
/// time. Uses come from the chip counter less the grant's base, or from the record without a
/// chip; a counter that cannot be the grant's reads as spent. An unreadable record lists none.
pub fn page_entries<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &Clock,
    circle: &CircleId,
) -> Vec<PageEntry> {
    let Ok(Some(record)) = board.grants_load() else {
        return Vec::new();
    };
    let now = clock.known_time(board.uptime_secs()).max(record.known_time);
    let mut out = Vec::new();
    for g in record
        .grants
        .iter()
        .filter(|g| g.circle_id == *circle && !g.revoked && g.exp.is_none_or(|e| now < e))
    {
        let used = match board.chip() {
            Some(chip) => chip
                .counter_get(counter_id(g.slot))
                .ok()
                .and_then(|raw| uses(raw, g.base, g.budget)),
            None => Some(g.used_soft),
        };
        let used = used.map_or_else(|| format!("{}", g.budget), |n| format!("{n}"));
        let until = match g.exp {
            Some(t) => {
                let d = date(t);
                format!("until {}", d.rsplit_once(' ').map_or(d.as_str(), |(day, _)| day))
            }
            None => String::from("until revoked"),
        };
        out.push(PageEntry {
            cti: g.cti,
            agent: g.agent.clone(),
            line: format!("{used} of {} \u{b7} {until}", g.budget),
        });
    }
    out
}

/// Revoke the grant `cti`: delete its chip counter, mark the record revoked and store it,
/// and log it. The record stays so the grant stays refused.
pub fn revoke<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &mut Clock,
    cti: &[u8; 16],
) -> Result<(), ErrorCode> {
    let mut record = load(board)?;
    let Some(g) = record.grants.iter_mut().find(|g| g.cti == *cti) else {
        return Err(ErrorCode::Denied);
    };
    g.revoked = true;
    let slot = g.slot;
    delete_counter(board, slot);
    store(board, clock, &mut record)?;
    board.log(&format!("grant {}: revoked", short(cti)));
    Ok(())
}

/// The expiry as the consent screen words it: `"until 29 Nov 2026 (laptop)"`, or
/// `"until revoked"` for a standing grant.
pub fn until_text(exp: Option<u64>) -> String {
    match exp {
        Some(t) => format!("until {} (laptop)", date(t)),
        None => String::from("until revoked"),
    }
}

/// A Unix time as a UTC date, `d MMM yyyy`.
fn date(t: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
    let z = (t / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// Sign `grant` with the circle's seat key and check the result before it is returned.
fn sign_grant<G: GrantBoard + ?Sized>(
    board: &mut G,
    c: &Circle,
    grant: &SignGrant,
) -> Result<Vec<u8>, ErrorCode> {
    let mut seat = seat_of(board.chip(), c)?;
    let public = seat.public()?;
    let signed = grant.sign(&mut seat, c.seat);
    if seat.failed() {
        return Err(ErrorCode::ChipError);
    }
    match SignGrant::verify(&signed, &public, &c.circle_id, c.seat) {
        Ok(g) if g == *grant => Ok(signed),
        _ => Err(ErrorCode::ChipError),
    }
}

/// Make the grant `req` asks for, after the holder allowed it: [`check_request`] again, the
/// time raised to the laptop's if later, the lowest free slot (the oldest revoked record makes
/// room when every slot is taken), a fresh `cti`, the next issue number as the grant's base
/// (on a chip), the grant signed by the seat key with that base in it, its counter set to the
/// base in the chip, and its record stored. Returns the signed grant and its slot.
pub fn issue<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &mut Clock,
    c: &Circle,
    req: &GrantRequestFields,
) -> Result<(Vec<u8>, u8), ErrorCode> {
    let mut grant = check_request(board, c, req)?;
    let mut record = load(board)?;
    clock.raise(req.time, board.uptime_secs());
    if record.grants.len() >= GRANTS_MAX {
        if let Some(i) = record.grants.iter().position(|g| g.revoked) {
            let old = record.grants.remove(i);
            board.log(&format!(
                "grant {}: revoked, forgotten to make room",
                short(&old.cti)
            ));
        }
    }
    let slot = record.next_free_slot().ok_or(ErrorCode::Denied)?;
    if !board.fill_random(&mut grant.cti) {
        board.log("grant request refused: the random source failed");
        return Err(ErrorCode::Denied);
    }
    // The base comes first: it is signed into the grant.
    if let Some(chip) = board.chip() {
        let issue_no = next_issue(chip)?;
        // An issue number past 32 bits would lose its high bits in the base: refuse.
        grant.base = match u32::try_from(issue_no) {
            Ok(n) if n != 0 => u64::from(n) << 32,
            _ => {
                board.log("grant request refused: the chip's issue counter is used up");
                return Err(ErrorCode::Denied);
            }
        };
    }
    let base = grant.base;
    let signed = sign_grant(board, c, &grant)?;
    if let Some(chip) = board.chip() {
        // Anything left in a free slot is stale: the record does not know it.
        let _ = chip.delete(counter_id(slot));
        chip.put_counter(counter_id(slot), base)
            .map_err(|_| ErrorCode::ChipError)?;
    }
    record.grants.push(GrantRecord {
        slot,
        cti: grant.cti,
        circle_id: c.circle_id,
        seat: c.seat,
        key_id: key_id(&grant.grantee),
        agent: grant.agent.clone(),
        action: String::from(Action::GitCommit.name()),
        author: grant.author.clone(),
        locations: grant.locations.clone(),
        budget: grant.budget,
        exp: grant.exp,
        revoked: false,
        used_soft: 0,
        base,
    });
    if let Err(e) = store(board, clock, &mut record) {
        delete_counter(board, slot);
        return Err(e);
    }
    board.log(&format!(
        "grant {}: issued to {} for {} git commits as {}",
        short(&grant.cti),
        grant.agent,
        grant.budget,
        grant.author
    ));
    Ok((signed, slot))
}

/// Decide a request to sign under a grant, in order: the grant is this seat's (else
/// `Refuse(Unparseable)`); its record is here with the grant's own base (else
/// `Refuse(Denied)`, "grant unknown") and not revoked (`Refuse(Denied)`); the proof is by the
/// grant's key over this request's hash and `nonce` (else `Refuse(Unparseable)`); the request
/// is a git commit signature with its text and an author line (else `Prompt`); then
/// [`authz::evaluate`] with the count from the chip (or the record), against the grant's own
/// base, and the badge's time. An expired or spent grant is removed, counter and
/// record, and refused; another author, location or action asks; another key or circle is
/// refused. Every reason is logged.
pub fn evaluate_request<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &mut Clock,
    record: &mut GrantsRecord,
    c: &Circle,
    req: &UnderGrantFields,
    nonce: &[u8; 16],
) -> Result<Allowed, Fallback> {
    use Fallback::{Prompt, Refuse};
    let public = seat_of(board.chip(), c)
        .and_then(|mut s| s.public())
        .map_err(Refuse)?;
    let Ok(grant) = SignGrant::verify(&req.grant, &public, &c.circle_id, c.seat) else {
        board.log("grant: not a sign grant from this seat");
        return Err(Refuse(ErrorCode::Unparseable));
    };
    let tag = short(&grant.cti);
    let say = |board: &mut G, what: &str, f: Fallback| {
        board.log(&format!("grant {tag}: {what}"));
        Err(f)
    };
    let Some(index) = record
        .grants
        .iter()
        .position(|g| g.cti == grant.cti && g.circle_id == c.circle_id)
    else {
        return say(board, "grant unknown", Refuse(ErrorCode::Denied));
    };
    let rec = record.grants[index].clone();
    // The base is the signed grant's. A record that says otherwise is not this grant's: it
    // could point the grant at another grant's counter.
    if rec.base != grant.base || (board.chip().is_some() && !base_ok(grant.base)) {
        return say(
            board,
            "grant unknown, the record's base is not the grant's",
            Refuse(ErrorCode::Denied),
        );
    }
    if rec.revoked {
        return say(board, "grant revoked", Refuse(ErrorCode::Denied));
    }
    let hash: [u8; 32] = Sha256::digest(&req.data).into();
    if !authz::proof_verify(&req.proof, &grant.grantee, &hash, nonce) {
        return say(
            board,
            "the proof is not by the grant's key over this request and nonce",
            Refuse(ErrorCode::Unparseable),
        );
    }
    let commit = match ssh::parse(&req.data) {
        Ok(s @ Signed::Sig { .. })
            if matches!(&s, Signed::Sig { namespace, .. } if namespace == "git")
                && ssh::check_content(&s, &req.content) =>
        {
            ssh::read_commit(&req.content)
        }
        _ => None,
    };
    let Some((author, subject)) = commit.and_then(|c| Some((c.author?, c.subject))) else {
        return say(
            board,
            "not a git commit with its text, asking",
            Prompt(ErrorCode::Unparseable),
        );
    };
    let used = match board.chip() {
        Some(chip) => match chip.counter_get(counter_id(rec.slot)) {
            Ok(raw) => match uses(raw, grant.base, grant.budget) {
                Some(n) => n,
                None => {
                    // The slot's counter is another grant's: this record is older than it
                    // (a restored image). The record goes; the counter stays its grant's.
                    record.grants.remove(index);
                    let _ = store(board, clock, record);
                    return say(
                        board,
                        "grant spent, its slot's counter is a later grant's",
                        Refuse(ErrorCode::Denied),
                    );
                }
            },
            Err(_) => {
                return say(board, "no counter in the chip", Refuse(ErrorCode::Denied));
            }
        },
        None => rec.used_soft,
    };
    let uptime = board.uptime_secs();
    clock.raise(req.time, uptime);
    let d = authz::evaluate(
        &grant,
        &EvalRequest {
            subject: Subject {
                key_id: key_id(&grant.grantee),
            },
            action: Action::GitCommit,
            resource: Resource {
                circle_id: c.circle_id,
                location: req.location.clone(),
            },
            context: Context {
                author,
                time: Some(clock.known_time(uptime)),
                used,
                revoked: rec.revoked,
            },
        },
    );
    match d.reason {
        Reason::Allowed => Ok(Allowed {
            grant,
            index,
            used,
            subject,
        }),
        Reason::Expired | Reason::Spent => {
            let why = if d.reason == Reason::Expired {
                "grant expired, removed"
            } else {
                "grant spent, removed"
            };
            record.grants.remove(index);
            delete_counter(board, rec.slot);
            // Refused whether or not the record is written: the counter is gone, and a
            // record without one is refused and swept.
            let _ = store(board, clock, record);
            say(board, why, Refuse(ErrorCode::Denied))
        }
        Reason::WrongAuthor => say(
            board,
            "the commit's author is not the grant's, asking",
            Prompt(ErrorCode::Denied),
        ),
        Reason::WrongLocation => say(
            board,
            "the repository is not one the grant names, asking",
            Prompt(ErrorCode::Denied),
        ),
        Reason::WrongAction => say(
            board,
            "the grant does not allow git commits, asking",
            Prompt(ErrorCode::Denied),
        ),
        Reason::WrongKey | Reason::WrongCircle => say(
            board,
            "another key or circle",
            Refuse(ErrorCode::Unparseable),
        ),
        Reason::Revoked => say(board, "grant revoked", Refuse(ErrorCode::Denied)),
    }
}

/// Sign under a grant: [`evaluate_request`] again (a request that would ask is refused here,
/// since nobody was asked), the use counted first (the chip's counter, or the record, stored
/// before the signature), the stored time written if it is behind, then the signature, checked
/// with the seat public key. Returns the SSH signature and what to show. `nonce_spent` is set
/// once the proof is accepted, before the count moves: the caller replaces the nonce even if
/// the signature then fails, so no proof is accepted twice.
pub fn sign_under_grant<G: GrantBoard + ?Sized>(
    board: &mut G,
    clock: &mut Clock,
    c: &Circle,
    req: &UnderGrantFields,
    nonce: &[u8; 16],
    nonce_spent: &mut bool,
) -> Result<(Vec<u8>, GrantUse), ErrorCode> {
    let mut record = load(board)?;
    let a = evaluate_request(board, clock, &mut record, c, req, nonce).map_err(|f| match f {
        Fallback::Prompt(_) => ErrorCode::Denied,
        Fallback::Refuse(code) => code,
    })?;
    // The proof is accepted: it is never accepted again, whatever happens below.
    *nonce_spent = true;
    // The budget and the base are the signed grant's. The record gives only the slot.
    let (slot, budget, base) = (record.grants[a.index].slot, a.grant.budget, a.grant.base);
    // Without a chip the count is in the record, which must be written before the signature.
    let mut counted_here = false;
    let used = match board.chip() {
        Some(chip) => {
            let raw = chip
                .counter_next(counter_id(slot))
                .map_err(|_| ErrorCode::ChipError)?;
            // Past the budget reads as more than it, and is refused below.
            raw.checked_sub(base).unwrap_or(u64::MAX)
        }
        None => {
            let g = &mut record.grants[a.index];
            g.used_soft += 1;
            counted_here = true;
            g.used_soft
        }
    };
    if used > u64::from(budget) {
        board.log(&format!("grant {}: spent", short(&a.grant.cti)));
        return Err(ErrorCode::Denied);
    }
    if counted_here {
        store(board, clock, &mut record)?;
    } else if clock.stale(board.uptime_secs()) {
        // Only the time: a failed write is logged and the signature goes ahead.
        let _ = store(board, clock, &mut record);
    }
    let mut seat = seat_of(board.chip(), c)?;
    let public = seat.public()?;
    let sig = seat.sign(&req.data)?;
    if !public.verify(&req.data, &sig) {
        return Err(ErrorCode::ChipError);
    }
    Ok((
        ssh::encode_signature(&sig),
        GrantUse {
            cti: a.grant.cti,
            agent: a.grant.agent,
            subject: a.subject,
            // `used <= budget`, so this fits.
            remaining: budget - used as u32,
            budget,
        },
    ))
}

/// At boot: remove what is spent, expired or half there, and return a log line for each.
/// A record of a circle not in `held` goes, revoked or not. So does one whose grant expired by
/// the stored time, or whose counter is spent or missing (a revoked one has none, and stays).
/// A counter no live grant names is deleted, and so is a revoked grant's counter left behind.
/// An unreadable record changes nothing.
///
/// The sweep has only the record, not the signed grant, so it reads the count against the
/// record's base. That can only remove a record or keep one: a use is always checked against
/// the grant's own base ([`evaluate_request`]).
pub fn sweep<G: GrantBoard + ?Sized>(board: &mut G, held: &[CircleId]) -> Vec<String> {
    let mut lines = Vec::new();
    let stored = match board.grants_load() {
        Ok(r) => r,
        Err(_) => {
            lines.push(String::from(
                "grants record: unreadable, grants and their counters left as they are",
            ));
            return lines;
        }
    };
    let mut record = stored.clone().unwrap_or_default();
    let now = record.known_time;
    let counters: Option<Vec<u8>> = match board.chip() {
        None => None,
        Some(chip) => match chip.ids() {
            Ok(ids) => Some(
                ids.iter()
                    .filter_map(|id| match KeyId::parse(id.0) {
                        Some((KeyKind::Grant, slot)) if slot != ISSUE_SLOT => Some(slot),
                        _ => None,
                    })
                    .collect(),
            ),
            Err(e) => {
                lines.push(format!("chip: could not list grant counters: {e:?}"));
                return lines;
            }
        },
    };
    let mut kept = Vec::new();
    for g in core::mem::take(&mut record.grants) {
        let used = match (&counters, board.chip()) {
            (Some(slots), Some(chip)) if slots.contains(&g.slot) => {
                // A counter that cannot be this grant's reads as spent.
                chip.counter_get(counter_id(g.slot))
                    .ok()
                    .map(|raw| match base_ok(g.base) {
                        true => uses(raw, g.base, g.budget).unwrap_or(u64::MAX),
                        false => u64::MAX,
                    })
            }
            (Some(_), _) => None,
            (None, _) => Some(g.used_soft),
        };
        let why = if !held.contains(&g.circle_id) {
            Some("its circle is gone")
        } else if g.exp.is_some_and(|e| now >= e) {
            Some("expired")
        } else if g.revoked {
            None
        } else {
            match used {
                None => Some("has no counter"),
                Some(n) if n >= u64::from(g.budget) => Some("spent"),
                Some(_) => None,
            }
        };
        match why {
            Some(why) => lines.push(format!(
                "grant {} in slot {}: {why}, removed",
                short(&g.cti),
                g.slot
            )),
            None => kept.push(g),
        }
    }
    record.grants = kept;
    for slot in counters.unwrap_or_default() {
        if record.grants.iter().any(|g| g.slot == slot && !g.revoked) {
            continue;
        }
        let Some(chip) = board.chip() else { break };
        lines.push(match chip.delete(counter_id(slot)) {
            Ok(()) => format!("grant counter in slot {slot}: no live grant, deleted"),
            Err(e) => {
                format!("grant counter in slot {slot}: no live grant, deleting failed: {e:?}")
            }
        });
    }
    if stored.is_some_and(|s| s != record) && board.grants_store(&record).is_err() {
        lines.push(String::from("grants record: write failed"));
    }
    lines
}

/// The badge's clock at boot, from the stored record's time.
pub fn boot_clock<G: GrantBoard + ?Sized>(board: &mut G) -> Clock {
    let stored = board
        .grants_load()
        .ok()
        .flatten()
        .map_or(0, |r| r.known_time);
    Clock::new(stored, board.uptime_secs())
}

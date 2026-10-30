//! The badge's answers to `Chip` requests: the seat key described, attested, and signing for
//! ssh, and the consent screen each signing request shows before it is answered.
//!
//! [`BadgeApps`] holds the circles the badge keeps and the board's chip for one request; each
//! request names its circle, and one the badge does not hold is `UnknownCircle`. [`BadgeApps::prompt`]
//! decides, before anyone is asked, whether a request is answered now, refused now, or shown:
//! a request to sign that the badge cannot describe is refused with `Unparseable` and never
//! reaches a screen. `ssh_sign` runs the same checks again after consent, signs, and verifies
//! the signature with the seat public key before it answers: a signature that does not verify
//! is `ChipError`, never returned.
//!
//! A circle whose seat key is in the chip signs there; one without a chip slot signs with its
//! software seat key and says so in `Describe`.
//!
//! - A contact card is decoded and shown (its name and how many handles and endpoints); one
//!   that names another circle or another seat is `Unparseable`.
//! - A grant, version 1 or 2, is decoded and shown; the badge signs only root grants of its
//!   own seat in the circle the request names, so anything else is `Unparseable`.
//! - A seal key is `HMAC-SHA256(root, label || circle_id)`, computed in the chip when the root
//!   is there and from the record otherwise. The badge must name the label on the screen, so a
//!   label that is not 1 to 32 printable ASCII characters is `Unparseable`.
//! - A grant request is checked by [`grants::check_request`] and shown; after Both,
//!   [`grants::issue`] makes it. A request under a grant is decided by
//!   [`grants::evaluate_request`] before anything is drawn: answered at once, refused, or
//!   asked as the ordinary signature it is (the badge then serves it as an `SshSign`). Grants
//!   need the board ([`BadgeApps::on_board`]); with the chip alone they are `NoChip`.

use crate::board::Circle;
use crate::chip::ChipSeat;
use crate::grants::{self, Clock, Fallback, GrantBoard, GrantUse};
use crate::screens::{self, Screen};
use crate::ssh::{self, HostCheck, Signed};
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_chip::{Attested, ChipOps, KeyId, KeyKind};
use cahoots_core::cose;
use cahoots_core::seat::{SeatKey, SeatPublic, SeatSigner};
use cahoots_objects::card::{self, Card};
use cahoots_objects::grant::{self, key_id, Grant, Grantor};
use cahoots_custodian::wire::{ChipRequest, ErrorCode, GrantRequestFields, SshHost, UnderGrantFields};
use cahoots_custodian::{ChipApps, ChipInfo, CircleId, CircleInfo, Custodian, PadError, PskHandle};
use zeroize::Zeroizing;

/// `Describe`'s `curve` for a P-256 seat key, the only kind there is.
pub const CURVE_P256: u8 = 0;

/// The namespace git signs commits in.
const GIT_NAMESPACE: &str = "git";

/// The longest seal key label the badge names on its screen.
pub const MAX_LABEL: usize = 32;

/// The badge's chip requests for the circles it holds.
pub struct BadgeApps<'a> {
    circles: &'a [Circle],
    hw: Hw<'a>,
    /// The nonce a proof under a grant must be made over: the link's current one, or `None`
    /// when the random source failed to draw one (then no nonce is given and no use accepted).
    nonce: Option<[u8; 16]>,
    /// A proof over `nonce` was accepted: the link must draw a new one.
    pub nonce_spent: bool,
    has_chip: bool,
    /// The signature under a grant this request made, for the badge to show; the link's nonce
    /// is spent.
    pub grant_use: Option<GrantUse>,
}

/// What the requests run on: the board's chip alone, or the board itself, which grants need
/// for their record, the time and the random source.
enum Hw<'a> {
    Chip(Option<&'a mut dyn ChipOps>),
    Board(&'a mut dyn GrantBoard, &'a mut Clock),
}

/// A seat key, in the chip or in software.
pub(crate) enum Seat<'a> {
    Chip(ChipSeat<'a>),
    Soft(SeatKey),
}

/// The circle's seat key: in the chip slot it names, or the software key it keeps.
pub(crate) fn seat_of<'c>(
    chip: Option<&'c mut dyn ChipOps>,
    circle: &Circle,
) -> Result<Seat<'c>, ErrorCode> {
    match circle.chip_slot {
        Some(slot) => {
            let chip = chip.ok_or(ErrorCode::NoChip)?;
            Ok(Seat::Chip(ChipSeat::new(
                chip,
                KeyId::new(KeyKind::Seat, slot),
            )))
        }
        None => {
            let seed = Zeroizing::new(circle.seat_key);
            SeatKey::from_seed(&seed)
                .map(Seat::Soft)
                .map_err(|_| ErrorCode::NoChip)
        }
    }
}

impl Seat<'_> {
    pub(crate) fn public(&mut self) -> Result<SeatPublic, ErrorCode> {
        match self {
            Seat::Chip(c) => {
                let p = c.public();
                c.error.map_or(Ok(p), |_| Err(ErrorCode::ChipError))
            }
            Seat::Soft(k) => Ok(k.public()),
        }
    }

    pub(crate) fn sign(&mut self, msg: &[u8]) -> Result<[u8; 64], ErrorCode> {
        match self {
            Seat::Chip(c) => {
                let sig = c.sign(msg);
                c.error.map_or(Ok(sig), |_| Err(ErrorCode::ChipError))
            }
            Seat::Soft(k) => Ok(k.sign(msg)),
        }
    }
}

/// A `Seat` as a signer for the objects' `sign` functions. A chip failure gives zeros here;
/// the caller checks `failed` and verifies the signature before it answers.
impl SeatSigner for Seat<'_> {
    fn public(&mut self) -> SeatPublic {
        match self {
            Seat::Chip(c) => c.public(),
            Seat::Soft(k) => k.public(),
        }
    }

    fn sign(&mut self, msg: &[u8]) -> [u8; 64] {
        match self {
            Seat::Chip(c) => c.sign(msg),
            Seat::Soft(k) => k.sign(msg),
        }
    }
}

impl Seat<'_> {
    pub(crate) fn failed(&self) -> bool {
        matches!(self, Seat::Chip(c) if c.error.is_some())
    }
}

/// A label the badge can name on its screen: 1 to [`MAX_LABEL`] printable ASCII characters.
fn label_ok(label: &str) -> bool {
    (1..=MAX_LABEL).contains(&label.len()) && label.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// The purpose names a grant's screen shows.
fn purpose_names(purposes: &[u8]) -> Vec<String> {
    purposes
        .iter()
        .map(|p| match card::purpose_name(*p) {
            Some(name) if *p <= 5 => String::from(name),
            _ => alloc::format!("purpose-{p}"),
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| alloc::format!("{b:02x}")).collect()
}

impl<'a> BadgeApps<'a> {
    /// The requests with the board's chip alone: a grant request or a use under a grant is
    /// `NoChip`, as there is nowhere to keep grants. `nonce` is the link's nonce.
    pub fn new(circles: &'a [Circle], chip: Option<&'a mut dyn ChipOps>, nonce: [u8; 16]) -> Self {
        let has_chip = chip.is_some();
        BadgeApps {
            circles,
            hw: Hw::Chip(chip),
            nonce: Some(nonce),
            nonce_spent: false,
            has_chip,
            grant_use: None,
        }
    }

    /// The requests on the board, grants included, with the badge's clock and the link's
    /// nonce.
    pub fn on_board(
        circles: &'a [Circle],
        board: &'a mut dyn GrantBoard,
        clock: &'a mut Clock,
        nonce: Option<[u8; 16]>,
    ) -> Self {
        let has_chip = board.chip().is_some();
        BadgeApps {
            circles,
            hw: Hw::Board(board, clock),
            nonce,
            nonce_spent: false,
            has_chip,
            grant_use: None,
        }
    }

    /// The board's chip, if it has one.
    fn chip(&mut self) -> Option<&mut dyn ChipOps> {
        match &mut self.hw {
            Hw::Chip(chip) => match chip {
                Some(c) => Some(&mut **c),
                None => None,
            },
            Hw::Board(board, _) => board.chip(),
        }
    }

    /// The board and the clock, for grants; `NoChip` without them.
    fn grant_board(&mut self) -> Result<(&mut dyn GrantBoard, &mut Clock), ErrorCode> {
        match &mut self.hw {
            Hw::Board(board, clock) => Ok((&mut **board, &mut **clock)),
            Hw::Chip(_) => Err(ErrorCode::NoChip),
        }
    }

    /// The circle the request names, if the badge holds it.
    fn held(&self, circle: &CircleId) -> Result<&'a Circle, ErrorCode> {
        let circles: &'a [Circle] = self.circles;
        circles
            .iter()
            .find(|c| c.circle_id == *circle)
            .ok_or(ErrorCode::UnknownCircle)
    }

    /// The circle's seat key: in the chip slot it names, or the software key it keeps.
    fn seat(&mut self, circle: &Circle) -> Result<Seat<'_>, ErrorCode> {
        seat_of(self.chip(), circle)
    }

    /// What to do with a `Chip` request before anyone is asked: `Ok(None)` to answer it now
    /// (it needs no consent), `Ok(Some(screen))` to show the screen and wait for the buttons,
    /// or `Err(code)` to refuse it now.
    pub fn prompt(
        &mut self,
        circle: &CircleId,
        op: &ChipRequest,
    ) -> Result<Option<Screen>, ErrorCode> {
        let c = self.held(circle)?;
        match op {
            ChipRequest::Describe | ChipRequest::Attest { .. } | ChipRequest::Nonce => Ok(None),
            ChipRequest::GrantRequest {
                action,
                author,
                location,
                agent,
                budget,
                exp,
                standing,
                cnf,
                time,
            } => {
                let req = GrantRequestFields {
                    action: action.clone(),
                    author: author.clone(),
                    location: location.clone(),
                    agent: agent.clone(),
                    budget: *budget,
                    exp: *exp,
                    standing: *standing,
                    cnf: *cnf,
                    time: *time,
                };
                let (board, _) = self.grant_board()?;
                grants::check_request(board, c, &req)?;
                Ok(Some(screens::grant_request(
                    agent,
                    author,
                    *budget,
                    &grants::until_text(*exp),
                    &c.params.name,
                )))
            }
            ChipRequest::SshSignUnderGrant {
                data,
                content,
                grant,
                proof,
                location,
                time,
            } => {
                let req = UnderGrantFields {
                    data: data.clone(),
                    content: content.clone(),
                    grant: grant.clone(),
                    proof: proof.clone(),
                    location: location.clone(),
                    time: *time,
                };
                let nonce = self.nonce;
                let (board, clock) = self.grant_board()?;
                let Some(nonce) = nonce else {
                    board.log("grant: no fresh nonce, use refused");
                    return Err(ErrorCode::Busy);
                };
                let mut record = match board.grants_load() {
                    Ok(r) => r.unwrap_or_default(),
                    Err(_) => return Err(ErrorCode::Storage),
                };
                match grants::evaluate_request(board, clock, &mut record, c, &req, &nonce) {
                    Ok(_) => Ok(None),
                    // Asked as an ordinary signature: the badge serves it as one.
                    Err(Fallback::Prompt(_)) => self
                        .ssh_screen(c, data, None, Some(content))
                        .map(Some),
                    Err(Fallback::Refuse(code)) => Err(code),
                }
            }
            ChipRequest::SshSign {
                data,
                host,
                content,
            } => self
                .ssh_screen(c, data, host.as_ref(), content.as_deref())
                .map(Some),
            ChipRequest::Card { card } => {
                let card = Self::check_card(c, card)?;
                Ok(Some(screens::card_sign(
                    &card.name,
                    card.handles.len(),
                    card.endpoints.len(),
                    &c.params.name,
                )))
            }
            ChipRequest::GrantSign { grant } => {
                let g = Self::check_grant(c, grant)?;
                let names = purpose_names(&g.purposes);
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                let short = hex(&key_id(&g.grantee)[..4]);
                Ok(Some(screens::grant(
                    &short,
                    &names,
                    g.budget,
                    g.not_after,
                    g.depth > 0,
                    &c.params.name,
                )))
            }
            ChipRequest::SealKey { label } => {
                if !label_ok(label) {
                    return Err(ErrorCode::Unparseable);
                }
                self.check_root(c)?;
                Ok(Some(screens::seal_key(label, &c.params.name)))
            }
        }
    }

    /// The card a request asks the badge to sign, if it is this seat's card for this circle.
    fn check_card(c: &Circle, bytes: &[u8]) -> Result<Card, ErrorCode> {
        let card = Card::decode(bytes).map_err(|_| ErrorCode::Unparseable)?;
        if card.circle_id != c.circle_id || card.seat != c.seat {
            return Err(ErrorCode::Unparseable);
        }
        Ok(card)
    }

    /// The grant a request asks the badge to sign, if it is a root grant from this seat in
    /// this circle: the seat key signs nothing else.
    fn check_grant(c: &Circle, bytes: &[u8]) -> Result<Grant, ErrorCode> {
        let (g, _) = Grant::decode_any(bytes).map_err(|_| ErrorCode::Unparseable)?;
        if g.circle_id != c.circle_id || g.grantor != Grantor::Seat(c.seat) || g.parent.is_some()
        {
            return Err(ErrorCode::Unparseable);
        }
        Ok(g)
    }

    /// Whether the circle's root is there to derive from: in its chip slot, or in the record.
    fn check_root(&mut self, c: &Circle) -> Result<(), ErrorCode> {
        match c.chip_slot {
            Some(slot) => {
                let chip = self.chip().ok_or(ErrorCode::NoChip)?;
                match chip.exists(KeyId::new(KeyKind::Root, slot)) {
                    Ok(true) => Ok(()),
                    _ => Err(ErrorCode::ChipError),
                }
            }
            None if c.root.is_some() => Ok(()),
            None => Err(ErrorCode::NoChip),
        }
    }

    /// Sign `payload` as a COSE_Sign1 under kid `[seat]` with the circle's seat key, and check
    /// the signature with the seat public key before it is returned.
    fn sign_object(&mut self, c: &Circle, payload: &[u8], domain: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        let mut seat = self.seat(c)?;
        let public = seat.public()?;
        let signed = cose::sign1(&mut seat, &[c.seat], payload, domain, false);
        if seat.failed() {
            return Err(ErrorCode::ChipError);
        }
        match cose::verify1(&public, &signed, None, domain) {
            Ok(p) if p == payload => Ok(signed),
            _ => Err(ErrorCode::ChipError),
        }
    }

    /// Check a request to sign for ssh and build the screen that describes it. Anything the
    /// badge cannot describe is `Unparseable`.
    fn ssh_screen(
        &mut self,
        c: &Circle,
        data: &[u8],
        host: Option<&SshHost>,
        content: Option<&[u8]>,
    ) -> Result<Screen, ErrorCode> {
        let signed = ssh::parse(data).map_err(|_| ErrorCode::Unparseable)?;
        let name = &c.params.name;
        match &signed {
            Signed::Login {
                user,
                session_id,
                key,
                host_key,
            } => {
                let public = self.seat(c)?.public()?;
                if *key != ssh::public_key_blob(&public) {
                    return Err(ErrorCode::Unparseable);
                }
                let check = match host {
                    // A host-bound request names the server's key itself: the binding must
                    // be for that key.
                    Some(h) if host_key.as_ref().is_some_and(|k| *k != h.host_key) => {
                        HostCheck::None
                    }
                    Some(h) => ssh::check_host(h, session_id),
                    None => HostCheck::None,
                };
                Ok(screens::ssh_login(user, &check, name))
            }
            Signed::Sig {
                namespace,
                hash_alg,
                hash,
            } if namespace != GIT_NAMESPACE => {
                // Any other namespace is a file (or stdin) someone signs on purpose. With
                // the content, the badge checks the hash and shows the first line; without
                // it (a file too big to send), the badge shows the hash and says the laptop
                // chose it. A signature in one namespace means nothing in another, so this
                // never signs a login or a commit.
                match content {
                    Some(bytes) => {
                        if !ssh::check_content(&signed, bytes) {
                            return Err(ErrorCode::Unparseable);
                        }
                        let preview = match core::str::from_utf8(bytes) {
                            Ok(text) => {
                                let line = text.lines().find(|l| !l.trim().is_empty());
                                match line {
                                    Some(l) => alloc::string::String::from(l.trim()),
                                    None => alloc::string::String::from("empty text"),
                                }
                            }
                            Err(_) => alloc::string::String::from("binary"),
                        };
                        Ok(screens::file_signature(
                            namespace,
                            &size_words(bytes.len()),
                            &preview,
                            true,
                            name,
                        ))
                    }
                    None => {
                        let mut digest = alloc::string::String::new();
                        for b in hash.iter().take(6) {
                            digest.push_str(&alloc::format!("{b:02x}"));
                        }
                        Ok(screens::file_signature(
                            namespace,
                            "size not sent",
                            &alloc::format!("{hash_alg} {digest}"),
                            false,
                            name,
                        ))
                    }
                }
            }
            Signed::Sig { .. } => {
                match content {
                    Some(text) => {
                        if !ssh::check_content(&signed, text) {
                            return Err(ErrorCode::Unparseable);
                        }
                        let (author, subject) =
                            ssh::describe_commit(text).ok_or(ErrorCode::Unparseable)?;
                        let subject = if subject.is_empty() {
                            "no subject"
                        } else {
                            subject.as_str()
                        };
                        Ok(screens::git_signature(&author, subject, true, name))
                    }
                    // Without the commit text the badge cannot say what it would sign.
                    None => Err(ErrorCode::Unparseable),
                }
            }
        }
    }
}

/// A size in plain words: "312 bytes", "4.2 KiB", "1.5 MiB".
fn size_words(n: usize) -> alloc::string::String {
    if n < 1024 {
        alloc::format!("{n} bytes")
    } else if n < 1024 * 1024 {
        alloc::format!("{:.1} KiB", n as f32 / 1024.0)
    } else {
        alloc::format!("{:.1} MiB", n as f32 / (1024.0 * 1024.0))
    }
}

impl ChipApps for BadgeApps<'_> {
    fn describe(&mut self, circle: &CircleId) -> Result<ChipInfo, ErrorCode> {
        let c = self.held(circle)?;
        let in_chip = c.chip_slot.is_some();
        let public = self.seat(c)?.public()?;
        Ok(ChipInfo {
            public: public.0.to_vec(),
            curve: CURVE_P256,
            in_chip,
            can_attest: in_chip,
        })
    }

    fn attest(&mut self, circle: &CircleId, freshness: &[u8; 16]) -> Result<Attested, ErrorCode> {
        let slot = self.held(circle)?.chip_slot.ok_or(ErrorCode::NoChip)?;
        let chip = self.chip().ok_or(ErrorCode::NoChip)?;
        chip.attest(KeyId::new(KeyKind::Seat, slot), freshness)
            .map_err(|_| ErrorCode::ChipError)
    }

    fn ssh_sign(
        &mut self,
        circle: &CircleId,
        data: &[u8],
        host: Option<&SshHost>,
        content: Option<&[u8]>,
    ) -> Result<Vec<u8>, ErrorCode> {
        let c = self.held(circle)?;
        // The same checks as before the screen: nothing is signed that was not described.
        self.ssh_screen(c, data, host, content)?;
        let mut seat = self.seat(c)?;
        let public = seat.public()?;
        let sig = seat.sign(data)?;
        if !public.verify(data, &sig) {
            return Err(ErrorCode::ChipError);
        }
        Ok(ssh::encode_signature(&sig))
    }

    fn card_sign(&mut self, circle: &CircleId, card: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        let c = self.held(circle)?;
        // The same checks as before the screen: the badge signs the card it described.
        Self::check_card(c, card)?;
        self.sign_object(c, card, card::DOMAIN)
    }

    fn grant_sign(&mut self, circle: &CircleId, grant: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        let c = self.held(circle)?;
        Self::check_grant(c, grant)?;
        self.sign_object(c, grant, grant::DOMAIN)
    }

    fn seal_key(&mut self, circle: &CircleId, label: &str) -> Result<[u8; 32], ErrorCode> {
        let c = self.held(circle)?;
        if !label_ok(label) {
            return Err(ErrorCode::Unparseable);
        }
        match (c.chip_slot, &c.root) {
            (Some(slot), _) => {
                let chip = self.chip().ok_or(ErrorCode::NoChip)?;
                let mut data = Vec::with_capacity(label.len() + c.circle_id.len());
                data.extend_from_slice(label.as_bytes());
                data.extend_from_slice(&c.circle_id);
                chip.hmac(KeyId::new(KeyKind::Root, slot), &data)
                    .map_err(|_| ErrorCode::ChipError)
            }
            (None, Some(root)) => Ok(cahoots_core::root::seal_key(
                root,
                label.as_bytes(),
                &c.circle_id,
            )),
            (None, None) => Err(ErrorCode::NoChip),
        }
    }

    fn nonce(&mut self, circle: &CircleId) -> Result<[u8; 16], ErrorCode> {
        self.held(circle)?;
        let nonce = self.nonce;
        let (board, _) = self.grant_board()?;
        match nonce {
            Some(n) => Ok(n),
            None => {
                board.log("grant: no fresh nonce to give");
                Err(ErrorCode::Busy)
            }
        }
    }

    fn grant_request(
        &mut self,
        circle: &CircleId,
        req: &GrantRequestFields,
    ) -> Result<(Vec<u8>, u8), ErrorCode> {
        let c = self.held(circle)?;
        let (board, clock) = self.grant_board()?;
        grants::issue(board, clock, c, req)
    }

    fn ssh_sign_under_grant(
        &mut self,
        circle: &CircleId,
        req: &UnderGrantFields,
    ) -> Result<(Vec<u8>, u32), ErrorCode> {
        let c = self.held(circle)?;
        let nonce = self.nonce.ok_or(ErrorCode::Busy)?;
        let (board, clock) = self.grant_board()?;
        let mut spent = false;
        let result = grants::sign_under_grant(board, clock, c, req, &nonce, &mut spent);
        self.nonce_spent |= spent;
        let (signature, used) = result?;
        let remaining = used.remaining;
        self.grant_use = Some(used);
        Ok((signature, remaining))
    }

    fn has_chip(&self) -> bool {
        self.has_chip
    }
}

/// What `serve` is handed on the badge: the pad's custodian and the chip requests together.
pub struct Served<'a, C> {
    pub custody: &'a mut C,
    pub apps: BadgeApps<'a>,
    /// The card this request signed, and its circle, for the badge to keep.
    pub signed_card: Option<(CircleId, Vec<u8>)>,
}

impl<C: Custodian> Custodian for Served<'_, C> {
    fn circles(&self) -> Vec<CircleInfo> {
        self.custody.circles()
    }

    fn psk(
        &mut self,
        circle: &CircleId,
        context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError> {
        self.custody.psk(circle, context)
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        self.custody.psk_for(handle, context)
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        self.custody.spend(circle, len)
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        self.custody.spend_for(handle, len)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        self.custody.remaining(circle)
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        self.custody.low_pad(circle)
    }
}

impl<C> ChipApps for Served<'_, C> {
    fn describe(&mut self, circle: &CircleId) -> Result<ChipInfo, ErrorCode> {
        self.apps.describe(circle)
    }

    fn attest(&mut self, circle: &CircleId, freshness: &[u8; 16]) -> Result<Attested, ErrorCode> {
        self.apps.attest(circle, freshness)
    }

    fn ssh_sign(
        &mut self,
        circle: &CircleId,
        data: &[u8],
        host: Option<&SshHost>,
        content: Option<&[u8]>,
    ) -> Result<Vec<u8>, ErrorCode> {
        self.apps.ssh_sign(circle, data, host, content)
    }

    fn card_sign(&mut self, circle: &CircleId, card: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        let signed = self.apps.card_sign(circle, card)?;
        self.signed_card = Some((*circle, signed.clone()));
        Ok(signed)
    }

    fn grant_sign(&mut self, circle: &CircleId, grant: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        self.apps.grant_sign(circle, grant)
    }

    fn seal_key(&mut self, circle: &CircleId, label: &str) -> Result<[u8; 32], ErrorCode> {
        self.apps.seal_key(circle, label)
    }

    fn nonce(&mut self, circle: &CircleId) -> Result<[u8; 16], ErrorCode> {
        self.apps.nonce(circle)
    }

    fn grant_request(
        &mut self,
        circle: &CircleId,
        req: &GrantRequestFields,
    ) -> Result<(Vec<u8>, u8), ErrorCode> {
        self.apps.grant_request(circle, req)
    }

    fn ssh_sign_under_grant(
        &mut self,
        circle: &CircleId,
        req: &UnderGrantFields,
    ) -> Result<(Vec<u8>, u32), ErrorCode> {
        self.apps.ssh_sign_under_grant(circle, req)
    }

    fn has_chip(&self) -> bool {
        self.apps.has_chip()
    }
}

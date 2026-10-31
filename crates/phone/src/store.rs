//! The phone's records: CBOR files in the app's container.
//!
//! ```text
//! phone.cbor                     this phone's own key, the name and the face seed
//! circles/<id>/circle.cbor       params, seat, words, manifest, root, pair keys, seat key label
//! circles/<id>/pad.bin           the pad, raw
//! circles/<id>/ledger.bin        spent ranges per slice (none yet: the app sends no pure-pad messages)
//! circles/<id>/chat.cbor         chat session state and the outbox
//! circles/<id>/cards.cbor        the signed cards read from the chat, and this seat's own
//! circles/<id>/messages.log      one CBOR message record after another, plaintext
//! circles/<id>/cursor.cbor       the last relay sequence number read
//! circles/<id>/settings.cbor     the relay address, the unread count, the circle's own face
//! channels/<id>/owner.cbor       a channel this phone owns: the owner core's export (secrets)
//! channels/<id>/member.cbor      a channel this phone reads: the member core's export (secrets)
//! channels/<id>/meta.cbor        the channel's app record: relay, name, policy, cursor, flags
//! channels/<id>/messages.cbor    the channel's messages, plaintext
//! ```
//!
//! Swift opens the container with complete file protection. `cursor.cbor` and `settings.cbor`
//! hold nothing secret, so Swift may give them protection until first unlock, and background
//! refresh can read the relay while the phone is locked.
//!
//! A circle is written into `circles/.<id>.new` and renamed into place, so a circle directory
//! is complete or absent. Every file is replaced by write, sync and rename.

use crate::PhoneError;
use minicbor::bytes::ByteVec;
use minicbor::{Decode, Encode};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

pub const PHONE: &str = "phone.cbor";
pub const CIRCLES: &str = "circles";
pub const CIRCLE: &str = "circle.cbor";
pub const PAD: &str = "pad.bin";
pub const LEDGER: &str = "ledger.bin";
pub const CHAT: &str = "chat.cbor";
pub const CARDS: &str = "cards.cbor";
pub const MESSAGES: &str = "messages.log";
pub const CURSOR: &str = "cursor.cbor";
pub const SETTINGS: &str = "settings.cbor";
pub const CHANNELS: &str = "channels";
pub const OWNER: &str = "owner.cbor";
pub const MEMBER: &str = "member.cbor";
pub const META: &str = "meta.cbor";
pub const CHANNEL_MESSAGES: &str = "messages.cbor";

/// Load `path`, or `None` when the file does not exist. Any other failure is an error, so a
/// file that cannot be read now (iOS protection while locked) is never replaced by defaults.
pub fn load_if_present<T: for<'b> Decode<'b, ()>>(path: &Path) -> Result<Option<T>, PhoneError> {
    if !path.exists() {
        return Ok(None);
    }
    load(path).map(Some)
}

pub(crate) fn io(e: std::io::Error) -> PhoneError {
    PhoneError::Storage {
        message: e.to_string(),
    }
}

fn format(what: &str) -> PhoneError {
    PhoneError::Storage {
        message: format!("The file {what} could not be read."),
    }
}

/// Write `data` to `path` so that a crash leaves the old file or the new one.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), PhoneError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp).map_err(io)?;
        f.write_all(data).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)?;
    if let Some(parent) = path.parent() {
        File::open(parent).and_then(|d| d.sync_all()).map_err(io)?;
    }
    Ok(())
}

fn save<T: Encode<()>>(path: &Path, value: &T) -> Result<(), PhoneError> {
    let mut bytes = minicbor::to_vec(value).map_err(|_| format(&path.display().to_string()))?;
    let r = write_atomic(path, &bytes);
    bytes.zeroize();
    r
}

fn load<T: for<'b> Decode<'b, ()>>(path: &Path) -> Result<T, PhoneError> {
    let mut bytes = fs::read(path).map_err(io)?;
    let r = minicbor::decode(&bytes).map_err(|_| format(&path.display().to_string()));
    bytes.zeroize();
    r
}

/// This phone's own key, made on first open.
#[derive(Encode, Decode, Clone, Debug)]
pub struct PhoneRecord {
    #[n(0)]
    pub label: String,
    #[n(1)]
    pub public: ByteVec,
    #[n(2)]
    pub fingerprint: String,
    /// The relay new ceremonies and circles use, when set. `None` is the default relay.
    #[n(3)]
    pub relay: Option<String>,
    /// The name on this phone's cards. `None` until the person gives one.
    #[n(4)]
    pub name: Option<String>,
    /// The seed of this phone's face: 32 random bytes drawn on first open, or drawn again on
    /// request. `None` only in a record written before faces, and `Phone::open` fills it.
    #[n(5)]
    pub face_seed: Option<ByteVec>,
}

impl PhoneRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(PHONE))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(PHONE), self)
    }
}

/// A circle this phone is a seat in. Secrets: the root and the pair keys.
#[derive(Encode, Decode, Clone)]
pub struct CircleRecord {
    /// `CircleParams::encode`.
    #[n(0)]
    pub params: ByteVec,
    #[n(1)]
    pub circle_id: ByteVec,
    #[n(2)]
    pub seat: u8,
    #[n(3)]
    pub tier: u8,
    /// "CSPRNG": the phone's contribution comes from the system's generator.
    #[n(4)]
    pub label: String,
    #[n(5)]
    pub words: Vec<String>,
    /// The encoded `SealedManifest`.
    #[n(6)]
    pub manifest: ByteVec,
    /// The circle root, 32 bytes.
    #[n(7)]
    pub root: ByteVec,
    /// The `Signer` label of this seat's key.
    #[n(8)]
    pub seat_label: String,
    #[n(9)]
    pub seat_public: ByteVec,
    /// One per seat, empty for this seat's own and for a seat without one.
    #[n(10)]
    pub pair_keys: Vec<ByteVec>,
    /// Where the hello words with each seat stand.
    #[n(11)]
    pub pair_counters: Vec<u32>,
}

impl Drop for CircleRecord {
    fn drop(&mut self) {
        self.root.zeroize();
        for k in &mut self.pair_keys {
            k.zeroize();
        }
    }
}

impl CircleRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(CIRCLE))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(CIRCLE), self)
    }
}

/// One message waiting for the relay's answer. `wire` is posted again, byte for byte, until
/// the relay answers with a sequence number or the same bytes are read back from it.
#[derive(Encode, Decode, Clone, Debug)]
pub struct OutboxEntry {
    #[n(0)]
    pub id: String,
    #[n(1)]
    pub wire: ByteVec,
}

/// The chat session's exported state and the outbox.
#[derive(Encode, Decode, Clone)]
pub struct ChatRecord {
    #[n(0)]
    pub state: ByteVec,
    #[n(1)]
    pub outbox: Vec<OutboxEntry>,
    /// History lines saved with the state that made them, and copied to `messages.log` after.
    /// A crash between the two leaves them here, and the next load copies the ones the log
    /// lacks: no line is lost and none is written twice.
    #[n(2)]
    pub journal: Vec<MessageRecord>,
    /// The wire of a bump this phone sent that the relay has not accepted. It is posted again,
    /// byte for byte, before the outbox, until the relay answers or the poll reads it back.
    #[n(3)]
    pub pending_bump: Option<ByteVec>,
    /// This seat's signed card, waiting to be sealed into the chat once the relay has been
    /// read to the end. Once sealed it moves to the outbox.
    #[n(4)]
    pub pending_card: Option<ByteVec>,
    /// This seat's card should be signed and published again: a signature was refused, or
    /// the card was never made. Each poll tries again.
    #[n(5)]
    #[cbor(default)]
    pub card_due: bool,
}

impl Drop for ChatRecord {
    fn drop(&mut self) {
        self.state.zeroize();
    }
}

impl ChatRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(CHAT))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(CHAT), self)
    }
}

#[derive(Encode, Decode, Clone, Copy, Debug, Default)]
pub struct CursorRecord {
    /// The last relay sequence number read; `None` before the first.
    #[n(0)]
    pub after: Option<u64>,
}

impl CursorRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(CURSOR))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(CURSOR), self)
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct SettingsRecord {
    #[n(0)]
    pub relay: String,
    /// Messages from others read from the relay since the history was last shown.
    #[n(1)]
    pub unread: u32,
    /// A face seed for this circle only, used on this seat's card in place of the phone's.
    #[n(2)]
    pub face_override: Option<ByteVec>,
}

impl SettingsRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(SETTINGS))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(SETTINGS), self)
    }
}

/// One seat's card: the signed bytes as read, and the relay sequence number they were read
/// at. `seq` is `None` for this seat's own card, kept when it was made.
#[derive(Encode, Decode, Clone, Debug)]
pub struct CardEntry {
    #[n(0)]
    pub seat: u8,
    #[n(1)]
    pub card: ByteVec,
    #[n(2)]
    pub seq: Option<u64>,
}

/// The cards of a circle, one per seat at most.
#[derive(Encode, Decode, Clone, Debug, Default)]
pub struct CardsRecord {
    #[n(0)]
    pub cards: Vec<CardEntry>,
}

impl CardsRecord {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        Ok(load_if_present(&dir.join(CARDS))?.unwrap_or_default())
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(CARDS), self)
    }

    pub fn get(&self, seat: u8) -> Option<&CardEntry> {
        self.cards.iter().find(|c| c.seat == seat)
    }

    /// Keep `card` for `seat` unless the card held was read at a later relay position.
    /// Returns whether it was kept.
    pub fn put(&mut self, seat: u8, card: Vec<u8>, seq: Option<u64>) -> bool {
        let entry = CardEntry {
            seat,
            card: card.into(),
            seq,
        };
        match self.cards.iter_mut().find(|c| c.seat == seat) {
            Some(held) => {
                if let (Some(old), Some(new)) = (held.seq, seq) {
                    if new < old {
                        return false;
                    }
                }
                *held = entry;
            }
            None => {
                self.cards.push(entry);
                self.cards.sort_by_key(|c| c.seat);
            }
        }
        true
    }
}

/// Spent pad ranges per slice, `(start, end)`.
#[derive(Encode, Decode, Clone, Debug, Default)]
pub struct LedgerRecord {
    #[n(0)]
    pub spent: Vec<Vec<(u32, u32)>>,
}

/// One chat message in `messages.log`.
#[derive(Encode, Decode, Clone, Debug)]
pub struct MessageRecord {
    #[n(0)]
    pub id: String,
    #[n(1)]
    pub sender: u8,
    #[n(2)]
    pub mine: bool,
    #[n(3)]
    pub text: String,
    #[n(4)]
    pub epoch: u32,
    #[n(5)]
    pub seq: Option<u64>,
    /// `None` or "text" for a message, "bump" for a note that the keys were reset.
    #[n(6)]
    pub kind: Option<String>,
}

/// Append one record to `messages.log`.
pub fn append_message(dir: &Path, m: &MessageRecord) -> Result<(), PhoneError> {
    let bytes = minicbor::to_vec(m).map_err(|_| format(MESSAGES))?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(MESSAGES))
        .map_err(io)?;
    f.write_all(&bytes).map_err(io)?;
    f.sync_all().map_err(io)
}

/// Every record in `messages.log`, oldest first. A torn last record is dropped.
pub fn read_messages(dir: &Path) -> Result<Vec<MessageRecord>, PhoneError> {
    let bytes = match fs::read(dir.join(MESSAGES)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(e)),
    };
    let mut d = minicbor::Decoder::new(&bytes);
    let mut out = Vec::new();
    while d.position() < bytes.len() {
        match d.decode::<MessageRecord>() {
            Ok(m) => out.push(m),
            Err(_) => break,
        }
    }
    Ok(out)
}

/// The files of a new circle, written into a hidden directory and renamed into place.
pub struct NewCircle<'a> {
    pub record: &'a CircleRecord,
    pub pad: &'a [u8],
    pub chat: &'a ChatRecord,
    pub settings: &'a SettingsRecord,
    pub slices: u8,
}

pub fn circles_dir(root: &Path) -> PathBuf {
    root.join(CIRCLES)
}

pub fn create_circle(root: &Path, id_hex: &str, c: &NewCircle) -> Result<PathBuf, PhoneError> {
    let base = circles_dir(root);
    fs::create_dir_all(&base).map_err(io)?;
    let dir = base.join(id_hex);
    if dir.exists() {
        return Err(PhoneError::Ceremony {
            message: "This phone already holds this circle.".into(),
        });
    }
    let tmp = base.join(format!(".{id_hex}.new"));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(io)?;
    }
    fs::create_dir_all(&tmp).map_err(io)?;
    write_atomic(&tmp.join(PAD), c.pad)?;
    save(
        &tmp.join(LEDGER),
        &LedgerRecord {
            spent: vec![Vec::new(); usize::from(c.slices)],
        },
    )?;
    c.record.save(&tmp)?;
    c.chat.save(&tmp)?;
    c.settings.save(&tmp)?;
    CursorRecord::default().save(&tmp)?;
    fs::rename(&tmp, &dir).map_err(io)?;
    File::open(&base).and_then(|d| d.sync_all()).map_err(io)?;
    Ok(dir)
}

/// The ids of the circles held, in name order of their directories.
pub fn circle_ids(root: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(circles_dir(root)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    ids.sort();
    ids
}

pub fn channels_dir(root: &Path) -> PathBuf {
    root.join(CHANNELS)
}

/// The ids of the channels held, in name order of their directories.
pub fn channel_ids(root: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(channels_dir(root)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().join(META).is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    ids.sort();
    ids
}

/// A channel's app record, beside the core's exported state: what `web/app/channels.js` keeps
/// in its channel record, and the poll loop's flags, which the phone keeps across calls.
#[derive(Encode, Decode, Clone, Debug, Default)]
pub struct ChannelMeta {
    /// This phone owns the channel (`owner.cbor`), or reads it (`member.cbor`).
    #[n(0)]
    pub owner: bool,
    /// The owner circle's id, hex.
    #[n(1)]
    pub circle: String,
    /// The relay's address: the circle's relay for an owner, `https://` and the link's host
    /// for a member.
    #[n(2)]
    pub relay: String,
    #[n(3)]
    pub name: String,
    /// The preset the owner picked; `None` for a member.
    #[n(4)]
    pub preset: Option<u8>,
    /// The approvals preset 1 asks for; 0 otherwise.
    #[n(5)]
    pub quorum: u8,
    /// The current signed policy.
    #[n(6)]
    #[cbor(with = "minicbor::bytes")]
    pub policy: Vec<u8>,
    #[n(7)]
    pub policy_line: String,
    /// This member's grant chain once the owner let it post; the self grant for an owner.
    #[n(8)]
    pub grant_chain: Option<ByteVec>,
    /// The owner circle's encoded `SealedManifest`.
    #[n(9)]
    #[cbor(with = "minicbor::bytes")]
    pub manifest: Vec<u8>,
    /// The last relay sequence number read.
    #[n(10)]
    pub seq: Option<u64>,
    /// This phone's member key id, hex.
    #[n(11)]
    pub kid: String,
    /// Join requests the owner allowed or refused.
    #[n(12)]
    pub handled: Vec<String>,
    /// What each admitted grant (hex id) was given: `"read,post"`.
    #[n(13)]
    pub actions: Vec<(String, String)>,
    /// SHA-256 of the gate token registered with the relay, hex.
    #[n(14)]
    pub gate_hash: Option<String>,
    /// The name this member asked to post under.
    #[n(15)]
    pub my_name: String,
    #[n(16)]
    pub wants_post: bool,
    /// A join request was posted; each poll looks for the grant.
    #[n(17)]
    pub requested: bool,
    /// The link no longer reads: retired, or this member was removed.
    #[n(18)]
    pub dead: bool,
    /// Key ids that sent control messages: the owner's.
    #[n(19)]
    pub owner_kids: Vec<String>,
    #[n(20)]
    pub unread: u32,
    /// Read with a proof by the member key instead of the link's gate token.
    #[n(21)]
    pub use_proof: bool,
    /// A rotation came with no deposit this member can open.
    #[n(22)]
    pub behind: bool,
    /// The highest epoch number of any entry read.
    #[n(23)]
    pub max_epoch: u32,
    /// An epoch whose deposit could not be fetched, tried again after the next sync.
    #[n(24)]
    pub retry_epoch: Option<ByteVec>,
    /// The owner's `Signer` label for its seat key.
    #[n(25)]
    pub seat_label: String,
    /// The owner's seat key, SEC1 compressed.
    #[n(26)]
    #[cbor(with = "minicbor::bytes")]
    pub seat_public: Vec<u8>,
}

impl ChannelMeta {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        load(&dir.join(META))
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(META), self)
    }
}

/// One line of a channel's history. `kind`: 0 text, 1 announcement, 2 a note from the app
/// (a member removed, a policy change), 3 a message refused on reading.
#[derive(Encode, Decode, Clone, Debug)]
pub struct ChannelMessageRecord {
    #[n(0)]
    pub id: String,
    /// The sender's member key id, hex.
    #[n(1)]
    pub kid: String,
    /// The name shown: an admitted member's name, "owner", or a short key id.
    #[n(2)]
    pub sender: String,
    #[n(3)]
    pub mine: bool,
    #[n(4)]
    pub text: String,
    #[n(5)]
    pub kind: u8,
    /// For an announcement: its approvals reached the quorum.
    #[n(6)]
    pub ok: bool,
    #[n(7)]
    pub seq: Option<u64>,
    /// Milliseconds since 1970: when the relay took it, or when this phone sent it.
    #[n(8)]
    pub time: u64,
}

/// A channel's history, oldest first.
#[derive(Encode, Decode, Clone, Debug, Default)]
pub struct ChannelMessages {
    #[n(0)]
    pub messages: Vec<ChannelMessageRecord>,
}

impl ChannelMessages {
    pub fn load(dir: &Path) -> Result<Self, PhoneError> {
        Ok(load_if_present(&dir.join(CHANNEL_MESSAGES))?.unwrap_or_default())
    }
    pub fn save(&self, dir: &Path) -> Result<(), PhoneError> {
        save(&dir.join(CHANNEL_MESSAGES), self)
    }
}

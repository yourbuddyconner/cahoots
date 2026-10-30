//! `cahoots-phone`: the iOS app's state machine over the Cahoots crates, behind UniFFI.
//!
//! One object, `Phone`, holds the circles, the ceremonies in progress and the chats. Swift
//! renders what it returns and supplies three things through callbacks: keys in the Secure
//! Enclave (`Signer`), HTTP to the relay (`Http`), and the directory the records live in,
//! which Swift opens with iOS file protection. Nothing cryptographic happens in Swift.
//!
//! A circle is a seat on this phone: the seat key is made by the `Signer` and never leaves the
//! enclave, and the pad, the circle root and the chat keys are files (tier 0). See
//! `docs/superpowers/specs/2026-10-30-ios-app-design.md`.

uniffi::setup_scaffolding!();

pub mod ceremony;
pub mod chat;
pub mod signer;
pub mod store;

use crate::ceremony::{Pending, Receiver};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use crate::chat::{Circle, DEFAULT_RELAY};
use crate::store::{ChatRecord, CircleRecord, NewCircle, PhoneRecord, SettingsRecord};
use cahoots_core::ceremony::Phase;
use cahoots_core::channel::{Channel, OFFER_PREFIX};
use cahoots_core::pairwise;
use cahoots_core::params::CircleParams;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

pub use crate::signer::Signer;

/// What the phone says about where its keys are.
pub const KEY_STORE: &str = "seat key in the Secure Enclave, secret protected by iOS";
/// The longest circle name, in bytes, so the name fits in the step-one code.
pub const MAX_NAME: usize = 200;
/// The `Signer` label of this phone's own key.
pub const PHONE_KEY: &str = "cahoots:phone";

/// Every error the app can be shown, in plain words. `Locked`, `Network` and `Signer` are
/// also what Swift's callbacks return.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PhoneError {
    /// The key needs Face ID or the passcode, and did not get it.
    #[error("This needs Face ID or your passcode.")]
    Locked,
    #[error("{message}")]
    Network { message: String },
    #[error("{message}")]
    Storage { message: String },
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    BadCode { message: String },
    #[error("{message}")]
    Ceremony { message: String },
    #[error("{message}")]
    Chat { message: String },
    #[error("{message}")]
    Signer { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for PhoneError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        PhoneError::Signer { message: e.reason }
    }
}

/// HTTP for the relay. `get` and `post` return the response body for a 2xx answer and an
/// error (`PhoneError::Network`) for anything else.
#[uniffi::export(callback_interface)]
pub trait Http: Send + Sync {
    fn get(&self, url: String) -> Result<Vec<u8>, PhoneError>;
    /// `body` is sent as `application/octet-stream`.
    fn post(&self, url: String, body: Vec<u8>) -> Result<Vec<u8>, PhoneError>;
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CircleSummary {
    /// The circle id, 32 hex characters.
    pub id: String,
    pub name: String,
    pub members: u8,
    /// This phone's seat, from 0.
    pub my_seat: u8,
    /// Whether the circle holds the root chat needs. Always true for a circle made here.
    pub has_root: bool,
    /// Messages from others read from the relay since the history was last shown.
    pub unread: u32,
    pub relay: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CeremonyPhase {
    /// Step one: setup codes.
    Commit,
    /// Step two: reveals.
    Reveal,
    /// Step three: signatures.
    Attest,
    /// Every signature is in. Compare the words, then keep or discard.
    Complete,
    /// A reveal did not match. Nothing is kept.
    Aborted,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CeremonyState {
    pub phase: CeremonyPhase,
    pub my_seat: u8,
    pub members: u8,
    pub name: String,
    /// Seats this step still needs a code from, from 0.
    pub missing: Vec<u8>,
    /// Frames read of the animation being scanned, and the fewest it needs.
    pub frames_read: u32,
    pub frames_needed: u32,
    /// The six check words once complete, else empty.
    pub words: Vec<String>,
    /// Step three waits for this seat's signature: call `ceremony_sign`.
    pub needs_signature: bool,
    /// Held through the relay: call `ceremony_tick` every 2 seconds. Otherwise codes only.
    pub through_relay: bool,
    /// Another device took this seat first on the relay. This phone posts nothing more:
    /// show that, and offer to leave. Joining again with a free seat works.
    pub seat_lost: bool,
    /// On the host: the seats whose code was scanned here and whose commitment was taken.
    pub scanned: Vec<u8>,
    /// On the host: seats that committed on the relay but were not scanned here.
    pub unscanned: Vec<u8>,
    /// On the host: everyone is scanned. Show "Everyone is scanned" and a Continue button
    /// that calls `ceremony_continue`.
    pub needs_continue: bool,
    /// On a joiner: the code to show the host ("Show this to the host"), drawn with
    /// `qr_matrix`. `None` once the host has taken it.
    pub my_join_code: Option<String>,
}

/// A circle to join, read from a host's code.
#[derive(Debug, Clone, uniffi::Record)]
pub struct JoinOffer {
    /// Hand this to `join_offer` (through the relay) or `join_ceremony` (codes only) with
    /// the seat picked.
    pub params_code: String,
    pub members: u8,
    pub name: String,
    /// The host's code for a ceremony held through the relay.
    pub through_relay: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct QrMatrix {
    /// Modules a side, without a quiet zone.
    pub size: u32,
    /// `size * size` modules, row by row, true for dark.
    pub bits: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatMessage {
    pub id: String,
    /// The sender's seat, from 0.
    pub sender: u8,
    pub mine: bool,
    pub text: String,
    /// The root epoch it was sent in.
    pub epoch: u32,
    /// Ours, and the relay has not taken it yet.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HelloWords {
    /// Two words you say.
    pub mine: String,
    /// Two words they say back.
    pub theirs: String,
    /// The hello number. Both phones move it with `next_hello`.
    pub number: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PhoneInfo {
    pub key_store: String,
    pub app_version: String,
    /// This phone's own key, short: four groups of four hex characters.
    pub fingerprint: String,
}

struct State {
    ceremonies: HashMap<String, Pending>,
    /// Reads a host's step-one animation before there is a ceremony to feed.
    join_receiver: Receiver,
    circles: HashMap<String, Circle>,
    /// The relay new ceremonies and circles use.
    relay: String,
}

/// The app's state. Every method takes one lock and holds it across the `Http` and `Signer`
/// callbacks it makes (a request, or a Face ID prompt). So Swift must call `Phone` off the
/// main thread, and the callbacks must never call back into `Phone` or wait for the main
/// thread while `Phone` waits for them: either deadlocks.
#[derive(uniffi::Object)]
pub struct Phone {
    dir: PathBuf,
    signer: Box<dyn Signer>,
    http: Box<dyn Http>,
    me: PhoneRecord,
    state: Mutex<State>,
}

fn not_found(what: &str) -> PhoneError {
    PhoneError::NotFound {
        message: format!("No {what} with that id on this phone."),
    }
}

/// A ceremony lives in memory only, so one the phone does not hold was lost when the app
/// closed. Joining it again cannot work: the old setup is already on the relay.
fn ceremony_lost() -> PhoneError {
    PhoneError::NotFound {
        message: "This ceremony was lost when the app closed. Everyone starts over with a new code.".into(),
    }
}

fn fingerprint(public: &[u8]) -> String {
    let h = hex::encode(&Sha256::digest(public)[..8]);
    [&h[0..4], &h[4..8], &h[8..12], &h[12..16]].join(" ")
}

fn phase(p: Phase) -> CeremonyPhase {
    match p {
        Phase::Commit => CeremonyPhase::Commit,
        Phase::Reveal => CeremonyPhase::Reveal,
        Phase::Attest => CeremonyPhase::Attest,
        Phase::Complete => CeremonyPhase::Complete,
        Phase::Aborted => CeremonyPhase::Aborted,
    }
}

fn params_err(_: cahoots_core::params::ParamsError) -> PhoneError {
    PhoneError::Ceremony {
        message: "A circle has 2 to 8 people, a name of at most 255 bytes, and a secret large enough for everyone.".into(),
    }
}

impl Phone {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn circle_dir(&self, id: &str) -> PathBuf {
        store::circles_dir(&self.dir).join(id)
    }

    /// The circle `id`, loaded on first use and kept, so a session stays synced for the life
    /// of this object.
    fn with_circle<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut Circle, &Phone) -> Result<T, PhoneError>,
    ) -> Result<T, PhoneError> {
        let mut st = self.lock();
        if !st.circles.contains_key(id) {
            if !store::circle_ids(&self.dir).iter().any(|c| c == id) {
                return Err(not_found("circle"));
            }
            let c = Circle::load(&self.circle_dir(id))?;
            st.circles.insert(id.to_string(), c);
        }
        let c = st.circles.get_mut(id).expect("inserted");
        f(c, self)
    }

    fn with_ceremony<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut Pending) -> Result<T, PhoneError>,
    ) -> Result<T, PhoneError> {
        let mut st = self.lock();
        let p = st.ceremonies.get_mut(id).ok_or_else(ceremony_lost)?;
        f(p)
    }

    /// A new ceremony id, and a seat key made for it under `cahoots:seat:<ceremony id>`. The
    /// circle id is not known until every reveal is in, so the key is named for the ceremony
    /// and the circle record keeps the label.
    fn begin(
        &self,
        params: CircleParams,
        seat: u8,
        channel: Option<Channel>,
        relay: Option<String>,
    ) -> Result<(String, Pending), PhoneError> {
        let id = chat::random_id()?;
        let label = format!("cahoots:seat:{id}");
        let relay = match relay {
            Some(r) => r,
            None => self.lock().relay.clone(),
        };
        let p = Pending::new(params, seat, self.signer.as_ref(), label, channel, relay)?;
        Ok((id, p))
    }

    fn state_of(p: &Pending) -> CeremonyState {
        let (read, needed) = p.progress();
        CeremonyState {
            phase: phase(p.phase()),
            my_seat: p.seat,
            members: p.params.n,
            name: p.params.name.clone(),
            missing: p.missing(),
            frames_read: read,
            frames_needed: needed,
            words: p.words(),
            needs_signature: p.needs_signature(),
            through_relay: p.through_relay(),
            seat_lost: p.seat_lost(),
            scanned: p.scanned(),
            unscanned: p.unscanned(),
            needs_continue: p.needs_continue(),
            my_join_code: p.my_join_code(),
        }
    }
}

#[uniffi::export]
impl Phone {
    /// Open the records in `dir`, making this phone's own key on first open.
    #[uniffi::constructor]
    pub fn open(
        dir: String,
        signer: Box<dyn Signer>,
        http: Box<dyn Http>,
    ) -> Result<Arc<Phone>, PhoneError> {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(store::circles_dir(&dir)).map_err(store::io)?;
        // Only a missing file is a first open. A file that cannot be read (iOS protection while
        // locked) is an error, and nothing is made or written over it.
        let me = match PhoneRecord::load(&dir) {
            Ok(r) => r,
            Err(_) if !dir.join(store::PHONE).exists() => {
                let public = signer.make_key(PHONE_KEY.into())?;
                let r = PhoneRecord {
                    label: PHONE_KEY.into(),
                    fingerprint: fingerprint(&public),
                    public: public.into(),
                    relay: None,
                };
                r.save(&dir)?;
                r
            }
            Err(e) => return Err(e),
        };
        let relay = me.relay.clone().unwrap_or_else(|| DEFAULT_RELAY.into());
        Ok(Arc::new(Phone {
            dir,
            signer,
            http,
            me,
            state: Mutex::new(State {
                ceremonies: HashMap::new(),
                join_receiver: Receiver::new(ceremony::JOIN_MAX_LEN),
                circles: HashMap::new(),
                relay,
            }),
        }))
    }

    pub fn circles(&self) -> Vec<CircleSummary> {
        store::circle_ids(&self.dir)
            .into_iter()
            .filter_map(|id| {
                self.with_circle(&id, |c, _| {
                    Ok(CircleSummary {
                        id: c.id_hex.clone(),
                        name: c.params.name.clone(),
                        members: c.params.n,
                        my_seat: c.record.seat,
                        has_root: c.record.root.len() == 32,
                        unread: c.settings.unread,
                        relay: c.settings.relay.clone(),
                    })
                })
                .ok()
            })
            .collect()
    }

    /// Start a ceremony as seat 1 (index 0). Returns the ceremony id. Through the relay,
    /// everyone joining scans `offer_code` and the rounds go through the relay with
    /// `ceremony_tick`. Otherwise the rounds are codes shown and scanned (`ceremony_frames`).
    pub fn start_ceremony(
        &self,
        name: String,
        members: u8,
        pad_len: u32,
        through_relay: bool,
    ) -> Result<String, PhoneError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PhoneError::Ceremony {
                message: "Give the circle a name.".into(),
            });
        }
        if name.len() > MAX_NAME {
            return Err(PhoneError::Ceremony {
                message: format!("A circle's name can be at most {MAX_NAME} bytes long. Try a shorter one."),
            });
        }
        let params = CircleParams::new(members, pad_len, name, false).map_err(params_err)?;
        let channel = through_relay.then(|| Channel::new(&mut rand_core::OsRng));
        let (id, p) = self.begin(params, 0, channel, None)?;
        self.lock().ceremonies.insert(id.clone(), p);
        Ok(id)
    }

    /// The code to show everyone joining a ceremony held through the relay: one static QR code,
    /// drawn with `qr_matrix`. The app shows it as a code only, with no way to copy it: a
    /// copied code can reach the clipboard of the user's other devices.
    pub fn offer_code(&self, id: String) -> Result<String, PhoneError> {
        self.with_ceremony(&id, |p| p.offer())
    }

    /// Join a ceremony held through the relay as `seat` (from 0), from the host's code
    /// (`cahoots:ceremony:v1:...`). Seat 0 is the host's. Returns the ceremony id. The
    /// ceremony, and the circle it makes, use the host's relay from the code, whatever this
    /// phone's default is. Of two devices on one seat, the one whose setup reached the relay
    /// second is told (`seat_lost`) and posts nothing more.
    pub fn join_offer(&self, text: String, seat: u8) -> Result<String, PhoneError> {
        let (channel, params, relay) =
            Channel::from_offer(&text).map_err(|_| PhoneError::BadCode {
                message: "That is not a host's code to join through the relay.".into(),
            })?;
        let relay = chat::check_relay(&relay).map_err(|_| PhoneError::BadCode {
            message: "The host's code names a relay address that does not read.".into(),
        })?;
        check_seat(&params, seat)?;
        let (id, p) = self.begin(params, seat, Some(channel), Some(relay))?;
        self.lock().ceremonies.insert(id.clone(), p);
        Ok(id)
    }

    /// One step of a ceremony held through the relay: post this seat's messages for the
    /// rounds it has reached, once each, then read and apply everyone else's. Call it every
    /// 2 seconds. When `needs_signature` says so, call `ceremony_sign`, then tick again. A
    /// phone that lost its seat gets the same error from every tick and posts nothing.
    pub fn ceremony_tick(&self, id: String) -> Result<CeremonyState, PhoneError> {
        self.with_ceremony(&id, |p| {
            p.tick(self.http.as_ref())?;
            Ok(Self::state_of(p))
        })
    }

    /// The host, once everyone is scanned (`needs_continue`), lets the ceremony go on.
    pub fn ceremony_continue(&self, id: String) -> Result<CeremonyState, PhoneError> {
        self.with_ceremony(&id, |p| {
            p.proceed()?;
            Ok(Self::state_of(p))
        })
    }

    /// The relay new ceremonies and new circles use.
    pub fn default_relay(&self) -> String {
        self.lock().relay.clone()
    }

    /// Set the relay new ceremonies and new circles use. Everyone in a ceremony held through
    /// the relay must use the same one. A circle already kept keeps its own (`set_relay`).
    pub fn set_default_relay(&self, url: String) -> Result<(), PhoneError> {
        let url = chat::check_relay(&url)?;
        let mut st = self.lock();
        let mut me = self.me.clone();
        me.relay = Some(url.clone());
        me.save(&self.dir)?;
        st.relay = url;
        Ok(())
    }

    /// Read one code the camera saw before joining: a setup line or setup code as text, or
    /// a frame of a host's step-one animation (the browser app shows only the animation).
    /// `Some` once a whole code is in; hand its `params_code` to `join_ceremony`.
    pub fn join_scan(&self, text: String) -> Result<Option<JoinOffer>, PhoneError> {
        if text.trim_start().starts_with(OFFER_PREFIX) {
            let (_, params, _) = Channel::from_offer(&text).map_err(|_| PhoneError::BadCode {
                message: "That host's code does not read.".into(),
            })?;
            return Ok(Some(JoinOffer {
                params_code: text.trim().to_string(),
                members: params.n,
                name: params.name,
                through_relay: true,
            }));
        }
        let code = if text.trim_start().starts_with("cahoots-") {
            ceremony::parse_tagged(&text).ok_or_else(|| PhoneError::BadCode {
                message: "That is not a code to join a circle from.".into(),
            })?
        } else {
            let bytes = URL_SAFE_NO_PAD
                .decode(text.trim())
                .map_err(|_| PhoneError::BadCode {
                    message: "That is not a Cahoots code.".into(),
                })?;
            let mut st = self.lock();
            match st.join_receiver.add(&bytes)? {
                None => return Ok(None),
                Some(data) => ceremony::parse_packed(&data).ok_or_else(|| PhoneError::BadCode {
                    message: "That Cahoots code does not read.".into(),
                })?,
            }
        };
        let params_code = match &code {
            ceremony::Code::Commit {
                seat,
                bytes,
                params: Some(p),
            } => ceremony::commit_code(*seat, bytes, p),
            _ => text.trim().to_string(),
        };
        let (params, _) = ceremony::join_from(code)?;
        Ok(Some(JoinOffer {
            params_code,
            members: params.n,
            name: params.name,
            through_relay: false,
        }))
    }

    /// The number of seats in the circle a join code is for, to offer a seat picker.
    pub fn ceremony_members(&self, params_code: String) -> Result<u8, PhoneError> {
        if let Ok((_, params, _)) = Channel::from_offer(&params_code) {
            return Ok(params.n);
        }
        Ok(ceremony::read_join(&params_code)?.0.n)
    }

    /// Join as `seat` (from 0) from a join code: a setup line (`cahoots-params:`, the first
    /// code `ceremony_frames` shows in step one), a setup code that carries the circle's
    /// parameters (`cahoots-commit:`), or a `JoinOffer`'s `params_code`. Seat 0 is the
    /// host's. Each person picks a different seat, as in the browser app; two phones on one
    /// seat are told when they scan each other.
    pub fn join_ceremony(&self, params_code: String, seat: u8) -> Result<String, PhoneError> {
        let (params, kept) = ceremony::read_join(&params_code)?;
        let refuse = |message: String| Err(PhoneError::Ceremony { message });
        check_seat(&params, seat)?;
        if let Some(ceremony::Code::Commit { seat: theirs, .. }) = &kept {
            if *theirs == seat {
                return refuse(format!(
                    "Seat {} is taken by the person you scanned. Pick another seat.",
                    seat + 1
                ));
            }
        }
        let (id, mut p) = self.begin(params, seat, None, None)?;
        if let Some(code) = kept {
            p.apply_kept(code)?;
        }
        self.lock().ceremonies.insert(id.clone(), p);
        Ok(id)
    }

    /// The codes to show now, as text payloads to draw one after another in a loop. In step
    /// three they carry this seat's reveal, for anyone still on step two, and its signature
    /// once `ceremony_sign` has made it.
    pub fn ceremony_frames(&self, id: String) -> Result<Vec<String>, PhoneError> {
        self.with_ceremony(&id, |p| p.frames())
    }

    /// Sign the circle's manifest with this seat's key, once, when `needs_signature` says so.
    /// The `Signer` may ask for Face ID; `Locked` changes nothing and can be tried again.
    pub fn ceremony_sign(&self, id: String) -> Result<CeremonyState, PhoneError> {
        self.with_ceremony(&id, |p| {
            p.sign(self.signer.as_ref())?;
            Ok(Self::state_of(p))
        })
    }

    /// Feed one code the camera read, or text pasted in. On the host of a ceremony through the
    /// relay this takes each joiner's code (`cahoots:joined:v1:...`).
    pub fn ceremony_scan(&self, id: String, text: String) -> Result<CeremonyState, PhoneError> {
        self.with_ceremony(&id, |p| {
            p.scan(&text)?;
            Ok(Self::state_of(p))
        })
    }

    /// Where the ceremony stands, without scanning anything.
    pub fn ceremony_state(&self, id: String) -> Result<CeremonyState, PhoneError> {
        self.with_ceremony(&id, |p| Ok(Self::state_of(p)))
    }

    /// Write the circle, after the words were read aloud and match. Returns the circle id.
    pub fn ceremony_keep(&self, id: String) -> Result<String, PhoneError> {
        let mut st = self.lock();
        let p = st.ceremonies.get(&id).ok_or_else(ceremony_lost)?;
        let r = p.result.as_ref().ok_or_else(|| PhoneError::Ceremony {
            message: "The ceremony is not finished. Every signature is needed first.".into(),
        })?;
        let root = r.root.as_ref().ok_or_else(|| PhoneError::Ceremony {
            message: "The ceremony made no circle root.".into(),
        })?;
        let id_hex = hex::encode(r.circle_id);
        let n = usize::from(r.params.n);
        let record = CircleRecord {
            params: r.params.encode().into(),
            circle_id: r.circle_id.to_vec().into(),
            seat: p.seat,
            tier: r.tier,
            label: "CSPRNG".into(),
            words: r.check_words.iter().map(|w| w.to_string()).collect(),
            manifest: r.manifest.encode().into(),
            root: root.to_vec().into(),
            seat_label: p.seat_label.clone(),
            seat_public: p.seat_public.0.to_vec().into(),
            pair_keys: r
                .pair_keys
                .iter()
                .map(|k| k.as_ref().map(|k| k.to_vec()).unwrap_or_default().into())
                .collect(),
            pair_counters: vec![0; n],
        };
        let session = cahoots_chat::ChatSession::new(root, r.circle_id, r.params.n, p.seat);
        let chat = ChatRecord {
            state: session.export_state().to_vec().into(),
            outbox: Vec::new(),
            journal: Vec::new(),
        };
        store::create_circle(
            &self.dir,
            &id_hex,
            &NewCircle {
                record: &record,
                pad: &r.pad,
                chat: &chat,
                settings: &SettingsRecord {
                    relay: p.relay.clone(),
                    unread: 0,
                },
                slices: r.params.slices(),
            },
        )?;
        st.ceremonies.remove(&id);
        Ok(id_hex)
    }

    /// Throw a ceremony away. Nothing of it was written. Its seat key, made under
    /// `cahoots:seat:<ceremony id>`, is Swift's to delete.
    pub fn ceremony_discard(&self, id: String) {
        self.lock().ceremonies.remove(&id);
    }

    /// Decode the first QR code in a grayscale camera frame (`width * height` bytes, row by row).
    pub fn decode_frame(&self, width: u32, height: u32, gray: Vec<u8>) -> Option<String> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || gray.len() != w * h {
            return None;
        }
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| gray[y * w + x]);
        img.detect_grids()
            .into_iter()
            .find_map(|g| g.decode().ok().map(|(_, text)| text))
    }

    /// The modules of a QR code for `text`, low error correction for density, as the
    /// browser app draws them.
    pub fn qr_matrix(&self, text: String) -> Result<QrMatrix, PhoneError> {
        use qrcode::{Color, EcLevel, QrCode};
        let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L).map_err(
            |_| PhoneError::BadCode {
                message: "That is too much to fit in one code.".into(),
            },
        )?;
        Ok(QrMatrix {
            size: code.width() as u32,
            bits: code
                .to_colors()
                .into_iter()
                .map(|c| c == Color::Dark)
                .collect(),
        })
    }

    /// Encrypt and send. Reads the relay first if this phone has not since it opened. A
    /// message the relay cannot take now waits, shown as pending, and goes with the next poll.
    pub fn send_chat(&self, circle: String, text: String) -> Result<(), PhoneError> {
        self.with_circle(&circle, |c, me| c.send(me.http.as_ref(), &text))
    }

    /// Read the relay after the cursor, keep and return what others sent, then send what waits.
    pub fn poll(&self, circle: String) -> Result<Vec<ChatMessage>, PhoneError> {
        self.with_circle(&circle, |c, me| c.poll(me.http.as_ref()))
    }

    /// Every message kept, oldest first. Clears the unread count.
    pub fn history(&self, circle: String) -> Vec<ChatMessage> {
        self.with_circle(&circle, |c, _| Ok(c.history()))
            .unwrap_or_default()
    }

    /// Send from the next epoch on.
    pub fn rotate(&self, circle: String) -> Result<(), PhoneError> {
        self.with_circle(&circle, |c, _| c.rotate())
    }

    /// The hello words with `other_seat` at the current hello number.
    pub fn hello_words(&self, circle: String, other_seat: u8) -> Result<HelloWords, PhoneError> {
        self.with_circle(&circle, |c, _| hello(c, other_seat))
    }

    /// Move to the next hello words with `other_seat`. Both people do this.
    pub fn next_hello(&self, circle: String, other_seat: u8) -> Result<HelloWords, PhoneError> {
        self.with_circle(&circle, |c, _| {
            hello(c, other_seat)?;
            let i = usize::from(other_seat);
            c.record.pair_counters[i] = c.record.pair_counters[i].wrapping_add(1);
            c.record.save(&c.dir)?;
            hello(c, other_seat)
        })
    }

    /// Every member must use the same relay.
    pub fn set_relay(&self, circle: String, url: String) -> Result<(), PhoneError> {
        self.with_circle(&circle, |c, _| c.set_relay(&url))
    }

    /// The history as readable text. Nothing secret is in it.
    pub fn export_history(&self, circle: String) -> Result<String, PhoneError> {
        self.with_circle(&circle, |c, _| {
            let unread = c.settings.unread;
            let msgs = c.history();
            // Exporting is not reading.
            if unread != 0 {
                c.settings.unread = unread;
                c.settings.save(&c.dir)?;
            }
            let mut out = format!(
                "Cahoots chat history\nCircle: {}\nPeople: {}\nYou are seat {}\n\n",
                c.params.name,
                c.params.n,
                c.record.seat + 1
            );
            for m in msgs {
                let who = if m.mine {
                    "You".to_string()
                } else {
                    format!("Seat {}", m.sender + 1)
                };
                let note = if m.pending { " (not sent yet)" } else { "" };
                out.push_str(&format!("{who}{note}: {}\n", m.text));
            }
            Ok(out)
        })
    }

    /// Delete the circle's records from this phone. Its seat key stays in the enclave until
    /// Swift deletes it under the label `circle_seat_label` gives.
    pub fn delete_circle(&self, circle: String) -> Result<(), PhoneError> {
        let mut st = self.lock();
        if !store::circle_ids(&self.dir).contains(&circle) {
            return Err(not_found("circle"));
        }
        st.circles.remove(&circle);
        std::fs::remove_dir_all(self.circle_dir(&circle)).map_err(store::io)
    }

    /// The `Signer` label of a circle's seat key.
    pub fn circle_seat_label(&self, circle: String) -> Result<String, PhoneError> {
        self.with_circle(&circle, |c, _| Ok(c.record.seat_label.clone()))
    }

    pub fn describe(&self) -> PhoneInfo {
        PhoneInfo {
            key_store: KEY_STORE.into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            fingerprint: self.me.fingerprint.clone(),
        }
    }
}

/// A joiner's seat: not the host's, and one the circle has.
fn check_seat(params: &CircleParams, seat: u8) -> Result<(), PhoneError> {
    let refuse = |message: String| Err(PhoneError::Ceremony { message });
    if seat == 0 {
        return refuse("Seat 1 is the host's. Pick another seat.".into());
    }
    if seat >= params.n {
        return refuse(format!(
            "This circle has {} seats. Pick one from 2 to {}.",
            params.n, params.n
        ));
    }
    Ok(())
}

fn hello(c: &Circle, other: u8) -> Result<HelloWords, PhoneError> {
    let none = || PhoneError::NotFound {
        message: "There are no hello words with that seat.".into(),
    };
    let key = c
        .record
        .pair_keys
        .get(usize::from(other))
        .ok_or_else(none)?;
    let key: zeroize::Zeroizing<[u8; 32]> =
        zeroize::Zeroizing::new(key[..].try_into().map_err(|_| none())?);
    let number = c.record.pair_counters[usize::from(other)];
    Ok(HelloWords {
        mine: pairwise::words(&key, c.record.seat, number).join(" "),
        theirs: pairwise::words(&key, other, number).join(" "),
        number,
    })
}

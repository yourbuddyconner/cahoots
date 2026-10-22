//! `cahoots-wasm`: wasm-bindgen bindings over `cahoots-core` for the browser client.
//!
//! Everything here also compiles natively so the API is unit-tested with plain `cargo test`.
//! Errors cross the boundary as short stable strings (`"reuse"`, `"tag"`, ...) that the UI maps
//! to messages.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_chat::{self as chat, ChatError, ChatSession, EpochId};
use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::ledger::{MemLedger, SpentLedger};
use cahoots_core::message::{MessageError, MessageType};
use cahoots_core::pad::{self, MemPadStorage, Pad, PadError, PadStorage};
use cahoots_core::pairwise;
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::PUBLIC_KEY_LEN;
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

#[wasm_bindgen]
pub fn b64_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

#[wasm_bindgen]
pub fn b64_decode(text: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(text.trim())
        .map_err(|_| "base64url".to_string())
}

/// Pad to the next bucket (32/64/128/256) with the length in the last byte.
#[wasm_bindgen]
pub fn bucket_pad(plaintext: &[u8]) -> Option<Vec<u8>> {
    pad::bucket_pad(plaintext)
}

#[wasm_bindgen]
pub fn bucket_unpad(padded: &[u8]) -> Option<Vec<u8>> {
    pad::bucket_unpad(padded).map(<[u8]>::to_vec)
}

/// Chat plaintext buckets (64/256/1024/4096) with a u16 length trailer.
#[wasm_bindgen]
pub fn chat_bucket_pad(plaintext: &[u8]) -> Option<Vec<u8>> {
    chat::bucket_pad(plaintext)
}

#[wasm_bindgen]
pub fn chat_bucket_unpad(padded: &[u8]) -> Option<Vec<u8>> {
    chat::bucket_unpad(padded).map(<[u8]>::to_vec)
}

/// Render `data` as a QR code SVG (byte mode, low error correction for density).
#[wasm_bindgen]
pub fn qr_svg(data: &[u8]) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(data, EcLevel::L).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build())
}

/// Decode the first QR code found in a grayscale frame (`width * height` bytes, row-major).
/// Returns `None` when nothing decodes. This is the camera path on every browser, including
/// iOS, where WebKit has no `BarcodeDetector`.
#[wasm_bindgen]
pub fn qr_decode_gray(width: u32, height: u32, gray: &[u8]) -> Option<String> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || gray.len() != w * h {
        return None;
    }
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| gray[y * w + x]);
    for grid in img.detect_grids() {
        if let Ok((_, text)) = grid.decode() {
            return Some(text);
        }
    }
    None
}

/// Decode from RGBA pixels as `canvas.getImageData` returns them.
#[wasm_bindgen]
pub fn qr_decode_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<String> {
    let n = width as usize * height as usize;
    if rgba.len() != n * 4 {
        return None;
    }
    let gray: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8)
        .collect();
    qr_decode_gray(width, height, &gray)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------
// Ceremony
// ---------------------------------------------------------------------------------------------

fn ceremony_err(e: CeremonyError) -> String {
    match e {
        CeremonyError::BadIndex(i) => format!("bad-index:{i}"),
        CeremonyError::ContributionLength => "contribution-length".into(),
        CeremonyError::WrongPhase => "wrong-phase".into(),
        CeremonyError::ConflictingCommit(i) => format!("conflicting-commit:{i}"),
        CeremonyError::BadKey(i) => format!("bad-key:{i}"),
        CeremonyError::Corrupted(i) => format!("corrupted:{i}"),
        CeremonyError::CommitMismatch(i) => format!("commit-mismatch:{i}"),
        CeremonyError::Aborted => "aborted".into(),
    }
}

fn params_err(e: cahoots_core::params::ParamsError) -> String {
    format!("params:{e:?}").to_lowercase()
}

/// One member's view of a ceremony in progress.
#[wasm_bindgen(js_name = Ceremony)]
pub struct WasmCeremony {
    inner: Ceremony,
}

#[wasm_bindgen(js_class = Ceremony)]
impl WasmCeremony {
    /// `contribution` is `L` bytes from `crypto.getRandomValues`. `extra` (optional) is a
    /// hardware contribution of the same length, for example from `cahoots-yk`; it is XORed in.
    /// `kem_seed` is 32 more random bytes; it derives the key other members seal reveals to.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)] // a JavaScript-facing constructor; the arguments are the ceremony's inputs
    pub fn new(
        n: u8,
        pad_len: u32,
        name: &str,
        agent_seat: bool,
        index: u8,
        contribution: &[u8],
        extra: Option<Vec<u8>>,
        kem_seed: &[u8],
    ) -> Result<WasmCeremony, String> {
        let params = CircleParams::new(n, pad_len, name, agent_seat).map_err(params_err)?;
        let seed: [u8; 32] = kem_seed
            .try_into()
            .map_err(|_| "kem-seed-length".to_string())?;
        let inner = match extra {
            Some(x) => Ceremony::with_extra_source(params, index, contribution.to_vec(), &x, seed),
            None => Ceremony::new(params, index, contribution.to_vec(), seed),
        }
        .map_err(ceremony_err)?;
        Ok(WasmCeremony { inner })
    }

    /// Canonical parameter bytes, shown to every member for confirmation and shared with joiners.
    pub fn params_bytes(&self) -> Vec<u8> {
        self.inner.params().encode()
    }

    pub fn index(&self) -> u8 {
        self.inner.index()
    }

    pub fn phase(&self) -> String {
        match self.inner.phase() {
            Phase::Commit => "commit",
            Phase::Reveal => "reveal",
            Phase::Complete => "complete",
            Phase::Aborted => "aborted",
        }
        .into()
    }

    /// Our commitment followed by our KEM public key: 32 + 1216 bytes. Public.
    pub fn commitment(&self) -> Vec<u8> {
        let mut out = self.inner.my_commitment().to_vec();
        out.extend_from_slice(self.inner.my_public_key());
        out
    }

    /// `commitment` is the 32 + 1216 bytes another member's `commitment()` returned.
    pub fn add_commitment(&mut self, from: u8, commitment: &[u8]) -> Result<(), String> {
        if commitment.len() != 32 + PUBLIC_KEY_LEN {
            return Err("commitment-length".to_string());
        }
        let c: [u8; 32] = commitment[..32].try_into().expect("32 bytes");
        self.inner
            .add_commitment(from, c, &commitment[32..])
            .map_err(ceremony_err)
    }

    /// Random bytes `reveal` needs the first time it is called.
    pub fn reveal_randomness_len(&self) -> usize {
        self.inner.reveal_randomness_len()
    }

    /// Our contribution sealed to every other member. `None` until every commitment is held.
    /// Built once; pass `reveal_randomness_len()` bytes from `crypto.getRandomValues`.
    pub fn reveal(&mut self, randomness: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let mut pos = 0usize;
        let mut short = false;
        let mut fill = |buf: &mut [u8]| {
            if pos + buf.len() > randomness.len() {
                short = true;
                buf.fill(0);
            } else {
                buf.copy_from_slice(&randomness[pos..pos + buf.len()]);
                pos += buf.len();
            }
        };
        let out = self.inner.my_reveal(&mut fill).map(<[u8]>::to_vec);
        if short {
            return Err("randomness-length".to_string());
        }
        Ok(out)
    }

    /// A sealed reveal from member `from`. Before every commitment is held it also stands in
    /// for that member's commitment.
    pub fn add_reveal(&mut self, from: u8, reveal: &[u8]) -> Result<(), String> {
        self.inner.add_reveal(from, reveal).map_err(ceremony_err)
    }

    pub fn missing_commitments(&self) -> Vec<u8> {
        self.inner.missing_commitments()
    }

    pub fn missing_reveals(&self) -> Vec<u8> {
        self.inner.missing_reveals()
    }

    /// Erase contributions and produce the circle. `label` is `"TRNG"` or `"CSPRNG"`.
    pub fn finish(self, label: &str) -> Result<CeremonyOutput, String> {
        let label = Label::parse(label)?;
        let index = self.inner.index();
        let r = self.inner.finish().map_err(ceremony_err)?;
        let words = r.check_words.join(" ");
        let slices = r.params.slices();
        let storage = MemPadStorage::new(r.pad.to_vec());
        let pad = Pad::with_circle_id(
            r.params,
            index,
            r.circle_id,
            storage,
            MemLedger::new(slices),
        )
        .map_err(pad_err)?;
        Ok(CeremonyOutput {
            words,
            circle: Some(Circle::from_pad_with_pairs(pad, label, r.pair_keys)),
        })
    }
}

#[wasm_bindgen]
pub struct CeremonyOutput {
    words: String,
    circle: Option<Circle>,
}

#[wasm_bindgen]
impl CeremonyOutput {
    /// Six check words separated by single spaces.
    pub fn words(&self) -> String {
        self.words.clone()
    }
    pub fn circle_id_hex(&self) -> String {
        self.circle
            .as_ref()
            .map(Circle::circle_id_hex)
            .unwrap_or_default()
    }
    pub fn into_circle(mut self) -> Circle {
        self.circle.take().expect("circle is present until taken")
    }
}

// ---------------------------------------------------------------------------------------------
// Circle (pad + ledger + label), with export/import for IndexedDB
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Label {
    Csprng = 0,
    Trng = 1,
}

impl Label {
    fn parse(s: &str) -> Result<Label, String> {
        match s {
            "TRNG" => Ok(Label::Trng),
            "CSPRNG" => Ok(Label::Csprng),
            _ => Err("label".into()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Label::Trng => "TRNG",
            Label::Csprng => "CSPRNG",
        }
    }
}

fn pad_err(e: PadError) -> String {
    match e {
        PadError::WrongCircle => "wrong-circle".into(),
        PadError::OutOfRange => "out-of-range".into(),
        PadError::Reuse => "reuse".into(),
        PadError::Exhausted => "exhausted".into(),
        PadError::Config => "config".into(),
        PadError::Message(MessageError::Tag) => "tag".into(),
        PadError::Message(m) => format!("message:{m:?}").to_lowercase(),
        PadError::Ledger(l) => format!("ledger:{l:?}").to_lowercase(),
        PadError::Storage(s) => format!("storage:{s:?}").to_lowercase(),
    }
}

/// State format: v1 is pad + ledger + label; v2 appends chat session state and the relay seq.
const STATE_VERSION: u8 = 3;

fn chat_err(e: ChatError) -> String {
    match e {
        ChatError::Version => "version".into(),
        ChatError::Type => "type".into(),
        ChatError::Length => "length".into(),
        ChatError::Tag => "tag".into(),
        ChatError::WrongCircle => "wrong-circle".into(),
        ChatError::BadSender => "bad-sender".into(),
        ChatError::Reuse => "reuse".into(),
        ChatError::Replay => "replay".into(),
        ChatError::TooFarAhead => "too-far-ahead".into(),
        ChatError::UnknownEpoch => "unknown-epoch".into(),
        ChatError::State => "state".into(),
        ChatError::Pad(p) => pad_err(p),
    }
}

fn epoch_str(id: EpochId) -> String {
    format!("{}:{}", id.slice, id.offset)
}

/// A circle on this device: the pad and ledger, the pad-seeded chat session over them,
/// the randomness label, and the last relay sequence number this device has consumed.
#[wasm_bindgen]
pub struct Circle {
    chat: ChatSession<MemPadStorage, MemLedger>,
    label: Label,
    relay_seq: Option<u64>,
    /// One X-Wing-derived key per other member, from the sealed reveals. See `cahoots_core::pairwise`.
    pair_keys: Vec<Option<Zeroizing<[u8; 32]>>>,
    /// Where the hello words with each member currently stand.
    pair_counters: Vec<u32>,
}

/// A decrypted chat message.
#[wasm_bindgen]
#[derive(Debug)]
pub struct ChatReceived {
    text: Vec<u8>,
    sender: u8,
    counter: u32,
    epoch: String,
}

#[wasm_bindgen]
impl ChatReceived {
    /// Plaintext bytes, still bucket-padded if the sender padded them.
    pub fn text(&self) -> Vec<u8> {
        self.text.clone()
    }
    pub fn sender(&self) -> u8 {
        self.sender
    }
    pub fn counter(&self) -> u32 {
        self.counter
    }
    /// `"<slice>:<offset>"` of the 32 pad bytes that key this epoch.
    pub fn epoch(&self) -> String {
        self.epoch.clone()
    }
}

impl Circle {
    fn from_pad(pad: Pad<MemPadStorage, MemLedger>, label: Label) -> Circle {
        let n = usize::from(pad.params().n);
        Circle::from_pad_with_pairs(pad, label, vec![None; n])
    }
    fn from_pad_with_pairs(
        pad: Pad<MemPadStorage, MemLedger>,
        label: Label,
        pair_keys: Vec<Option<Zeroizing<[u8; 32]>>>,
    ) -> Circle {
        let me = pad.my_slice();
        let n = pair_keys.len();
        Circle {
            chat: ChatSession::new(pad, me),
            label,
            relay_seq: None,
            pair_keys,
            pair_counters: vec![0; n],
        }
    }
    fn pad(&self) -> &Pad<MemPadStorage, MemLedger> {
        self.chat.pad()
    }
    fn pad_mut(&mut self) -> &mut Pad<MemPadStorage, MemLedger> {
        self.chat.pad_mut()
    }
}

/// A decrypted message.
/// The hello words with one member at one counter value.
#[wasm_bindgen]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloWords {
    mine: String,
    theirs: String,
    counter: u32,
}

#[wasm_bindgen]
impl HelloWords {
    /// Two words you say, separated by a space.
    pub fn mine(&self) -> String {
        self.mine.clone()
    }
    /// Two words they say back.
    pub fn theirs(&self) -> String {
        self.theirs.clone()
    }
    pub fn counter(&self) -> u32 {
        self.counter
    }
}

#[wasm_bindgen]
#[derive(Debug)]
pub struct Received {
    plaintext: Vec<u8>,
    msg_type: u8,
    slice: u8,
    offset: u32,
}

#[wasm_bindgen]
impl Received {
    pub fn plaintext(&self) -> Vec<u8> {
        self.plaintext.clone()
    }
    pub fn msg_type(&self) -> u8 {
        self.msg_type
    }
    pub fn slice(&self) -> u8 {
        self.slice
    }
    pub fn offset(&self) -> u32 {
        self.offset
    }
}

#[wasm_bindgen]
impl Circle {
    pub fn circle_id_hex(&self) -> String {
        hex(&self.pad().circle_id())
    }
    pub fn name(&self) -> String {
        self.pad().params().name.clone()
    }
    pub fn label(&self) -> String {
        self.label.as_str().into()
    }
    pub fn my_slice(&self) -> u8 {
        self.pad().my_slice()
    }
    pub fn members(&self) -> u8 {
        self.pad().params().n
    }
    pub fn agent_seat(&self) -> bool {
        self.pad().params().agent_seat
    }
    pub fn pad_len(&self) -> u32 {
        self.pad().params().pad_len
    }
    pub fn slice_len(&self) -> u32 {
        self.pad().params().slice_len()
    }
    /// Unspent bytes left in this member's slice.
    pub fn remaining(&self) -> u32 {
        self.pad().remaining()
    }

    // ---- pad-seeded chat (docs/chat.md) ----

    /// `"<slice>:<offset>"` of the epoch this device sends in, if any.
    pub fn current_epoch(&self) -> Option<String> {
        self.chat.current_epoch().map(epoch_str)
    }

    /// Spend 32 bytes from this member's slice and switch to a fresh epoch.
    pub fn rotate_epoch(&mut self) -> Result<String, String> {
        self.chat.start_epoch().map(epoch_str).map_err(chat_err)
    }

    /// Encrypt a chat message (starting an epoch if needed) and return base64url wire bytes.
    pub fn chat_send(&mut self, plaintext: &[u8]) -> Result<String, String> {
        let wire = self.chat.send(plaintext).map_err(chat_err)?;
        Ok(b64_encode(&wire))
    }

    /// Verify and decrypt a chat message. Errors: `reuse`, `tag`, `replay`, `too-far-ahead`,
    /// `wrong-circle`, `version`, `length`, `base64url`.
    pub fn chat_receive(&mut self, wire_b64: &str) -> Result<ChatReceived, String> {
        let wire = b64_decode(wire_b64)?;
        let (hdr, text) = self.chat.receive(&wire).map_err(chat_err)?;
        Ok(ChatReceived {
            text,
            sender: hdr.sender,
            counter: hdr.counter,
            epoch: epoch_str(hdr.epoch),
        })
    }

    /// Last relay sequence number consumed, persisted with the circle.
    pub fn relay_seq(&self) -> Option<u64> {
        self.relay_seq
    }

    /// Whether this circle holds a pair key with `peer` (false for yourself and for circles made
    /// before sealed reveals).
    pub fn has_pair_key(&self, peer: u8) -> bool {
        self.pair_keys
            .get(usize::from(peer))
            .map(Option::is_some)
            .unwrap_or(false)
    }
    pub fn pair_counter(&self, peer: u8) -> u32 {
        self.pair_counters
            .get(usize::from(peer))
            .copied()
            .unwrap_or(0)
    }
    pub fn set_pair_counter(&mut self, peer: u8, counter: u32) {
        if let Some(c) = self.pair_counters.get_mut(usize::from(peer)) {
            *c = counter;
        }
    }
    /// The hello words with `peer` at the current counter: what you say, what they say back.
    pub fn hello_words(&self, peer: u8) -> Result<HelloWords, String> {
        let key = self
            .pair_keys
            .get(usize::from(peer))
            .and_then(Option::as_ref)
            .ok_or_else(|| "no-pair-key".to_string())?;
        let counter = self.pair_counter(peer);
        let me = self.pad().my_slice();
        Ok(HelloWords {
            mine: pairwise::words(key, me, counter).join(" "),
            theirs: pairwise::words(key, peer, counter).join(" "),
            counter,
        })
    }
    /// Move to the next hello words with `peer`. Both sides do this.
    pub fn next_hello(&mut self, peer: u8) -> Result<HelloWords, String> {
        if !self.has_pair_key(peer) {
            return Err("no-pair-key".into());
        }
        let next = self.pair_counter(peer).wrapping_add(1);
        self.set_pair_counter(peer, next);
        self.hello_words(peer)
    }
    pub fn set_relay_seq(&mut self, seq: Option<u64>) {
        self.relay_seq = seq;
    }

    /// True while the pure-pad path still has a usable key range. Used by tests.
    pub fn pure_pad_still_works(&self) -> bool {
        self.pad().remaining() >= 32
    }

    /// Encrypt and return base64url wire bytes. The pad range is spent and erased first.
    pub fn send(&mut self, msg_type: u8, plaintext: &[u8]) -> Result<String, String> {
        let ty = MessageType::try_from(msg_type).map_err(|_| "type".to_string())?;
        let wire = self.pad_mut().send(ty, plaintext).map_err(pad_err)?;
        Ok(b64_encode(&wire))
    }

    /// Verify and decrypt base64url wire bytes. Errors: `reuse`, `tag`, `wrong-circle`, ...
    pub fn receive(&mut self, wire_b64: &str) -> Result<Received, String> {
        let wire = b64_decode(wire_b64)?;
        let (hdr, pt) = self.pad_mut().receive(&wire).map_err(pad_err)?;
        Ok(Received {
            plaintext: pt,
            msg_type: hdr.msg_type as u8,
            slice: hdr.slice,
            offset: hdr.offset,
        })
    }

    /// Serialize pad, ledger, params, label, chat state, and relay seq. Store it in IndexedDB.
    pub fn export(&self) -> Vec<u8> {
        let pad = self.pad();
        let params = pad.params().encode();
        let slices = pad.params().slices();
        let pad_len = pad.params().pad_len;
        let mut out = Vec::with_capacity(64 + params.len() + pad_len as usize);
        out.push(STATE_VERSION);
        out.push(self.label as u8);
        out.push(pad.my_slice());
        out.extend_from_slice(&pad.circle_id());
        out.extend_from_slice(&(params.len() as u16).to_le_bytes());
        out.extend_from_slice(&params);
        out.push(slices);
        for s in 0..slices {
            let ranges = pad.ledger().ranges(s);
            out.extend_from_slice(&(ranges.len() as u32).to_le_bytes());
            for &(a, b) in ranges {
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
            }
        }
        out.extend_from_slice(&pad_len.to_le_bytes());
        let mut bytes = Zeroizing::new(vec![0u8; pad_len as usize]);
        pad.storage().read(0, &mut bytes);
        out.extend_from_slice(&bytes);
        // v2 trailer: chat state and relay seq.
        let chat_state = self.chat.export_state();
        out.extend_from_slice(&(chat_state.len() as u32).to_le_bytes());
        out.extend_from_slice(&chat_state);
        match self.relay_seq {
            Some(seq) => {
                out.push(1);
                out.extend_from_slice(&seq.to_le_bytes());
            }
            None => out.extend_from_slice(&[0; 9]),
        }
        out.push(self.pair_keys.len() as u8);
        for k in &self.pair_keys {
            match k {
                Some(k) => {
                    out.push(1);
                    out.extend_from_slice(k.as_ref());
                }
                None => out.extend_from_slice(&[0; 33]),
            }
        }
        for c in &self.pair_counters {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out
    }

    pub fn import(state: &[u8]) -> Result<Circle, String> {
        let mut r = Reader(state);
        let err = |_: ()| "state".to_string();
        let version = r.u8().map_err(err)?;
        if !(1..=STATE_VERSION).contains(&version) {
            return Err("state-version".into());
        }
        let label = match r.u8().map_err(err)? {
            0 => Label::Csprng,
            1 => Label::Trng,
            _ => return Err("state".into()),
        };
        let my_slice = r.u8().map_err(err)?;
        let mut circle_id = [0u8; 16];
        circle_id.copy_from_slice(r.take(16).map_err(err)?);
        let plen = usize::from(r.u16().map_err(err)?);
        let params = CircleParams::decode(r.take(plen).map_err(err)?).map_err(params_err)?;
        let params_n = params.n;
        let slices = r.u8().map_err(err)?;
        if slices != params.slices() {
            return Err("state".into());
        }
        let mut ledger = MemLedger::new(slices);
        for s in 0..slices {
            let count = r.u32().map_err(err)?;
            for _ in 0..count {
                let a = r.u32().map_err(err)?;
                let b = r.u32().map_err(err)?;
                ledger
                    .mark_spent(s, a, b.saturating_sub(a))
                    .map_err(|_| "state".to_string())?;
            }
        }
        let pad_len = r.u32().map_err(err)? as usize;
        let bytes = r.take(pad_len).map_err(err)?.to_vec();
        let pad = Pad::with_circle_id(
            params,
            my_slice,
            circle_id,
            MemPadStorage::new(bytes),
            ledger,
        )
        .map_err(pad_err)?;
        if version == 1 {
            if !r.0.is_empty() {
                return Err("state".into());
            }
            return Ok(Circle::from_pad(pad, label));
        }
        let chat_len = r.u32().map_err(err)? as usize;
        let chat_state = r.take(chat_len).map_err(err)?;
        let relay_seq = match r.u8().map_err(err)? {
            0 => {
                r.take(8).map_err(err)?;
                None
            }
            1 => Some(r.u64().map_err(err)?),
            _ => return Err("state".into()),
        };
        let n = usize::from(params_n);
        let (pair_keys, pair_counters) = if version >= 3 {
            if usize::from(r.u8().map_err(err)?) != n {
                return Err("state".into());
            }
            let mut keys = Vec::with_capacity(n);
            for _ in 0..n {
                let present = r.u8().map_err(err)?;
                let bytes = r.take(32).map_err(err)?;
                keys.push(match present {
                    0 => None,
                    1 => {
                        let mut k = Zeroizing::new([0u8; 32]);
                        k.copy_from_slice(bytes);
                        Some(k)
                    }
                    _ => return Err("state".into()),
                });
            }
            let mut counters = Vec::with_capacity(n);
            for _ in 0..n {
                counters.push(r.u32().map_err(err)?);
            }
            (keys, counters)
        } else {
            (vec![None; n], vec![0; n])
        };
        if !r.0.is_empty() {
            return Err("state".into());
        }
        let chat = ChatSession::import_state(pad, my_slice, chat_state).map_err(chat_err)?;
        Ok(Circle {
            chat,
            label,
            relay_seq,
            pair_keys,
            pair_counters,
        })
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ()> {
        if self.0.len() < n {
            return Err(());
        }
        let (h, t) = self.0.split_at(n);
        self.0 = t;
        Ok(h)
    }
    fn u8(&mut self) -> Result<u8, ()> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ()> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, ()> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, ()> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().map_err(|_| ())?))
    }
}

// ---------------------------------------------------------------------------------------------
// Fountain coding for animated QR (RaptorQ, RFC 6330)
// ---------------------------------------------------------------------------------------------

/// Packet magic. Every animated-QR frame starts with it so a scanner can ignore other codes.
const FOUNTAIN_MAGIC: &[u8; 4] = b"CHQ1";
const OTI_LEN: usize = 12;

/// Encodes one transfer into an endless stream of packets. Packet `i` is deterministic, so the
/// sender just displays packets 0, 1, 2, ... at ~10 fps until the receiver says it is done.
#[wasm_bindgen]
pub struct FountainEncoder {
    oti: [u8; OTI_LEN],
    blocks: Vec<Vec<EncodingPacket>>,
    encoder: Encoder,
}

#[wasm_bindgen]
impl FountainEncoder {
    /// `packet_size` is the payload bytes per QR frame. 400 to 800 scans well on phones.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], packet_size: u16) -> FountainEncoder {
        let encoder = Encoder::with_defaults(data, packet_size);
        let blocks = encoder
            .get_block_encoders()
            .iter()
            .map(|b| b.source_packets())
            .collect();
        FountainEncoder {
            oti: encoder.get_config().serialize(),
            blocks,
            encoder,
        }
    }

    /// Number of source packets. A lossless channel needs exactly this many frames.
    pub fn source_packets(&self) -> u32 {
        self.blocks.iter().map(|b| b.len() as u32).sum()
    }

    /// Packet `i`: the source packets first, then an unbounded supply of repair packets.
    pub fn packet(&self, i: u32) -> Vec<u8> {
        let nblocks = self.blocks.len() as u32;
        let block = (i % nblocks) as usize;
        let j = i / nblocks;
        let source = &self.blocks[block];
        let pkt = if (j as usize) < source.len() {
            source[j as usize].clone()
        } else {
            let repair_id = j - source.len() as u32;
            self.encoder.get_block_encoders()[block]
                .repair_packets(repair_id, 1)
                .remove(0)
        };
        let body = pkt.serialize();
        let mut out = Vec::with_capacity(4 + OTI_LEN + body.len());
        out.extend_from_slice(FOUNTAIN_MAGIC);
        out.extend_from_slice(&self.oti);
        out.extend_from_slice(&body);
        out
    }
}

#[wasm_bindgen]
pub struct FountainDecoder {
    oti: Option<[u8; OTI_LEN]>,
    decoder: Option<Decoder>,
    needed: u32,
    seen: Vec<[u8; 4]>,
    result: Option<Vec<u8>>,
}

impl Default for FountainDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl FountainDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> FountainDecoder {
        FountainDecoder {
            oti: None,
            decoder: None,
            needed: 0,
            seen: Vec::new(),
            result: None,
        }
    }

    /// Feed one scanned packet. Returns `true` once the transfer is complete.
    /// Errors: `magic` (not a Cahoots frame), `mixed` (a different transfer), `short`.
    pub fn add(&mut self, packet: &[u8]) -> Result<bool, String> {
        if packet.len() < 4 + OTI_LEN + 4 {
            return Err(if packet.starts_with(FOUNTAIN_MAGIC) {
                "short"
            } else {
                "magic"
            }
            .into());
        }
        if &packet[..4] != FOUNTAIN_MAGIC {
            return Err("magic".into());
        }
        let mut oti = [0u8; OTI_LEN];
        oti.copy_from_slice(&packet[4..4 + OTI_LEN]);
        match self.oti {
            None => {
                let config = ObjectTransmissionInformation::deserialize(&oti);
                let len = config.transfer_length();
                let sym = u64::from(config.symbol_size().max(1));
                self.needed = len.div_ceil(sym).max(1) as u32;
                self.decoder = Some(Decoder::new(config));
                self.oti = Some(oti);
            }
            Some(existing) if existing != oti => return Err("mixed".into()),
            Some(_) => {}
        }
        if self.result.is_some() {
            return Ok(true);
        }
        let body = &packet[4 + OTI_LEN..];
        let mut id = [0u8; 4];
        id.copy_from_slice(&body[..4]);
        if !self.seen.contains(&id) {
            self.seen.push(id);
        }
        let pkt = EncodingPacket::deserialize(body);
        if let Some(data) = self.decoder.as_mut().expect("decoder set").decode(pkt) {
            self.result = Some(data);
        }
        Ok(self.result.is_some())
    }

    pub fn done(&self) -> bool {
        self.result.is_some()
    }

    /// Fraction of the minimum packet count seen so far. Reaches 1.0 at or before completion.
    pub fn progress(&self) -> f64 {
        if self.result.is_some() {
            return 1.0;
        }
        if self.needed == 0 {
            return 0.0;
        }
        (self.seen.len() as f64 / f64::from(self.needed)).min(0.99)
    }

    /// Bytes expected, once the first packet is seen.
    pub fn transfer_length(&self) -> u32 {
        self.oti
            .map(|o| ObjectTransmissionInformation::deserialize(&o).transfer_length() as u32)
            .unwrap_or(0)
    }

    pub fn result(&self) -> Option<Vec<u8>> {
        self.result.clone()
    }
}

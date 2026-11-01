//! A circle's public object: what the badge tells any tap about one circle.
//!
//! One binary object per circle slot in the chip ([`cahoots_chip::KeyKind::Public`]), read by
//! any session and written only by the badge's owner session. The badge writes it when a
//! circle is kept and rewrites it when a phone is paired or unpaired. It is canonical CBOR, a
//! map with nine integer keys, every one present:
//!
//! | key | value |
//! |---|---|
//! | 0 | circle id, bstr 16 |
//! | 1 | relay, text: empty for the phone's own default relay (the badge knows no relay) |
//! | 2 | the sealed manifest, bstr |
//! | 3 | the signed contact card, bstr, or null |
//! | 4 | the device grant for the paired phone, bstr, or null |
//! | 5 | the chat generation after the pending bump, bstr 32: the one a newly paired phone fetches |
//! | 6 | the bumps the badge signed at unpairs that a device has to post, oldest first, each moving from the generation the one before moves to: an array of at most [`MAX_PENDING_BUMPS`] bstr, empty when there are none |
//! | 7 | the badge nonce drawn at first boot, bstr 16, kept for the record (pairing draws its own) |
//! | 8 | the paired phone's public key, bstr 33, or null |
//!
//! The object is made a little larger than its first contents, so a grant, a bump and a card
//! fit later without making it again, and it is zero-filled after the CBOR: a reader decodes
//! one item from the start and the rest must be zeros ([`PublicObject::decode`]).

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::circle_id::CircleId;
use core::convert::Infallible;
use minicbor::data::Type;
use minicbor::{Decoder, Encoder};

/// The most bumps a public object carries. See `crate::Badge::unpair` for how a list at the cap
/// is shortened.
pub const MAX_PENDING_BUMPS: usize = 4;

/// The most bytes a public object holds.
pub const MAX_PUBLIC: usize = 8192;

/// The room a new public object leaves after its first contents, for the device grant, a
/// pending bump, the phone's key and a card written later.
pub const PUBLIC_ROOM: usize = 2048;

/// One circle's public object. See the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicObject {
    pub circle_id: CircleId,
    pub relay: String,
    /// The encoded `cahoots_core::manifest::SealedManifest`.
    pub manifest: Vec<u8>,
    pub card: Option<Vec<u8>>,
    pub grant: Option<Vec<u8>>,
    /// The generation after the pending bump.
    pub generation: [u8; 32],
    /// The bumps the badge signed and no device is known to have posted, oldest first.
    pub pending_bumps: Vec<Vec<u8>>,
    pub badge_nonce: [u8; 16],
    pub phone_public: Option<[u8; 33]>,
}

/// Bytes that are not a public object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

impl PublicObject {
    /// The canonical CBOR, without the zeros after it.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        self.write(&mut e).expect("writing to a Vec cannot fail");
        e.into_writer()
    }

    fn write(&self, e: &mut Encoder<Vec<u8>>) -> Result<(), minicbor::encode::Error<Infallible>> {
        fn opt(
            e: &mut Encoder<Vec<u8>>,
            v: Option<&[u8]>,
        ) -> Result<(), minicbor::encode::Error<Infallible>> {
            match v {
                Some(b) => e.bytes(b)?,
                None => e.null()?,
            };
            Ok(())
        }
        e.map(9)?;
        e.u8(0)?.bytes(&self.circle_id)?;
        e.u8(1)?.str(&self.relay)?;
        e.u8(2)?.bytes(&self.manifest)?;
        e.u8(3)?;
        opt(e, self.card.as_deref())?;
        e.u8(4)?;
        opt(e, self.grant.as_deref())?;
        e.u8(5)?.bytes(&self.generation)?;
        e.u8(6)?.array(self.pending_bumps.len() as u64)?;
        for b in &self.pending_bumps {
            e.bytes(b)?;
        }
        e.u8(7)?.bytes(&self.badge_nonce)?;
        e.u8(8)?;
        opt(e, self.phone_public.as_ref().map(|p| p.as_slice()))?;
        Ok(())
    }

    /// The object as stored in a binary object of `len` bytes: the CBOR, then zeros. `None` if
    /// it does not fit.
    pub fn padded(&self, len: usize) -> Option<Vec<u8>> {
        let mut out = self.encode();
        if out.len() > len {
            return None;
        }
        out.resize(len, 0);
        Some(out)
    }

    /// The length of a new object for these contents: its CBOR and [`PUBLIC_ROOM`], at most
    /// [`MAX_PUBLIC`]. `None` if the contents alone are larger than that.
    pub fn new_len(&self) -> Option<usize> {
        let n = self.encode().len();
        (n <= MAX_PUBLIC).then(|| (n + PUBLIC_ROOM).min(MAX_PUBLIC))
    }

    /// What a new public object holds when these contents do not all fit
    /// [`crate::public::MAX_PUBLIC`]: all of them, else without the card, else without the card
    /// and the device grant (the least a phone reads a circle by). `None` if even that does not
    /// fit.
    pub fn fitted_for_new(&self) -> Option<PublicObject> {
        let no_card = PublicObject { card: None, ..self.clone() };
        let minimal = PublicObject { grant: None, ..no_card.clone() };
        [self.clone(), no_card, minimal].into_iter().find(|o| o.new_len().is_some())
    }

    /// Strict: one canonical map from the start, every key present in order, then only zeros.
    pub fn decode(bytes: &[u8]) -> Result<PublicObject, Malformed> {
        if bytes.len() > MAX_PUBLIC {
            return Err(Malformed);
        }
        let mut d = Decoder::new(bytes);
        if d.map().map_err(|_| Malformed)? != Some(9) {
            return Err(Malformed);
        }
        let key = |d: &mut Decoder, k: u8| -> Result<(), Malformed> {
            (d.u8().map_err(|_| Malformed)? == k).then_some(()).ok_or(Malformed)
        };
        let fixed = |d: &mut Decoder, n: usize| -> Result<Vec<u8>, Malformed> {
            let b = d.bytes().map_err(|_| Malformed)?;
            (b.len() == n).then(|| b.to_vec()).ok_or(Malformed)
        };
        let opt = |d: &mut Decoder| -> Result<Option<Vec<u8>>, Malformed> {
            if d.datatype().map_err(|_| Malformed)? == Type::Null {
                d.skip().map_err(|_| Malformed)?;
                return Ok(None);
            }
            Ok(Some(d.bytes().map_err(|_| Malformed)?.to_vec()))
        };
        key(&mut d, 0)?;
        let circle_id: CircleId = fixed(&mut d, 16)?.try_into().map_err(|_| Malformed)?;
        key(&mut d, 1)?;
        let relay = String::from(d.str().map_err(|_| Malformed)?);
        key(&mut d, 2)?;
        let manifest = d.bytes().map_err(|_| Malformed)?.to_vec();
        key(&mut d, 3)?;
        let card = opt(&mut d)?;
        key(&mut d, 4)?;
        let grant = opt(&mut d)?;
        key(&mut d, 5)?;
        let generation: [u8; 32] = fixed(&mut d, 32)?.try_into().map_err(|_| Malformed)?;
        key(&mut d, 6)?;
        let n = d.array().map_err(|_| Malformed)?.ok_or(Malformed)?;
        if n > MAX_PENDING_BUMPS as u64 {
            return Err(Malformed);
        }
        let mut pending_bumps = Vec::new();
        for _ in 0..n {
            pending_bumps.push(d.bytes().map_err(|_| Malformed)?.to_vec());
        }
        key(&mut d, 7)?;
        let badge_nonce: [u8; 16] = fixed(&mut d, 16)?.try_into().map_err(|_| Malformed)?;
        key(&mut d, 8)?;
        let phone_public = match opt(&mut d)? {
            Some(p) => Some(<[u8; 33]>::try_from(p.as_slice()).map_err(|_| Malformed)?),
            None => None,
        };
        let end = d.position();
        let o = PublicObject {
            circle_id,
            relay,
            manifest,
            card,
            grant,
            generation,
            pending_bumps,
            badge_nonce,
            phone_public,
        };
        if o.encode() != bytes[..end] || bytes[end..].iter().any(|&b| b != 0) {
            return Err(Malformed);
        }
        Ok(o)
    }
}

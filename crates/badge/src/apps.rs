//! The badge's answers to `Chip` requests: the seat key described, attested, and signing for
//! ssh, and the consent screen each signing request shows before it is answered.
//!
//! [`BadgeApps`] holds the circle and the board's chip for one request. [`BadgeApps::prompt`]
//! decides, before anyone is asked, whether a request is answered now, refused now, or shown:
//! a request to sign that the badge cannot describe is refused with `Unparseable` and never
//! reaches a screen. `ssh_sign` runs the same checks again after consent, signs, and verifies
//! the signature with the seat public key before it answers: a signature that does not verify
//! is `ChipError`, never returned.
//!
//! A circle whose seat key is in the chip signs there; one without a chip slot signs with its
//! software seat key and says so in `Describe`. Contact cards, grants and seal keys come later:
//! they answer `NoChip` now, before asking.

use crate::board::Circle;
use crate::chip::ChipSeat;
use crate::screens::{self, Screen};
use crate::ssh::{self, HostCheck, Signed};
use alloc::vec::Vec;
use cahoots_chip::{Attested, ChipOps, KeyId, KeyKind};
use cahoots_core::seat::{SeatKey, SeatPublic, SeatSigner};
use cahoots_custodian::wire::{ChipRequest, ErrorCode, SshHost};
use cahoots_custodian::{ChipApps, ChipInfo, CircleId, CircleInfo, Custodian, PadError, PskHandle};
use zeroize::Zeroizing;

/// `Describe`'s `curve` for a P-256 seat key, the only kind there is.
pub const CURVE_P256: u8 = 0;

/// The namespace git signs commits in.
const GIT_NAMESPACE: &str = "git";

/// The badge's chip requests for the circle it holds.
pub struct BadgeApps<'a> {
    circle: &'a Circle,
    chip: Option<&'a mut dyn ChipOps>,
}

/// A seat key, in the chip or in software.
enum Seat<'a> {
    Chip(ChipSeat<'a>),
    Soft(SeatKey),
}

impl Seat<'_> {
    fn public(&mut self) -> Result<SeatPublic, ErrorCode> {
        match self {
            Seat::Chip(c) => {
                let p = c.public();
                c.error.map_or(Ok(p), |_| Err(ErrorCode::ChipError))
            }
            Seat::Soft(k) => Ok(k.public()),
        }
    }

    fn sign(&mut self, msg: &[u8]) -> Result<[u8; 64], ErrorCode> {
        match self {
            Seat::Chip(c) => {
                let sig = c.sign(msg);
                c.error.map_or(Ok(sig), |_| Err(ErrorCode::ChipError))
            }
            Seat::Soft(k) => Ok(k.sign(msg)),
        }
    }
}

impl<'a> BadgeApps<'a> {
    pub fn new(circle: &'a Circle, chip: Option<&'a mut dyn ChipOps>) -> Self {
        BadgeApps { circle, chip }
    }

    fn held(&self, circle: &CircleId) -> Result<(), ErrorCode> {
        if *circle == self.circle.circle_id {
            Ok(())
        } else {
            Err(ErrorCode::UnknownCircle)
        }
    }

    /// The circle's seat key: in the chip slot it names, or the software key it keeps.
    fn seat(&mut self) -> Result<Seat<'_>, ErrorCode> {
        match self.circle.chip_slot {
            Some(slot) => {
                let chip = self.chip.as_deref_mut().ok_or(ErrorCode::NoChip)?;
                Ok(Seat::Chip(ChipSeat::new(
                    chip,
                    KeyId::new(KeyKind::Seat, slot),
                )))
            }
            None => {
                let seed = Zeroizing::new(self.circle.seat_key);
                SeatKey::from_seed(&seed)
                    .map(Seat::Soft)
                    .map_err(|_| ErrorCode::NoChip)
            }
        }
    }

    /// What to do with a `Chip` request before anyone is asked: `Ok(None)` to answer it now
    /// (it needs no consent), `Ok(Some(screen))` to show the screen and wait for the buttons,
    /// or `Err(code)` to refuse it now.
    pub fn prompt(
        &mut self,
        circle: &CircleId,
        op: &ChipRequest,
    ) -> Result<Option<Screen>, ErrorCode> {
        self.held(circle)?;
        match op {
            ChipRequest::Describe | ChipRequest::Attest { .. } => Ok(None),
            ChipRequest::SshSign {
                data,
                host,
                content,
            } => self
                .ssh_screen(data, host.as_ref(), content.as_deref())
                .map(Some),
            ChipRequest::Card { .. }
            | ChipRequest::GrantSign { .. }
            | ChipRequest::SealKey { .. } => Err(ErrorCode::NoChip),
        }
    }

    /// Check a request to sign for ssh and build the screen that describes it. Anything the
    /// badge cannot describe is `Unparseable`.
    fn ssh_screen(
        &mut self,
        data: &[u8],
        host: Option<&SshHost>,
        content: Option<&[u8]>,
    ) -> Result<Screen, ErrorCode> {
        let signed = ssh::parse(data).map_err(|_| ErrorCode::Unparseable)?;
        let name = &self.circle.params.name;
        match &signed {
            Signed::Login {
                user,
                session_id,
                key,
                host_key,
            } => {
                let public = self.seat()?.public()?;
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
            Signed::Sig { namespace, .. } => {
                if namespace != GIT_NAMESPACE {
                    return Err(ErrorCode::Unparseable);
                }
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

impl ChipApps for BadgeApps<'_> {
    fn describe(&mut self, circle: &CircleId) -> Result<ChipInfo, ErrorCode> {
        self.held(circle)?;
        let in_chip = self.circle.chip_slot.is_some();
        let public = self.seat()?.public()?;
        Ok(ChipInfo {
            public: public.0.to_vec(),
            curve: CURVE_P256,
            in_chip,
            can_attest: in_chip,
        })
    }

    fn attest(&mut self, circle: &CircleId, freshness: &[u8; 16]) -> Result<Attested, ErrorCode> {
        self.held(circle)?;
        let slot = self.circle.chip_slot.ok_or(ErrorCode::NoChip)?;
        let chip = self.chip.as_deref_mut().ok_or(ErrorCode::NoChip)?;
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
        self.held(circle)?;
        // The same checks as before the screen: nothing is signed that was not described.
        self.ssh_screen(data, host, content)?;
        let mut seat = self.seat()?;
        let public = seat.public()?;
        let sig = seat.sign(data)?;
        if !public.verify(data, &sig) {
            return Err(ErrorCode::ChipError);
        }
        Ok(ssh::encode_signature(&sig))
    }

    fn card_sign(&mut self, circle: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        self.held(circle)?;
        Err(ErrorCode::NoChip)
    }

    fn grant_sign(&mut self, circle: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        self.held(circle)?;
        Err(ErrorCode::NoChip)
    }

    fn seal_key(&mut self, circle: &CircleId, _: &str) -> Result<[u8; 32], ErrorCode> {
        self.held(circle)?;
        Err(ErrorCode::NoChip)
    }
}

/// What `serve` is handed on the badge: the pad's custodian and the chip requests together.
pub struct Served<'a, C> {
    pub custody: &'a mut C,
    pub apps: BadgeApps<'a>,
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
        self.apps.card_sign(circle, card)
    }

    fn grant_sign(&mut self, circle: &CircleId, grant: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        self.apps.grant_sign(circle, grant)
    }

    fn seal_key(&mut self, circle: &CircleId, label: &str) -> Result<[u8; 32], ErrorCode> {
        self.apps.seal_key(circle, label)
    }
}

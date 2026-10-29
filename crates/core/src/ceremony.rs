//! The ceremony state machine. Transport-agnostic: the caller feeds commitments and sealed
//! reveals from the ring (devices) or animated QR (phones).
//!
//! Commitments are public: a hash plus the member's KEM public key, seat public key, and
//! declared device tier. Reveals are sealed to
//! every other member (see [`crate::reveal`]), so the link never carries a contribution in
//! the clear. The pad is the XOR of the contributions and never depends on the key exchange.
//!
//! Once every reveal is in, every member builds the same manifest (see [`crate::manifest`])
//! and signs it with its seat key; the ceremony is complete when all `n` attestations are held.
//! Every member then also holds every reveal's content key, from which it derives the circle
//! root (see [`crate::root`]), the same on every card.
//!
//! Two ways to hold the seat key:
//!
//! - [`Ceremony::new`] takes a seed; the ceremony holds the software key, signs the manifest
//!   the moment it is built, and hands the key over in [`CeremonyResult::seat_key`].
//! - [`Ceremony::with_seat_public`] takes only the public key; the key stays where it is (a
//!   chip). When the phase reaches `Attest`, [`Ceremony::my_attestation`] is `None` until the
//!   caller signs with [`Ceremony::sign_manifest`], passing the signer as a
//!   `&mut dyn SeatSigner`. The ceremony never keeps the signer.

use crate::checkwords::check_words;
use crate::circle_id::{circle_id, CircleId};
use crate::combine::combine_into;
use crate::commit::{commit, verify_commit, Commitment};
use crate::cose;
use crate::manifest::{Manifest, SealedManifest, Seat, MANIFEST_AAD};
use crate::pairwise::pair_key;
use crate::params::CircleParams;
use crate::reveal::{self, RevealError, KEM_SEED_LEN, PUBLIC_KEY_LEN};
use crate::root;
use crate::seat::{SeatKey, SeatPublic, SeatSigner};
use alloc::vec;
use alloc::vec::Vec;
use x_wing::DecapsulationKey;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Collecting commitments. Our reveal is withheld.
    Commit,
    /// All commitments held. Collecting reveals.
    Reveal,
    /// All reveals verified; the manifest is built. Collecting attestations.
    Attest,
    /// Every seat's attestation verified. Call `finish`.
    Complete,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeremonyError {
    BadIndex(u8),
    ContributionLength,
    WrongPhase,
    /// A commitment, KEM public key, or seat public key that does not parse.
    BadKey(u8),
    /// The seat seed is zero or not below the curve order; draw another.
    BadSeed,
    /// A declared device tier outside 0 to 4.
    BadTier,
    /// A second, different commitment arrived for this member.
    ConflictingCommit(u8),
    /// A sealed reveal that failed to open: garbled in transit, or not addressed to us.
    /// Nothing changed; receive it again.
    Corrupted(u8),
    /// A reveal did not match its commitment. The ceremony is aborted.
    CommitMismatch(u8),
    /// A second reveal from a member already revealed that matches its commitment but was
    /// sealed again, under another content key or other encapsulations. The first is kept and
    /// nothing changed: the root and pair keys never move once a reveal is held.
    ConflictingReveal(u8),
    /// An attestation that does not verify against the member's seat key, our manifest
    /// bytes, and its key id. Nothing changed; receive it again.
    BadAttestation(u8),
    /// A second, different, valid attestation for a member already held, or one for our own
    /// index that is not ours.
    ConflictingAttestation(u8),
    Aborted,
}

#[derive(Debug)]
pub struct CeremonyResult {
    pub params: CircleParams,
    pub pad: Zeroizing<Vec<u8>>,
    pub check_words: [&'static str; 6],
    pub circle_id: CircleId,
    /// One key per other member, derived from the two X-Wing secrets exchanged with it.
    /// `None` for our own index. See `crate::pairwise`.
    pub pair_keys: Vec<Option<Zeroizing<[u8; 32]>>>,
    /// Our seat's signing key for this circle, when the ceremony was built from a seed. `None`
    /// when it was built from a seat public key: the key lives in a chip. Zeroized on drop.
    pub seat_key: Option<SeatKey>,
    /// The circle root, the same for every member (see `crate::root`). `None` only if our own
    /// reveal was never sealed, which a completed ceremony cannot reach in practice.
    pub root: Option<Zeroizing<[u8; 32]>>,
    /// Our declared device tier.
    pub tier: u8,
    /// The manifest body and every seat's attestation, in seat order.
    pub manifest: SealedManifest,
}

#[derive(Debug)]
pub struct Ceremony {
    params: CircleParams,
    index: u8,
    contribution: Zeroizing<Vec<u8>>,
    dk: DecapsulationKey,
    pk: [u8; PUBLIC_KEY_LEN],
    /// The software seat key, when built from a seed. `None` when the key is held outside.
    seat_key: Option<SeatKey>,
    tier: u8,
    commitments: Vec<Option<Commitment>>,
    public_keys: Vec<Option<[u8; PUBLIC_KEY_LEN]>>,
    /// Seat public keys and declared tiers, stored with each member's commitment.
    seat_publics: Vec<Option<SeatPublic>>,
    tiers: Vec<Option<u8>>,
    /// Running XOR of verified reveals. Individual `R_j` are never stored for long.
    pad: Zeroizing<Vec<u8>>,
    revealed: Vec<bool>,
    /// Contributions opened before every commitment was in (late reveals that stood in for
    /// their commitment). Applied when the reveal phase opens.
    pending: Vec<(u8, Zeroizing<Vec<u8>>)>,
    sealed: Option<Vec<u8>>,
    /// Shared secrets: what we encapsulated to each member, and what each encapsulated to us.
    sent: Vec<Option<Zeroizing<[u8; 32]>>>,
    received: Vec<Option<Zeroizing<[u8; 32]>>>,
    /// Every seat's reveal content key: ours from sealing, the others' from opening.
    content_keys: Vec<Option<Zeroizing<[u8; 32]>>>,
    /// Set when the phase becomes `Attest`: the circle ID and the canonical manifest bytes.
    circle_id: Option<CircleId>,
    manifest: Option<Vec<u8>>,
    /// Verified attestations, per member; ours is made when the manifest is built.
    attestations: Vec<Option<Vec<u8>>>,
    phase: Phase,
}

impl Ceremony {
    /// `contribution` is `L` bytes from the device's randomness. `kem_seed` is 32 more bytes
    /// from the same source; it derives the keypair other members seal their reveals to.
    /// `seat_seed` is 32 more; it is the seat's signing key. `tier` is the device's declared
    /// tier, 0 to 4.
    pub fn new(
        params: CircleParams,
        index: u8,
        contribution: Vec<u8>,
        kem_seed: [u8; KEM_SEED_LEN],
        seat_seed: [u8; 32],
        tier: u8,
    ) -> Result<Self, CeremonyError> {
        Self::check_args(&params, index, &contribution, tier)?;
        let seat_key = SeatKey::from_seed(&seat_seed).map_err(|_| CeremonyError::BadSeed)?;
        let seat_public = seat_key.public();
        Self::build(
            params,
            index,
            contribution,
            kem_seed,
            seat_public,
            Some(seat_key),
            tier,
        )
    }

    /// Like `new`, but the seat key is held outside the ceremony (a chip) and only its public
    /// key comes in: ask the signer for `public()` before calling this, since the commitment
    /// binds it. Once the phase is `Attest`, sign the manifest with `sign_manifest`.
    pub fn with_seat_public(
        params: CircleParams,
        index: u8,
        contribution: Vec<u8>,
        kem_seed: [u8; KEM_SEED_LEN],
        seat_public: SeatPublic,
        tier: u8,
    ) -> Result<Self, CeremonyError> {
        Self::check_args(&params, index, &contribution, tier)?;
        let seat_public =
            SeatPublic::parse(&seat_public.0).map_err(|_| CeremonyError::BadKey(index))?;
        Self::build(
            params,
            index,
            contribution,
            kem_seed,
            seat_public,
            None,
            tier,
        )
    }

    fn check_args(
        params: &CircleParams,
        index: u8,
        contribution: &[u8],
        tier: u8,
    ) -> Result<(), CeremonyError> {
        if index >= params.n {
            return Err(CeremonyError::BadIndex(index));
        }
        if contribution.len() != params.pad_len as usize {
            return Err(CeremonyError::ContributionLength);
        }
        check_tier(tier)
    }

    fn build(
        params: CircleParams,
        index: u8,
        contribution: Vec<u8>,
        kem_seed: [u8; KEM_SEED_LEN],
        seat_public: SeatPublic,
        seat_key: Option<SeatKey>,
        tier: u8,
    ) -> Result<Self, CeremonyError> {
        let n = usize::from(params.n);
        let (dk, pk) = reveal::keypair(&kem_seed);
        let my_commit = commit(&params, index, &pk, &seat_public, tier, &contribution);
        let mut commitments = vec![None; n];
        commitments[usize::from(index)] = Some(my_commit);
        let mut public_keys = vec![None; n];
        public_keys[usize::from(index)] = Some(pk);
        let mut seat_publics = vec![None; n];
        seat_publics[usize::from(index)] = Some(seat_public);
        let mut tiers = vec![None; n];
        tiers[usize::from(index)] = Some(tier);
        // Our own contribution is folded in from the start.
        let mut revealed = vec![false; n];
        revealed[usize::from(index)] = true;
        Ok(Ceremony {
            pad: Zeroizing::new(contribution.clone()),
            params,
            index,
            contribution: Zeroizing::new(contribution),
            dk,
            pk,
            seat_key,
            tier,
            commitments,
            public_keys,
            seat_publics,
            tiers,
            revealed,
            pending: Vec::new(),
            sealed: None,
            sent: vec![None; n],
            received: vec![None; n],
            content_keys: vec![None; n],
            circle_id: None,
            manifest: None,
            attestations: vec![None; n],
            phase: Phase::Commit,
        })
    }

    /// Like `new`, but XORs a second independent source (YubiKey, table TRNG) into the
    /// device contribution first. The extra source never replaces the device TRNG.
    pub fn with_extra_source(
        params: CircleParams,
        index: u8,
        mut contribution: Vec<u8>,
        extra: &[u8],
        kem_seed: [u8; KEM_SEED_LEN],
        seat_seed: [u8; 32],
        tier: u8,
    ) -> Result<Self, CeremonyError> {
        combine_into(&mut contribution, extra).map_err(|_| CeremonyError::ContributionLength)?;
        Self::new(params, index, contribution, kem_seed, seat_seed, tier)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn index(&self) -> u8 {
        self.index
    }
    pub fn params(&self) -> &CircleParams {
        &self.params
    }

    pub fn my_commitment(&self) -> Commitment {
        self.commitments[usize::from(self.index)].expect("own commitment is set")
    }

    /// The KEM public key other members seal their reveals to. Public; travels with the commitment.
    pub fn my_public_key(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.pk
    }

    /// Our seat public key. Public; travels with the commitment.
    pub fn my_seat_public(&self) -> SeatPublic {
        self.seat_publics[usize::from(self.index)].expect("own seat key is set")
    }

    /// Random bytes `my_reveal` needs the first time it is called.
    pub fn reveal_randomness_len(&self) -> usize {
        reveal::randomness_len(self.params.n)
    }

    /// Our contribution sealed to every other member. `None` until every commitment is held.
    /// Built once; later calls return the same bytes and do not call `fill`.
    pub fn my_reveal(&mut self, fill: &mut dyn FnMut(&mut [u8])) -> Option<&[u8]> {
        if !matches!(self.phase, Phase::Reveal | Phase::Attest | Phase::Complete) {
            return None;
        }
        if self.sealed.is_none() {
            let recipients: Vec<(u8, &[u8])> = (0..self.params.n)
                .filter(|&j| j != self.index)
                .map(|j| {
                    (
                        j,
                        self.public_keys[usize::from(j)]
                            .as_ref()
                            .expect("all keys held")
                            .as_slice(),
                    )
                })
                .collect();
            let sealed = reveal::seal(
                &self.params,
                self.index,
                &self.pk,
                &self.seat_publics[usize::from(self.index)].expect("own seat key is set"),
                self.tier,
                &recipients,
                &self.contribution,
                fill,
            )
            .expect("own key and held keys are valid");
            for (to, s) in sealed.secrets {
                self.sent[usize::from(to)] = Some(s);
            }
            self.content_keys[usize::from(self.index)] = Some(sealed.content_key);
            self.sealed = Some(sealed.blob);
        }
        self.sealed.as_deref()
    }

    /// Commitments still missing, for the ring forwarder.
    pub fn missing_commitments(&self) -> Vec<u8> {
        (0..self.params.n)
            .filter(|&i| self.commitments[usize::from(i)].is_none())
            .collect()
    }

    /// Reveals still missing from other members. Never includes our own index.
    pub fn missing_reveals(&self) -> Vec<u8> {
        (0..self.params.n)
            .filter(|&i| !self.revealed[usize::from(i)])
            .collect()
    }

    /// Public key held for a member, once its commitment (or a late reveal) is in.
    pub fn public_key_of(&self, index: u8) -> Option<&[u8; PUBLIC_KEY_LEN]> {
        self.public_keys.get(usize::from(index))?.as_ref()
    }

    /// Seat public key held for a member, once its commitment (or a late reveal) is in.
    pub fn seat_public_of(&self, index: u8) -> Option<&SeatPublic> {
        self.seat_publics.get(usize::from(index))?.as_ref()
    }

    /// Declared device tier held for a member, once its commitment (or a late reveal) is in.
    pub fn tier_of(&self, index: u8) -> Option<u8> {
        *self.tiers.get(usize::from(index))?
    }

    /// A member's commitment with the KEM public key, seat public key, and declared tier it
    /// was made over.
    pub fn add_commitment(
        &mut self,
        from: u8,
        c: Commitment,
        kem_pk: &[u8],
        seat_pk: &[u8],
        tier: u8,
    ) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        reveal::parse_public_key(kem_pk).map_err(|_| CeremonyError::BadKey(from))?;
        let seat_public = SeatPublic::parse(seat_pk).map_err(|_| CeremonyError::BadKey(from))?;
        check_tier(tier)?;
        let mut pk = [0u8; PUBLIC_KEY_LEN];
        pk.copy_from_slice(kem_pk);
        self.store_commitment(from, c, pk, seat_public, tier)?;
        self.advance_if_all_committed();
        Ok(())
    }

    fn store_commitment(
        &mut self,
        from: u8,
        c: Commitment,
        pk: [u8; PUBLIC_KEY_LEN],
        seat_public: SeatPublic,
        tier: u8,
    ) -> Result<(), CeremonyError> {
        let i = usize::from(from);
        match self.commitments[i] {
            Some(existing) => {
                let same = existing == c
                    && self.public_keys[i] == Some(pk)
                    && self.seat_publics[i] == Some(seat_public)
                    && self.tiers[i] == Some(tier);
                if !same {
                    return Err(CeremonyError::ConflictingCommit(from));
                }
            }
            None => {
                self.commitments[i] = Some(c);
                self.public_keys[i] = Some(pk);
                self.seat_publics[i] = Some(seat_public);
                self.tiers[i] = Some(tier);
            }
        }
        Ok(())
    }

    fn advance_if_all_committed(&mut self) {
        if self.phase == Phase::Commit && self.commitments.iter().all(Option::is_some) {
            self.phase = Phase::Reveal;
            let pending = core::mem::take(&mut self.pending);
            for (from, r) in pending {
                // Verified when it was opened; the commitment it stood in for was derived from it.
                self.fold(from, &r);
            }
        }
    }

    fn fold(&mut self, from: u8, r: &[u8]) {
        if !self.revealed[usize::from(from)] {
            combine_into(&mut self.pad, r).expect("verified reveal has pad length");
            self.revealed[usize::from(from)] = true;
        }
        if self.phase == Phase::Reveal && self.revealed.iter().all(|&v| v) {
            self.enter_attest();
        }
    }

    /// Every reveal is in: build the manifest, sign it if we hold the seat key, and collect the
    /// others' signatures.
    fn enter_attest(&mut self) {
        let id = circle_id(&self.pad);
        let seats = (0..self.params.n)
            .map(|j| Seat {
                seat: j,
                key: self.seat_publics[usize::from(j)].expect("all commitments held"),
                tier: self.tiers[usize::from(j)].expect("all commitments held"),
                evidence: Vec::new(),
            })
            .collect();
        let body = Manifest {
            circle_id: id,
            params: self.params.clone(),
            seats,
            floor: Manifest::default_floor(&self.params),
            genesis: 0,
        }
        .encode();
        self.attestations[usize::from(self.index)] = self
            .seat_key
            .as_ref()
            .map(|key| cose::sign1(key, &[self.index], &body, MANIFEST_AAD, true));
        self.circle_id = Some(id);
        self.manifest = Some(body);
        self.phase = Phase::Attest;
        self.complete_if_all_attested();
    }

    fn complete_if_all_attested(&mut self) {
        if self.phase == Phase::Attest && self.attestations.iter().all(Option::is_some) {
            self.phase = Phase::Complete;
        }
    }

    /// Our attestation of the manifest, signed by a seat key held outside the ceremony (built
    /// with `with_seat_public`). The signature is checked against our seat public key before it
    /// is kept: a signer with another key is `BadAttestation(index)` and changes nothing.
    /// `WrongPhase` before `Attest`. Once our attestation is held, later calls do nothing and
    /// do not ask the signer.
    pub fn sign_manifest(&mut self, signer: &mut dyn SeatSigner) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        let Some(body) = self.manifest.as_deref() else {
            return Err(CeremonyError::WrongPhase);
        };
        let i = usize::from(self.index);
        if self.attestations[i].is_some() {
            return Ok(());
        }
        let mine = cose::sign1(&mut *signer, &[self.index], body, MANIFEST_AAD, true);
        let public = self.my_seat_public();
        cose::verify1(&public, &mine, Some(body), MANIFEST_AAD)
            .map_err(|_| CeremonyError::BadAttestation(self.index))?;
        self.attestations[i] = Some(mine);
        self.complete_if_all_attested();
        Ok(())
    }

    /// The circle ID. `None` before `Attest`.
    pub fn circle_id(&self) -> Option<CircleId> {
        self.circle_id
    }

    /// Every seat's reveal content key, in seat order: the input to the circle root (see
    /// `crate::root`). Every member holds the same list. `None` until every reveal is in and
    /// our own has been sealed.
    pub fn reveal_secrets(&self) -> Option<Zeroizing<Vec<[u8; 32]>>> {
        if self.phase == Phase::Aborted {
            return None;
        }
        let mut out = Zeroizing::new(Vec::with_capacity(self.content_keys.len()));
        for k in &self.content_keys {
            out.push(**k.as_ref()?);
        }
        Some(out)
    }

    /// The circle root: `root::derive(reveal_secrets, circle_id)`. `None` before `Attest`.
    pub fn circle_root(&self) -> Option<Zeroizing<[u8; 32]>> {
        let id = self.circle_id?;
        let secrets = self.reveal_secrets()?;
        Some(Zeroizing::new(root::derive(&secrets, &id)))
    }

    /// The canonical manifest bytes every member signs. `None` before `Attest`.
    pub fn manifest(&self) -> Option<&[u8]> {
        self.manifest.as_deref()
    }

    /// Our seat's detached COSE_Sign1 over the manifest, key id `[index]`, external AAD
    /// `cahoots/manifest/v1`. `None` before `Attest`. Built once: with the manifest when the
    /// ceremony holds the seat key, or by `sign_manifest` when it does not.
    pub fn my_attestation(&self) -> Option<Vec<u8>> {
        if self.phase == Phase::Aborted {
            return None;
        }
        self.attestations[usize::from(self.index)].clone()
    }

    /// Members whose attestation is not yet held. All of them before `Attest`.
    pub fn missing_attestations(&self) -> Vec<u8> {
        (0..self.params.n)
            .filter(|&i| self.attestations[usize::from(i)].is_none())
            .collect()
    }

    /// A member's attestation of the manifest. Verified with its seat key against our
    /// manifest bytes; a bad one is `BadAttestation` and changes nothing. An identical
    /// re-forward is accepted; a different valid one is `ConflictingAttestation`.
    pub fn add_attestation(&mut self, from: u8, bytes: &[u8]) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        let Some(body) = self.manifest.as_deref() else {
            return Err(CeremonyError::WrongPhase);
        };
        let i = usize::from(from);
        if from == self.index {
            return if self.attestations[i].as_deref() == Some(bytes) {
                Ok(())
            } else {
                Err(CeremonyError::ConflictingAttestation(from))
            };
        }
        let bad = CeremonyError::BadAttestation(from);
        if cose::kid1(bytes).map_err(|_| bad)? != [from] {
            return Err(bad);
        }
        let key = self.seat_publics[i].as_ref().expect("all commitments held");
        cose::verify1(key, bytes, Some(body), MANIFEST_AAD).map_err(|_| bad)?;
        match &self.attestations[i] {
            Some(held) if held.as_slice() != bytes => {
                return Err(CeremonyError::ConflictingAttestation(from));
            }
            Some(_) => {}
            None => self.attestations[i] = Some(bytes.to_vec()),
        }
        self.complete_if_all_attested();
        Ok(())
    }

    fn abort(&mut self) {
        self.phase = Phase::Aborted;
        self.contribution.fill(0);
        self.pad.fill(0);
        self.pending.clear();
        self.sealed = None;
        self.sent.iter_mut().for_each(|s| *s = None);
        self.received.iter_mut().for_each(|s| *s = None);
        self.content_keys.iter_mut().for_each(|s| *s = None);
    }

    /// Open a sealed reveal from `from` and fold it in.
    ///
    /// Before every commitment is held, a reveal may stand in for its own commitment: the
    /// commitment is derived from the opened contribution, KEM key, seat key, and tier. That is safe for the
    /// receiver because its own contribution was fixed before it showed anyone anything.
    /// Our own reveal is a no-op. A re-forwarded reveal after completion is still verified.
    /// An identical re-forward is accepted; a second, differently sealed reveal from a member
    /// already held is `ConflictingReveal` and changes nothing.
    pub fn add_reveal(&mut self, from: u8, blob: &[u8]) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        if from == self.index {
            return Ok(());
        }
        let opened = match reveal::open(&self.params, self.index, &self.dk, blob) {
            Ok(o) if o.from == from => o,
            Ok(_)
            | Err(RevealError::Malformed)
            | Err(RevealError::NotForMe)
            | Err(RevealError::Corrupted) => {
                return Err(CeremonyError::Corrupted(from));
            }
        };
        let i = usize::from(from);
        match self.commitments[i] {
            Some(expected) => {
                // Verified against the reveal's own seat key and tier: a reveal that carries
                // another seat key or tier than was committed does not match.
                let pk = self.public_keys[i].expect("key stored with commitment");
                if !verify_commit(
                    &self.params,
                    from,
                    &pk,
                    &opened.seat_public,
                    opened.tier,
                    &opened.contribution,
                    &expected,
                ) {
                    self.abort();
                    return Err(CeremonyError::CommitMismatch(from));
                }
            }
            None => {
                check_tier(opened.tier)?;
                let c = commit(
                    &self.params,
                    from,
                    &opened.public_key,
                    &opened.seat_public,
                    opened.tier,
                    &opened.contribution,
                );
                self.store_commitment(from, c, opened.public_key, opened.seat_public, opened.tier)?;
            }
        }
        // The commitment binds the contribution but not the content key or the encapsulations,
        // so the sender could seal the same contribution again and both blobs would verify.
        // Keep the first: a later one with another content key or X-Wing secret is refused and
        // changes nothing, so the root and the pair keys never move.
        let conflicts = |held: &Option<Zeroizing<[u8; 32]>>, new: &Zeroizing<[u8; 32]>| {
            held.as_ref().is_some_and(|h| h.as_ref() != new.as_ref())
        };
        if conflicts(&self.content_keys[i], &opened.content_key)
            || conflicts(&self.received[i], &opened.shared)
        {
            return Err(CeremonyError::ConflictingReveal(from));
        }
        if self.received[i].is_none() {
            self.received[i] = Some(opened.shared.clone());
        }
        if self.content_keys[i].is_none() {
            self.content_keys[i] = Some(opened.content_key.clone());
        }
        if self.phase == Phase::Commit {
            if !self.pending.iter().any(|(j, _)| *j == from) {
                self.pending.push((from, opened.contribution));
            }
            self.advance_if_all_committed();
        } else {
            self.fold(from, &opened.contribution);
        }
        Ok(())
    }

    /// Erase the contribution and hand over the pad, check words, circle ID, seat key (when
    /// held), circle root, and the sealed manifest.
    pub fn finish(self) -> Result<CeremonyResult, CeremonyError> {
        if self.phase != Phase::Complete {
            return Err(CeremonyError::WrongPhase);
        }
        let words = check_words(&self.params, &self.pad);
        let root = self.circle_root();
        let id = self.circle_id.expect("set when the phase became Attest");
        let pair_keys = (0..self.params.n)
            .map(|j| {
                if j == self.index {
                    return None;
                }
                let to_j = self.sent[usize::from(j)].as_ref()?;
                let to_me = self.received[usize::from(j)].as_ref()?;
                Some(pair_key(
                    &self.params,
                    self.index,
                    j,
                    to_j.as_ref(),
                    to_me.as_ref(),
                ))
            })
            .collect();
        let manifest = SealedManifest {
            body: self.manifest.expect("set when the phase became Attest"),
            attestations: self
                .attestations
                .into_iter()
                .map(|a| a.expect("all attestations held when complete"))
                .collect(),
        };
        Ok(CeremonyResult {
            check_words: words,
            circle_id: id,
            pad: self.pad,
            params: self.params,
            pair_keys,
            seat_key: self.seat_key,
            root,
            tier: self.tier,
            manifest,
        })
        // `self.contribution` and `self.dk` are wiped on drop here.
    }
}

fn check_tier(tier: u8) -> Result<(), CeremonyError> {
    if tier <= 4 {
        Ok(())
    } else {
        Err(CeremonyError::BadTier)
    }
}

//! The ceremony state machine. Transport-agnostic: the caller feeds commitments and sealed
//! reveals from the ring (devices) or animated QR (phones).
//!
//! Commitments are public: a hash plus the member's KEM public key. Reveals are sealed to
//! every other member (see [`crate::reveal`]), so the link never carries a contribution in
//! the clear. The pad is the XOR of the contributions and never depends on the key exchange.

use crate::checkwords::check_words;
use crate::circle_id::{circle_id, CircleId};
use crate::combine::combine_into;
use crate::commit::{commit, verify_commit, Commitment};
use crate::pairwise::pair_key;
use crate::params::CircleParams;
use crate::reveal::{self, RevealError, KEM_SEED_LEN, PUBLIC_KEY_LEN};
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
    /// All reveals verified. Call `finish`.
    Complete,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeremonyError {
    BadIndex(u8),
    ContributionLength,
    WrongPhase,
    /// A commitment or public key that does not parse.
    BadKey(u8),
    /// A second, different commitment arrived for this member.
    ConflictingCommit(u8),
    /// A sealed reveal that failed to open: garbled in transit, or not addressed to us.
    /// Nothing changed; receive it again.
    Corrupted(u8),
    /// A reveal did not match its commitment. The ceremony is aborted.
    CommitMismatch(u8),
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
}

#[derive(Debug)]
pub struct Ceremony {
    params: CircleParams,
    index: u8,
    contribution: Zeroizing<Vec<u8>>,
    dk: DecapsulationKey,
    pk: [u8; PUBLIC_KEY_LEN],
    commitments: Vec<Option<Commitment>>,
    public_keys: Vec<Option<[u8; PUBLIC_KEY_LEN]>>,
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
    phase: Phase,
}

impl Ceremony {
    /// `contribution` is `L` bytes from the device's randomness. `kem_seed` is 32 more bytes
    /// from the same source; it derives the keypair other members seal their reveals to.
    pub fn new(
        params: CircleParams,
        index: u8,
        contribution: Vec<u8>,
        kem_seed: [u8; KEM_SEED_LEN],
    ) -> Result<Self, CeremonyError> {
        if index >= params.n {
            return Err(CeremonyError::BadIndex(index));
        }
        if contribution.len() != params.pad_len as usize {
            return Err(CeremonyError::ContributionLength);
        }
        let n = usize::from(params.n);
        let (dk, pk) = reveal::keypair(&kem_seed);
        let my_commit = commit(&params, index, &pk, &contribution);
        let mut commitments = vec![None; n];
        commitments[usize::from(index)] = Some(my_commit);
        let mut public_keys = vec![None; n];
        public_keys[usize::from(index)] = Some(pk);
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
            commitments,
            public_keys,
            revealed,
            pending: Vec::new(),
            sealed: None,
            sent: vec![None; n],
            received: vec![None; n],
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
    ) -> Result<Self, CeremonyError> {
        combine_into(&mut contribution, extra).map_err(|_| CeremonyError::ContributionLength)?;
        Self::new(params, index, contribution, kem_seed)
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

    /// Random bytes `my_reveal` needs the first time it is called.
    pub fn reveal_randomness_len(&self) -> usize {
        reveal::randomness_len(self.params.n)
    }

    /// Our contribution sealed to every other member. `None` until every commitment is held.
    /// Built once; later calls return the same bytes and do not call `fill`.
    pub fn my_reveal(&mut self, fill: &mut dyn FnMut(&mut [u8])) -> Option<&[u8]> {
        if !matches!(self.phase, Phase::Reveal | Phase::Complete) {
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
                &recipients,
                &self.contribution,
                fill,
            )
            .expect("own key and held keys are valid");
            for (to, s) in sealed.secrets {
                self.sent[usize::from(to)] = Some(s);
            }
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

    pub fn add_commitment(
        &mut self,
        from: u8,
        c: Commitment,
        public_key: &[u8],
    ) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        reveal::parse_public_key(public_key).map_err(|_| CeremonyError::BadKey(from))?;
        let mut pk = [0u8; PUBLIC_KEY_LEN];
        pk.copy_from_slice(public_key);
        self.store_commitment(from, c, pk)?;
        self.advance_if_all_committed();
        Ok(())
    }

    fn store_commitment(
        &mut self,
        from: u8,
        c: Commitment,
        pk: [u8; PUBLIC_KEY_LEN],
    ) -> Result<(), CeremonyError> {
        let i = usize::from(from);
        match (&self.commitments[i], &self.public_keys[i]) {
            (Some(existing), Some(existing_pk)) if *existing == c && *existing_pk == pk => {}
            (Some(_), _) => return Err(CeremonyError::ConflictingCommit(from)),
            (None, _) => {
                self.commitments[i] = Some(c);
                self.public_keys[i] = Some(pk);
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
        if self.revealed.iter().all(|&v| v) {
            self.phase = Phase::Complete;
        }
    }

    fn abort(&mut self) {
        self.phase = Phase::Aborted;
        self.contribution.fill(0);
        self.pad.fill(0);
        self.pending.clear();
        self.sealed = None;
        self.sent.iter_mut().for_each(|s| *s = None);
        self.received.iter_mut().for_each(|s| *s = None);
    }

    /// Open a sealed reveal from `from` and fold it in.
    ///
    /// Before every commitment is held, a reveal may stand in for its own commitment: the
    /// commitment is derived from the opened contribution and key. That is safe for the
    /// receiver because its own contribution was fixed before it showed anyone anything.
    /// Our own reveal is a no-op. A re-forwarded reveal after completion is still verified.
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
        self.received[i] = Some(opened.shared.clone());
        match self.commitments[i] {
            Some(expected) => {
                let pk = self.public_keys[i].expect("key stored with commitment");
                if !verify_commit(&self.params, from, &pk, &opened.contribution, &expected) {
                    self.abort();
                    return Err(CeremonyError::CommitMismatch(from));
                }
            }
            None => {
                let c = commit(&self.params, from, &opened.public_key, &opened.contribution);
                self.store_commitment(from, c, opened.public_key)?;
            }
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

    /// Erase the contribution and hand over the pad, check words, and circle ID.
    pub fn finish(self) -> Result<CeremonyResult, CeremonyError> {
        if self.phase != Phase::Complete {
            return Err(CeremonyError::WrongPhase);
        }
        let words = check_words(&self.params, &self.pad);
        let id = circle_id(&self.pad);
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
        Ok(CeremonyResult {
            check_words: words,
            circle_id: id,
            pad: self.pad,
            params: self.params,
            pair_keys,
        })
        // `self.contribution` and `self.dk` are wiped on drop here.
    }
}

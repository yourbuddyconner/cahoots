//! The ceremony state machine (handoff 3.4). Transport-agnostic: the caller feeds
//! commitments and reveals from the ring (devices) or animated QR (phones).

use crate::checkwords::check_words;
use crate::circle_id::{circle_id, CircleId};
use crate::combine::combine_into;
use crate::commit::{commit, verify_commit, Commitment};
use crate::params::CircleParams;
use alloc::vec;
use alloc::vec::Vec;
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
    /// A second, different commitment arrived for this member.
    ConflictingCommit(u8),
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
}

#[derive(Debug)]
pub struct Ceremony {
    params: CircleParams,
    index: u8,
    contribution: Zeroizing<Vec<u8>>,
    commitments: Vec<Option<Commitment>>,
    /// Running XOR of verified reveals. Individual `R_j` are never stored.
    pad: Zeroizing<Vec<u8>>,
    revealed: Vec<bool>,
    phase: Phase,
}

impl Ceremony {
    pub fn new(
        params: CircleParams,
        index: u8,
        contribution: Vec<u8>,
    ) -> Result<Self, CeremonyError> {
        if index >= params.n {
            return Err(CeremonyError::BadIndex(index));
        }
        if contribution.len() != params.pad_len as usize {
            return Err(CeremonyError::ContributionLength);
        }
        let n = usize::from(params.n);
        let my_commit = commit(&params, index, &contribution);
        let mut commitments = vec![None; n];
        commitments[usize::from(index)] = Some(my_commit);
        Ok(Ceremony {
            pad: Zeroizing::new(vec![0u8; params.pad_len as usize]),
            params,
            index,
            contribution: Zeroizing::new(contribution),
            commitments,
            revealed: vec![false; n],
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
    ) -> Result<Self, CeremonyError> {
        combine_into(&mut contribution, extra).map_err(|_| CeremonyError::ContributionLength)?;
        Self::new(params, index, contribution)
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

    /// Our contribution, available only once every commitment is held.
    pub fn my_reveal(&self) -> Option<&[u8]> {
        matches!(self.phase, Phase::Reveal | Phase::Complete).then(|| self.contribution.as_slice())
    }

    /// Commitments still missing, for the ring forwarder.
    pub fn missing_commitments(&self) -> Vec<u8> {
        (0..self.params.n)
            .filter(|&i| self.commitments[usize::from(i)].is_none())
            .collect()
    }

    pub fn missing_reveals(&self) -> Vec<u8> {
        (0..self.params.n)
            .filter(|&i| !self.revealed[usize::from(i)])
            .collect()
    }

    pub fn add_commitment(&mut self, from: u8, c: Commitment) -> Result<(), CeremonyError> {
        if self.phase == Phase::Aborted {
            return Err(CeremonyError::Aborted);
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        let slot = &mut self.commitments[usize::from(from)];
        match slot {
            Some(existing) if *existing == c => {}
            Some(_) => return Err(CeremonyError::ConflictingCommit(from)),
            None => *slot = Some(c),
        }
        if self.phase == Phase::Commit && self.commitments.iter().all(Option::is_some) {
            self.phase = Phase::Reveal;
        }
        Ok(())
    }

    pub fn add_reveal(&mut self, from: u8, r: &[u8]) -> Result<(), CeremonyError> {
        match self.phase {
            Phase::Aborted => return Err(CeremonyError::Aborted),
            Phase::Reveal => {}
            _ => return Err(CeremonyError::WrongPhase),
        }
        if from >= self.params.n {
            return Err(CeremonyError::BadIndex(from));
        }
        let expected = self.commitments[usize::from(from)].expect("all commitments held in Reveal");
        if !verify_commit(&self.params, from, r, &expected) {
            self.phase = Phase::Aborted;
            self.contribution.fill(0);
            self.pad.fill(0);
            return Err(CeremonyError::CommitMismatch(from));
        }
        if !self.revealed[usize::from(from)] {
            combine_into(&mut self.pad, r).expect("verified reveal has pad length");
            self.revealed[usize::from(from)] = true;
        }
        if self.revealed.iter().all(|&v| v) {
            self.phase = Phase::Complete;
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
        Ok(CeremonyResult {
            check_words: words,
            circle_id: id,
            pad: self.pad,
            params: self.params,
        })
        // `self.contribution` is a Zeroizing<Vec> and is wiped on drop here.
    }
}

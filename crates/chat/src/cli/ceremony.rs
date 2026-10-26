//! A development shortcut: the real ceremony state machine, run for every member in one
//! process with OS-CSPRNG contributions. Nothing here is a hardware TRNG, so the circles
//! are labelled `CSPRNG`.

use super::store::{write_label, StoreError};
use cahoots_core::ceremony::Ceremony;
use cahoots_core::params::CircleParams;
use cahoots_core::CircleId;
use cahoots_provider::FileProvider;
use std::path::Path;
use std::string::ToString;
use std::vec::Vec;
use zeroize::Zeroizing;

pub struct LocalCeremony {
    pub circle_id: CircleId,
    pub check_words: [&'static str; 6],
    pub member_dirs: Vec<std::path::PathBuf>,
}

pub fn local_ceremony(
    members: u8,
    pad_len: u32,
    name: &str,
    out: &Path,
) -> Result<LocalCeremony, StoreError> {
    let params = CircleParams::new(members, pad_len, name, false)
        .map_err(|e| StoreError::Format(std::format!("params: {e:?}")))?;
    let mut devs = Vec::new();
    for i in 0..members {
        let mut r = Zeroizing::new(std::vec![0u8; pad_len as usize]);
        getrandom::getrandom(&mut r).map_err(|e| StoreError::Io(e.to_string()))?;
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(seed.as_mut()).map_err(|e| StoreError::Io(e.to_string()))?;
        let mut seat_seed = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(seat_seed.as_mut()).map_err(|e| StoreError::Io(e.to_string()))?;
        devs.push(
            // Tier 0: these members run in software on a laptop, with nothing protecting
            // the pad but the file system.
            Ceremony::new(params.clone(), i, r.to_vec(), *seed, *seat_seed, 0)
                .map_err(|e| StoreError::Format(std::format!("ceremony: {e:?}")))?,
        );
    }
    let commits: Vec<_> = devs
        .iter()
        .map(|d| (d.my_commitment(), *d.my_public_key(), d.my_seat_public()))
        .collect();
    for d in devs.iter_mut() {
        for (j, (c, pk, sp)) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c, pk, &sp.0, 0)
                .map_err(|e| StoreError::Format(std::format!("commit: {e:?}")))?;
        }
    }
    let mut fill = |buf: &mut [u8]| {
        getrandom::getrandom(buf).expect("os randomness");
    };
    let reveals: Vec<Vec<u8>> = devs
        .iter_mut()
        .map(|d| d.my_reveal(&mut fill).expect("reveal phase").to_vec())
        .collect();
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r)
                .map_err(|e| StoreError::Format(std::format!("reveal: {e:?}")))?;
        }
    }
    let attestations: Vec<Vec<u8>> = devs
        .iter()
        .map(|d| d.my_attestation().expect("attest phase"))
        .collect();
    for d in devs.iter_mut() {
        for (j, a) in attestations.iter().enumerate() {
            d.add_attestation(j as u8, a)
                .map_err(|e| StoreError::Format(std::format!("attest: {e:?}")))?;
        }
    }
    let mut member_dirs = Vec::new();
    let mut result = None;
    for (i, d) in devs.into_iter().enumerate() {
        let r = d
            .finish()
            .map_err(|e| StoreError::Format(std::format!("finish: {e:?}")))?;
        let dir = out.join(std::format!("member-{i}"));
        let mut provider =
            FileProvider::open(&dir).map_err(|e| StoreError::Io(std::format!("{e:?}")))?;
        let id = provider
            .add_circle(params.clone(), i as u8, r.pad.to_vec())
            .map_err(|e| StoreError::Io(std::format!("{e:?}")))?;
        write_label(&dir.join(hex::encode(id)), "CSPRNG")?;
        member_dirs.push(dir);
        match &result {
            None => result = Some((r.circle_id, r.check_words)),
            Some((cid, words)) => {
                if *cid != r.circle_id || *words != r.check_words {
                    return Err(StoreError::Format("members disagree".into()));
                }
            }
        }
    }
    let (circle_id, check_words) = result.expect("at least two members");
    Ok(LocalCeremony {
        circle_id,
        check_words,
        member_dirs,
    })
}

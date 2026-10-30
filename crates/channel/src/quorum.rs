//! Quorum approvals (channels spec §5, §6): seats' signed yes verdicts on a request, counted
//! against a rule.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatSigner;
use cahoots_core::CircleId;
use cahoots_objects::Approval;

use crate::ChannelError;

/// Approval profile of a post that needs seat approvals.
pub const PROFILE_POST: &str = "channel-post";
/// Approval profile of a policy or channel object change.
pub const PROFILE_GOVERN: &str = "channel-govern";

/// A seat's yes on `request_digest`, naming the channel's current policy `policy_id`, signed
/// under kid `[seat]`.
#[allow(clippy::too_many_arguments)]
pub fn approval_for<S: SeatSigner>(
    key: S,
    seat: u8,
    circle_id: &CircleId,
    profile: &str,
    fields: Vec<(String, String)>,
    request_digest: [u8; 32],
    policy_id: &[u8],
    log: u64,
    assurance: u8,
    nonce: [u8; 16],
) -> Vec<u8> {
    Approval {
        circle_id: *circle_id,
        seat,
        profile: String::from(profile),
        fields,
        request_digest,
        policy_id: policy_id.to_vec(),
        verdict: true,
        assurance,
        log,
        nonce,
    }
    .sign(key, &[seat])
}

/// Count the distinct seats whose approval says yes to `request_digest` under `profile` at
/// `assurance` or above, and refuse with [`ChannelError::Quorum`] when they number fewer than
/// `quorum`. Every approval must verify against `manifest`: a bad or foreign one fails the whole
/// check. With `quorum == 0` nothing is read and the count is 0.
pub fn check_quorum(
    approvals: &[Vec<u8>],
    manifest: &Manifest,
    request_digest: &[u8; 32],
    profile: &str,
    quorum: u8,
    assurance: u8,
) -> Result<usize, ChannelError> {
    if quorum == 0 {
        return Ok(0);
    }
    let mut seats: Vec<u8> = Vec::new();
    for bytes in approvals {
        let a = Approval::verify(bytes, manifest).map_err(ChannelError::Object)?;
        if a.verdict
            && a.profile == profile
            && a.request_digest == *request_digest
            && a.assurance >= assurance
            && !seats.contains(&a.seat)
        {
            seats.push(a.seat);
        }
    }
    let have = seats.len();
    if have < usize::from(quorum) {
        return Err(ChannelError::Quorum { have, need: quorum });
    }
    Ok(have)
}

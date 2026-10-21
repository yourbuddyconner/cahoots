//! Circle parameters chosen by the ceremony leader (handoff 3.2, 3.4 step 1).

use alloc::string::String;
use alloc::vec::Vec;

/// Protocol version carried in the encoded parameters.
pub const PARAMS_VERSION: u8 = 1;
/// Smallest useful slice: one 32-byte one-time Poly1305 key.
pub const MIN_SLICE_LEN: u32 = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CircleParams {
    /// Number of human members, 2..=8.
    pub n: u8,
    /// Pad length `L` in bytes.
    pub pad_len: u32,
    /// Human-readable circle name, at most 255 bytes of UTF-8.
    pub name: String,
    /// Whether the circle reserves one extra slice for an AI agent seat.
    pub agent_seat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamsError {
    MemberCount,
    NameTooLong,
    PadTooSmall,
    Malformed,
}

impl CircleParams {
    pub fn new(n: u8, pad_len: u32, name: &str, agent_seat: bool) -> Result<Self, ParamsError> {
        let p = CircleParams {
            n,
            pad_len,
            name: String::from(name),
            agent_seat,
        };
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), ParamsError> {
        if !(2..=8).contains(&self.n) {
            return Err(ParamsError::MemberCount);
        }
        if self.name.len() > 255 {
            return Err(ParamsError::NameTooLong);
        }
        if self.pad_len / u32::from(self.slices()) < MIN_SLICE_LEN {
            return Err(ParamsError::PadTooSmall);
        }
        Ok(())
    }

    /// Number of slices: `n` member slices plus one agent slice when seated.
    pub fn slices(&self) -> u8 {
        self.n + u8::from(self.agent_seat)
    }

    /// Index of the agent slice, if the circle seats an agent.
    pub fn agent_slice(&self) -> Option<u8> {
        self.agent_seat.then_some(self.n)
    }

    /// Length of every slice: `floor(L / slices)`.
    pub fn slice_len(&self) -> u32 {
        self.pad_len / u32::from(self.slices())
    }

    /// Start offset of slice `k` within the pad.
    pub fn slice_start(&self, k: u8) -> u32 {
        u32::from(k) * self.slice_len()
    }

    /// Canonical encoding, bound into every commitment and check-word hash:
    /// `version | n | L (u32 LE) | agent_seat | name_len | name`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.name.len());
        out.push(PARAMS_VERSION);
        out.push(self.n);
        out.extend_from_slice(&self.pad_len.to_le_bytes());
        out.push(u8::from(self.agent_seat));
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ParamsError> {
        if bytes.len() < 8 || bytes[0] != PARAMS_VERSION {
            return Err(ParamsError::Malformed);
        }
        let name_len = bytes[7] as usize;
        if bytes.len() != 8 + name_len || bytes[6] > 1 {
            return Err(ParamsError::Malformed);
        }
        let name = core::str::from_utf8(&bytes[8..]).map_err(|_| ParamsError::Malformed)?;
        let pad_len = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        Self::new(bytes[1], pad_len, name, bytes[6] == 1)
    }
}

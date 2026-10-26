use crate::circle::Circle;
use crate::{CircleId, CircleInfo, PadError, PadProvider, PskHandle, PSK_LEN};
use cahoots_core::circle_id::circle_id;
use cahoots_core::ledger::MemLedger;
use cahoots_core::pad::MemPadStorage;
use cahoots_core::params::CircleParams;
use cahoots_custodian::to_psk;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// In-memory provider for tests and the WASM client.
#[derive(Default)]
pub struct MemProvider {
    circles: HashMap<CircleId, Circle<MemPadStorage, MemLedger>>,
}

impl MemProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a freshly combined pad. Returns its circle ID.
    pub fn add_circle(
        &mut self,
        params: CircleParams,
        my_slice: u8,
        pad: Vec<u8>,
    ) -> Result<CircleId, PadError> {
        let id = circle_id(&pad);
        let slices = params.slices();
        let c = Circle::new(
            params,
            my_slice,
            id,
            MemPadStorage::new(pad),
            MemLedger::new(slices),
        )?;
        self.circles.insert(id, c);
        Ok(id)
    }

    pub fn circles(&self) -> Vec<CircleId> {
        self.circles.keys().copied().collect()
    }

    fn get(&mut self, id: &CircleId) -> Result<&mut Circle<MemPadStorage, MemLedger>, PadError> {
        self.circles.get_mut(id).ok_or(PadError::UnknownCircle)
    }
}

impl PadProvider for MemProvider {
    fn circles(&self) -> Vec<CircleInfo> {
        self.circles.values().map(|c| c.info()).collect()
    }

    fn psk(
        &mut self,
        circle: &CircleId,
        _context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError> {
        let (h, b) = self.get(circle)?.spend_own(PSK_LEN)?;
        Ok((h, to_psk(b)))
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        _context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        Ok(to_psk(
            self.get(&handle.circle_id)?.spend_peer(handle, PSK_LEN)?,
        ))
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        self.get(circle)?.spend_own(len)
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        self.get(&handle.circle_id)?.spend_peer(handle, len)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        self.circles
            .get(circle)
            .map(|c| u64::from(c.remaining()))
            .unwrap_or(0)
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        self.circles
            .get(circle)
            .map(|c| c.low_pad())
            .unwrap_or(true)
    }
}

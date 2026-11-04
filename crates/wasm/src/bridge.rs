//! The bridge in the browser: a badge on a USB cable joins a ceremony the browser hosts
//! through the relay. The state machine is [`cahoots_hal::bridge::Bridge`]; this wraps it
//! for JavaScript, which owns the serial port and the relay. Bytes read from the port go in
//! with `cable_in`, bytes to write come out with `cable_out`, every relay entry the host
//! reads is handed over with `deliver`, and what the badge posts is taken with `take_posts`.
//! Each badge's join code comes out of `take_join_codes`, for the host to take as a scan.

use crate::{b64_encode, offer_err};
use cahoots_core::channel::Channel;
use cahoots_core::frame;
use cahoots_hal::bridge::{Bridge, BridgeEnv, BridgeError, BridgeStep, CryptoRngCore};
use cahoots_hal::pile::KIND_PING;
use cahoots_hal::Link;

/// How often the browser pings the badge, as the laptop does: a badge that hears nothing
/// for two seconds takes the host for gone and drops what it sends down.
const PING_EVERY_MS: f64 = 500.0;
use rand_core::OsRng;
use wasm_bindgen::prelude::*;

/// The browser's random source and clock.
struct Env {
    rng: OsRng,
}

impl BridgeEnv for Env {
    fn rng(&mut self) -> &mut dyn CryptoRngCore {
        &mut self.rng
    }
    fn now_ms(&mut self) -> u64 {
        js_sys::Date::now() as u64
    }
}

/// The serial port as the bridge sees it: what JavaScript read, and what it should write.
struct Cable {
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    present: bool,
}

impl Link for Cable {
    fn send(&mut self, frame: &[u8]) {
        self.outbox.extend_from_slice(frame);
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.inbox.len());
        buf[..n].copy_from_slice(&self.inbox[..n]);
        self.inbox.drain(..n);
        n
    }
    fn peer_present(&self) -> bool {
        self.present
    }
}

fn bridge_err(e: BridgeError) -> String {
    match e {
        BridgeError::Count { pile, offer } => format!("count:{pile}:{offer}"),
        BridgeError::Params { seat } => format!("params:{seat}"),
        BridgeError::Aborted { seat, reason } => format!("aborted:{seat}:{reason}"),
        BridgeError::Link => "unplugged".into(),
    }
}

#[wasm_bindgen]
pub struct WasmBridge {
    bridge: Bridge,
    cable: Cable,
    codes: Vec<String>,
    last_ping: f64,
    ping_seq: u32,
}

#[wasm_bindgen]
impl WasmBridge {
    /// A bridge for the host's `offer` text with `badges` cards on the cable. The offer says
    /// how many cards it was made for: `count:<pile>:<offer>` when that is not `badges`.
    #[wasm_bindgen(constructor)]
    pub fn new(offer: &str, badges: u8) -> Result<WasmBridge, String> {
        let o = Channel::from_offer(offer).map_err(offer_err)?;
        let bridge = Bridge::new(o, badges, Box::new(Env { rng: OsRng })).map_err(bridge_err)?;
        Ok(WasmBridge {
            bridge,
            cable: Cable { inbox: Vec::new(), outbox: Vec::new(), present: true },
            codes: Vec::new(),
            last_ping: 0.0,
            ping_seq: 0,
        })
    }

    /// Bytes read from the port.
    pub fn cable_in(&mut self, bytes: &[u8]) {
        self.cable.inbox.extend_from_slice(bytes);
    }

    /// Bytes to write to the port, taken once.
    pub fn cable_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.cable.outbox)
    }

    /// The port closed or the badge went away.
    pub fn unplug(&mut self) {
        self.cable.present = false;
    }

    /// One relay entry for the offer's channel, as the host read it.
    pub fn deliver(&mut self, wire: &[u8]) {
        self.bridge.deliver(vec![(0, wire.to_vec())]);
    }

    /// The badge's messages to post to the relay, base64, in order, taken once.
    pub fn take_posts(&mut self) -> Vec<String> {
        self.bridge.take_posts().iter().map(|w| b64_encode(w)).collect()
    }

    /// Each badge's join code as its commitment came down the cable, taken once.
    pub fn take_join_codes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.codes)
    }

    /// How many seat 1 setups on the relay were not the host's.
    pub fn strays(&self) -> usize {
        self.bridge.strays()
    }

    /// One pass: `"working"` or `"done"`, the latter once every seat's signature has gone
    /// where it was going. Errors: `count:<pile>:<offer>` for a pile of the wrong size,
    /// `params:<seat>` for a card that proposed other parameters, `aborted:<seat>:<reason>`,
    /// and `unplugged`.
    pub fn step(&mut self) -> Result<String, String> {
        let now = js_sys::Date::now();
        if now - self.last_ping >= PING_EVERY_MS {
            self.last_ping = now;
            let ping = frame::encode(&frame::Frame { kind: KIND_PING, seq: self.ping_seq, payload: &[] });
            self.ping_seq = self.ping_seq.wrapping_add(1);
            self.cable.send(&ping);
        }
        let codes = &mut self.codes;
        let step = self
            .bridge
            .step(&mut self.cable, &mut |_, code| codes.push(code.to_string()))
            .map_err(bridge_err)?;
        Ok(match step {
            BridgeStep::Working => "working",
            BridgeStep::Done => "done",
        }
        .into())
    }
}

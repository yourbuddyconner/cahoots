# Custodian architecture: design

*Later work: what a badge does with its seat keys, and the `Chip` family of requests, is in the [chip apps design](2026-10-28-chip-apps-design.md). The wire messages are in [protocol](../../protocol.md) section 6.*

*Written 2026-10-25 from a discussion with Conner. Decisions he made: software and the V1 board edits proceed in parallel; a stored pad is not portable between custodians (only the contract and the ledger semantics are shared); animated QR stays RaptorQ; the display contract stays `show_words`, `show_qr`, `show_status`.*

## The idea

Everything in Cahoots is one of two roles. A **custodian** holds a pad and can spend from it without ever reusing a byte. A **consumer** asks a custodian for bytes and turns them into something: a WireGuard key, a file key, a chat epoch, hello words. The ceremony is how custodians come to hold the same pad. The device tiers are how strongly a custodian keeps the one rule. The feed is how consumers' output travels.

A phone, a laptop with the file provider, the Nano, V1, V2 and a hosted enclave seat are all custodians at different tiers. Today they expose four different APIs for that one role: the HAL's `PadStore`, the provider's `PadProvider`, the wasm bindings, and the hosted seat to come. This design gives the role one name, one contract and one wire protocol, and puts the badge's behaviour in one crate that every board runs.

## What stays

- `cahoots-core`: the protocol. Unchanged.
- `cahoots-hal`: the four traits (`Entropy`, `PadStore`, `Display`, `Link`), the ring driver, the pile, the health tests, the ledger. Unchanged.
- The pile's frame format and kinds. Unchanged.
- The app's RaptorQ animated QR. Unchanged.

## New crate: `cahoots-custodian` (`crates/custodian`)

`no_std` + `alloc`. Three things.

**1. The contract.** The trait that is `cahoots-provider`'s `PadProvider` today, moved here and renamed `Custodian`, with one method added:

```rust
pub trait Custodian {
    fn circles(&self) -> Vec<CircleInfo>;
    fn psk(&mut self, circle: &CircleId, context: &[u8]) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError>;
    fn psk_for(&mut self, handle: &PskHandle, context: &[u8]) -> Result<Zeroizing<[u8; 32]>, PadError>;
    fn spend(&mut self, circle: &CircleId, len: u32) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError>;
    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError>;
    fn remaining(&self, circle: &CircleId) -> u64;
    fn low_pad(&self, circle: &CircleId) -> bool;
}

pub struct CircleInfo { pub circle_id: CircleId, pub seat: u8, pub n: u8, pub name: String, pub tier: u8 }
```

`PskHandle`, `PadError`, `PSK_LEN` and `LOW_PAD_PERCENT` move with it. The blanket `impl Custodian for cahoots_core::pad::Pad<S, L>` moves with it, so firmware gets a custodian for free from `StorePad` plus `PersistentLedger`. `cahoots-provider` re-exports everything under its old names (`pub use cahoots_custodian::Custodian as PadProvider;`) and keeps `MemProvider` and `FileProvider`. The invariant every implementation must keep, stated in the crate docs: a range is recorded spent and erased before its bytes are returned, and no range is ever returned twice.

**2. The wire protocol.** CBOR (RFC 8949) via `minicbor`, array-encoded, so a request is a small fixed message on any transport.

| Request | Fields | Response |
|---|---|---|
| `Identify` | | `Identity { circles: Vec<CircleInfo>, tier: u8, name: String }` |
| `Psk` | `circle, context` | `Psk { handle, key: [u8; 32] }` |
| `PskFor` | `handle, context` | `Key { key: [u8; 32] }` |
| `Spend` | `circle, len` | `Bytes { handle, bytes }` |
| `SpendFor` | `handle, len` | `Bytes { handle, bytes }` |
| `Remaining` | `circle` | `Remaining { bytes: u64, low: bool }` |

Every response can instead be `Error { code }` with codes `UnknownCircle`, `Exhausted`, `AlreadySpent`, `OutOfRange`, `Config`, `Storage`, `Malformed`, `Denied`, `Busy`. `serve(custodian, consent, request_bytes) -> response_bytes` is the one function a transport calls. `Consent` is a trait with `fn allow(&mut self, req: &Request) -> bool`; a badge implements it with a screen line and the B button, a laptop provider with `AlwaysAllow`.

Transports, in order of arrival:
- Touch links and the laptop's serial link: `cahoots_core::frame::Frame` with kinds `KIND_REQUEST = 40` and `KIND_RESPONSE = 41`. The pile layer already ignores kinds it does not own.
- The screen: a response is bytes, and bytes can be shown with `Display::show_qr`. One-way, by design.
- USB HID with 64-byte reports and CTAP-style init and continuation packets. A later plan, with the Pico 2 port.

**3. Nothing about storage.** Each custodian keeps its own on-flash or on-disk layout. Only the contract and the ledger semantics are shared.

## New crate: `cahoots-badge` (`crates/badge`)

`no_std` + `alloc`. The badge's behaviour, written once against a `Board` trait, so a board crate is drivers plus a pin table.

```rust
pub trait Board {
    type Entropy: Entropy; type Pad: PadStore; type Records: RecordStore;
    type Display: Display; type Link: Link;
    fn entropy(&mut self) -> &mut Self::Entropy;
    fn display(&mut self) -> &mut Self::Display;
    fn a_pressed(&mut self) -> bool;   // edge, consumed
    fn b_pressed(&mut self) -> bool;
    fn a_down(&self) -> bool;          // level
    fn delay_ms(&mut self, ms: u32);
    fn links(&mut self) -> (Self::Link, Self::Link);          // (down, up)
    fn load_circle(&self) -> Option<Circle>;
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError>;
    fn wipe(&mut self, pad_len: u32) -> Result<(), BoardError>;
    fn write_pad(&mut self, pad: &[u8]) -> Result<Self::Pad, BoardError>;
    fn open_pad(&mut self, len: u32) -> Self::Pad;
    fn records(&mut self, fresh: bool) -> Result<Self::Records, BoardError>;
    fn proposal(&self) -> (String, u32);   // (name, pad_len) if this card is the bottom of the pile
    fn tier(&self) -> u8;
    fn selftest(&mut self) -> Vec<String>; // board-specific lines; the badge adds the common ones
    fn log(&mut self, line: &str);
}

pub struct Circle { pub params: CircleParams, pub seat: u8, pub circle_id: CircleId }
```

`Badge<B: Board>` has `boot()`, which runs the self-test screen, forgets the circle if A is held, and shows the words of a stored circle; `run_once()`, which finds the pile, runs the ceremony, and keeps or discards; and, once a circle is stored, an idle loop with a three-item menu cycled by A and chosen by B: **words** (the six words), **key as code** (spend 32 bytes, show them with `show_qr`, the one-way transport), and **serve** (answer custodian requests arriving on the down link, each one shown on the status line and allowed by B or refused by A). The custodian it serves is `Pad<StorePad<B::Pad>, PersistentLedger<B::Records>>`.

The ESP32-S3 crate becomes a `Board` implementation plus a `main` of about sixty lines. The host simulator gains a `HostBoard` and a `cahoots-host badge` command, so the simulator runs the exact code the hardware runs. The Pico 2 crate, when it comes, is a third `Board`.

## Tier floor (later plan)

The leader names a minimum tier in the circle parameters; a device below it refuses to join; the words screen shows the circle's tier. This changes the canonical parameter encoding, so it is a protocol revision with new vectors and app changes, and it gets its own plan.

## V1 board edits (`hardware/v1`)

The harness exposes the maker world's standard bus and the Pico's spare pins, so peripherals plug in without soldering and firmware forks have something to drive:

- **J5, a Qwiic / STEMMA QT socket.** The same JST-SH 4-pin horizontal footprint as the pile ports, Qwiic pin order (1 GND, 2 3V3, 3 SDA, 4 SCL), on the front at the bottom edge beside the pile ports, SDA on GP8 and SCL on GP9 (I2C0).
- **J6, a 2x5 2.54 mm header** on the back near the bottom edge, carrying 3V3, GND and eight spare GPIOs: GP6, GP7, GP10, GP11, GP12, GP13, GP22, GP26. Nothing else on the board may use those pins.
- **The pogo pair carries no power.** Pin 3 stays spare. Feeding one badge's regulator from another's is a decision for V2, if ever; two UART lines and ground is the pile.

The DIY Pico 2 build gets the same I2C pins in its table, so a breadboard build and V1 agree.

## Later plans, in order

1. Pico 2 board crate with the USB HID transport and the `cahoots-host` serial fallback.
2. A `BadgeProvider` backend in `cahoots-provider` over USB HID or serial, so WireGuard, age and chat get bytes from a badge unchanged, and WebHID in the app.
3. The tier floor.
4. Xous `Board` for V2, developed in hosted mode.

## Out of scope

Dynamic loading of apps on bare-metal boards, Tock, a shared pad file format, UR encoding, the hosted seat's policy layer.

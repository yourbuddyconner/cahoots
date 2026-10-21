# Project plan

Milestones from the handoff of 2026-10-20, plus a network-bootstrap track added on 2026-10-20 after the WireGuard and mesh discussion (`docs/use-cases.md`). Status reflects the `main` branch.

## Hardware track

| Milestone | Scope | Done when | Status |
|---|---|---|---|
| M0 Repo | Workspace, CI (fmt, clippy, test, no_std, wasm, vectors) | CI green on the layout | **done** |
| M1 cahoots-core | Sections 3.1 to 3.6 as pure logic | Unit, property, and vector tests pass; builds no_std | **done** |
| M2 Phone and laptop client + YubiKey | WASM core, offline PWA, animated-QR ceremony, compose/read, TRNG/CSPRNG label, `cahoots-yk` | Three clients run a ceremony with no network; YubiKey contributions get the TRNG label; throughput measured | code done; camera scan and YubiKey throughput unverified |
| M3 TVC app | Agent seat, policy, square log, vouching on TVC2 simulation | Acceptance scenario (handoff section 6) passes | not started |
| M4a Host simulator | `cahoots-hal` driver plus socket Link, file PadStore, PNG Display | Four simulated devices complete a ring ceremony; words match the vectors | **done** |
| M4b ESP32-S3 bring-up | TRNG with entropy source and health tests, NOR pad partition, e-paper, UART touch link, USB CDC for YubiKey bytes | Four boards complete a ceremony over wires in under 30 s at 256 KiB | not started, boards to order |
| M4c Xous hosted | `dabao` target in `bao1x-emu` | Passes the device suite against the host simulator | not started |
| M5 Dabao port | Driver swap, RRAM erase verification | Four Dabaos, and a mixed ring with ESP32-S3s, complete a ceremony | December 2026 |
| M6 Agent seat on hardware | A device sponsors the agent seat and approves from a device | A device-approved agent message posts and decrypts everywhere | after M3, M5 |
| M7 Meetup kit | Ceremony guide, square display, batteries, enclosures, pogo connectors | Conner runs a meetup with devices and phones in one circle | after M4b |

## SDK track

| Milestone | Scope | Done when | Status |
|---|---|---|---|
| S1 Provider, HPKE, age | `cahoots-provider` with memory and file backends, HPKE `mode_psk`, `age-plugin-cahoots` | Two laptops encrypt a 1 GB file with `age -r cahoots:<circle>`; replayed offset refused; PSK vectors published | code done; the `age` CLI end-to-end run is pending |
| S2 Circle-MAC approval demo | TVC-hosted verifier a Turnkey policy can require | Forged or replayed approval refused | after M3 |
| S3 Recovery circles | Shamir split at a device ceremony | 3-of-5 recovers; two shares reveal nothing | after M5 |
| S4 Transport adapters | TLS 1.3 PSK importer, MLS external PSK, WireGuard helper | Interop tests against rustls or OpenSSL, OpenMLS, `wg` | WireGuard helper done under S1; `wg` interop test pending |
| S5 TVC pad custody | TVC backend for the provider | Workload gets a PSK only under the pinned policy; every spend logged | after M3 |

## Network-bootstrap track (new)

The design is in `docs/use-cases.md` sections 3 to 5. These milestones turn a circle into a trust root for a network.

| Milestone | Scope | Done when | Depends on |
|---|---|---|---|
| N1 WireGuard fleet bootstrap | `cahoots-wg` CLI: `peer` announcements, `psk` issue and accept, `[Peer]` rendering, `wg set` command, rotation schedule | Two Linux boxes bring up a WireGuard tunnel whose public keys and PSK came only from a circle. Rotation over the tunnel works. A replayed PSK handle is refused on both ends. `wg show` reports the preshared key present. | S1 |
| N2 Control message subtypes | `peer`, `psk`, `rotate`, `slice-transfer` encodings in `cahoots-core`, with vectors | Vectors published; the PWA shows peer announcements in a circle | M1 |
| N3 Headless node provisioning | Badge-to-node slice transfer over USB or the touch link, with erase verification on the badge | A simulated host node receives a pad from a simulated badge; the badge's copy reads back erased | M4a, then M4b |
| N4 Mesh adapters | Meshtastic channel key, 802.11s SAE password, Reticulum link PSK helpers | Each helper has an interop test against the real stack where one is installable; otherwise a documented manual test | N1 |
| N5 Federation profile | Pairwise pads from pair touches at the ceremony; sender-authentic messages; per-pair tunnel PSKs | Three devices complete group plus pairwise ceremonies; a member cannot forge another's announcement | M4b |
| N6 Numbers Station over LoRa | A badge with a LoRa radio broadcasts pure-pad messages with fountain framing; the radio is provably off during contribution | A message sent over LoRa decrypts on a phone client; a capture shows ciphertext only | M4b, M7 |

Suggested order: N1 and N2 alongside M2, because they need no hardware and give the first "I bootstrapped a real network from a ceremony" story. N3 follows M4a. N5 and N6 follow the ESP32-S3 bring-up.

## Circle-apps track (new)

Design in `docs/use-cases.md` section 8.

| Milestone | Scope | Done when | Depends on |
|---|---|---|---|
| A1 Property demos | `words` (spoken challenge), fair draw, SSH bootstrap helper, and the transcript-forging tool for the deniability claim | Each has a test and a one-paragraph doc with its security claim and limits | M2, N2 |
| A2 App distribution | `app` control subtype carrying a module hash; clients refuse unposted modules | A posted hash enables a module in every member's client; an unposted one is refused | N2 |
| A3 Reducer runtime | WASM reducer host in `cahoots-wasm` and the PWA, with send quotas, mandatory bucket padding, and a visibly separate app frame | The vote app runs in three clients and agrees on the tally from the square's order | A2 |
| A4 Protocol v2 items | Anonymous region with a retry rule for collisions; hidden circle ID in the header | Vectors published; v1 and v2 messages coexist by version byte | A3 |

## Chat track (new, 2026-10-20)

Design in `docs/chat.md`: epoch keys from 32-byte pad spends, per-sender symmetric ratchets, ChaCha20-Poly1305, relayed over the Numbers Station.

| Milestone | Scope | Done when | Depends on |
|---|---|---|---|
| C1 Chat crate | `cahoots-chat`: epoch derivation, chains, message format, session state, vectors | Three sessions round-trip with rotation, out-of-order delivery, tamper and replay rejection; builds for wasm32 and thumbv7em | **done** |
| C2 Numbers Station | `cahoots-station`: hash-chained append-only log, pure-pad reuse refusal, long-poll feed, display page, Dockerfile | Integration test posts and reads through HTTP; a broken chain refuses to start | **done** |
| C3 CLI demo | `cahoots-chat` binary: local dev ceremony, REPL client against the relay | Three terminals chat through one relay; an epoch rotates; a tampered entry is rejected | **done** |
| C4 PWA chat | Chat screen in `web/app` over the wasm bindings, relay URL setting, session persisted in IndexedDB | Two browser tabs chat through the relay; the UI says "end-to-end encrypted, pad-hedged" | **done** (verified in Chrome) |
| C5 TVC deploy | The station as a TVC2 workload with its head anchored | The deploy record pins the image and the log head is anchored | C2, access to `tkhq/test-tvc-2` |

## Open decisions

Carried from the handoff section 7, with additions.

1. ~~Repo location.~~ Decided: `github.com/yourbuddyconner/cahoots`.
2. Agent runtime: a Valet agent calling the app, or a model inside TVC.
3. Square exposure at meetups: public internet or local network only.
4. Screen: e-ink or OLED, decided after comparing both in M5.
5. When marketing gets involved.
6. SDK priority: S2 or S3 as the first customer pitch.
7. Who reviews S2, S3, and S5.
8. Whether to buy an open-design USB TRNG as an auditable second source.
9. Whether to fuse secure boot and flash encryption on the spare ESP32-S3. Needs Conner's approval.
10. **New.** Whether the first network demo is WireGuard between laptops (N1, no hardware) or a Meshtastic circle (N4, needs radios). WireGuard is faster to ship. Meshtastic is closer to the community angle.
11. **New.** Whether the federation profile (N5) is worth its ceremony cost before there is a multi-party deployment that needs it.
12. **New.** Whether circle apps (A2, A3) come before or after the TVC app (M3). They share the control-subtype work in N2.
13. **New.** Whether the genesis manifest (regions, holders, purposes, spend rules) replaces slices now, before C4 lands, or after. It touches core, vectors, provider, wasm, and host. Decided in conversation on 2026-10-20 to adopt the manifest and drop delegation; timing open.

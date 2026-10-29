# The chip in the stack: seat keys in silicon, attestation, SSH, cards, circles

*Written 2026-10-28 from a conversation with Conner, after the SE051 Click board answered on the Pico 2's I2C bus. This is the design for M4f's first half: the NXP SE051 as the place a badge's keys live, and the first things a laptop can ask a badge to do with them. The pad key in the chip, the ledger counter in the chip and bus encryption are the second half and are named as later work. The phone app that uses the card over NFC has its own design.*

## The idea

A circle of people who met in a room already leaves each of them with a seat key, bound into a signed manifest, and a pad. This design moves the seat key into the chip and makes it the one identity a person has in that circle, for every protocol. The seat key is P-256; SSH, git signing, TLS client certificates and the circle's own signed objects all speak P-256; so there is no "SSH key", there is the seat key used for SSH. The manifest is the directory of a circle: `cahoots-host roster --ssh` prints an `authorized_keys` file, and "who may log into this box" means "this circle". Attesting a seat key, which the chip does with NXP's provisioned credentials, is what proves a seat is hardware.

Where a protocol wants a shared secret rather than a signature, the pad is the answer as today. For the group chat, the ceremony also gives every member's card the same circle root, a secret no one outside the room can derive; it lives in the chip as a key that is usable but never readable, and chat keys and pair keys are computed from it.

The first version, in build order, each step usable on its own:

1. **Seat keys in the chip.** The ceremony generates the seat key inside the chip and signs through it. Badges without a chip keep software keys.
2. **Attestation.** One command proves a seat key lives in an SE051, verified against NXP's chain; the tiers table gains the rung.
3. **SSH and git from the seat key.** An agent, a git signing helper, the roster command.
4. **Contact cards.** A seat-signed, pad-sealed profile, carried over the station or a cable.
5. **Many circles on one badge.** A list of records, a slot per circle in the chip, a chooser in the menu.
6. **Grants as standard tokens.** The CWT payload and a laptop-side verifier.

## Goals

- A person with a V1 badge (or today's bench rig: a Pico 2 and the Click board) logs into a server and signs a git commit with a key that exists only in the chip, after pressing both buttons on a screen that names what is being signed.
- Anyone holding a seat's public key and NXP's root certificates can verify the key was generated in an SE051, with no Cahoots server involved.
- A new member of a circle runs one command and has every other member's contact card: name, handles, endpoints for the apps.
- One badge holds several circles and nothing links them.
- A grant signed by a badge is a standard token a token endpoint or a policy engine can consume without reading Cahoots source.
- Every chip operation a badge offers is reachable through one request family on the custodian protocol, so new apps are new request kinds.
- The whole path is testable on the laptop without hardware.

## Non-goals, for this version

- Bitcoin, Ethereum, secp256k1 and Ed25519 wallets. The chip supports both curves; a wallet app comes after SSH proves the path.
- Threshold or MPC signing across a circle. Conner wants it on the device; it is its own design.
- A certificate authority run by the circle.
- The pad key in the chip, the ledger counter in the chip, encrypted spends, bus encryption (SCP03). Second half of M4f.
- The phone app and the chip's NFC side. Nothing here enables the contactless interface; the app's design decides what the chip exposes over it, with the objects this design creates (the circle root, the seat key) as its material.
- Circle widening (adding a member to an existing circle). A later design; the same-person proof in the gatherings notes is its starting point.
- A PIN or a duress PIN. Consent is the two buttons, as today.
- USB HID. The transport is the serial link the host already uses.

## Anti-goals

- **Blind signing.** The badge never shows a hash and asks for consent. Each screen shows facts the badge computed or verified, and marks what it could not verify as the laptop's claim with "(laptop)". A request whose content the badge cannot describe is refused.
- **Our own token format.** Grants are CBOR Web Tokens with standard claim names.
- **Keys that come out.** No request returns a private key. The sealed-key requests from the custodian spec are a separate family and unchanged.
- **A dependency that taints the firmware's license.** The repo is MIT OR Apache-2.0; see the driver decision.
- **Two circles that can be linked through a badge.** Seat keys, roots and records of different circles share nothing derivable.

## What exists and is reused

- `crates/custodian`: the `Custodian` trait, CBOR `Request`/`Response` on the link, `serve` with a `Consent` hook, frame kinds 40 and 41. The badge's idle menu has a Serve page that answers requests on the down link with both-button consent; `crates/host` has tests that drive it.
- `crates/objects`: `Grant`, `Approval`, `Fact`, `Receipt` as COSE_Sign1 over canonical CBOR, signed by a seat's P-256 key; grant chains that only narrow; `verify_chain` against a manifest.
- `crates/core`: seat keys (P-256 ECDSA, `SeatKey`), the manifest, COSE_Sign1, `Purpose`, pair keys from the X-Wing reveal secrets, the ceremony.
- `crates/badge`: the step machine, screens, the menu, `Board` trait; `crates/rp2350`: the Pico 2 port with I2C0 on V1's pins (SDA GP8, SCL GP9, enable GP2) and a boot probe in `src/se.rs` that got an R-block back from the chip.
- `crates/chat`, the relay client in `crates/relay`, and the Numbers Station as the relay: envelopes addressed by a circle topic.
- Chip facts, from the SE051 data sheet rev 2.0: I2C address 0x48, T=1 over I2C, the IoT applet with EC keys on NIST, Brainpool, Koblitz (secp256k1) and Edwards (Ed25519) curves, RSA, AES, HMAC keys, ECDH, digests, counters, attestation with trust-provisioned root credentials, SCP03, two concurrent sessions, policies per object, objects exportable sealed under a chip key. 45 kB of secure flash on the C2: a few hundred bytes per key, so the circle count is a product limit, not a chip limit. Custom applets exist only on the SE051P variant; ours is the C2, so the IoT applet's command set is the whole surface.

## The design

### 1. The chip driver and the `chip` crate

**Decision: build on Nitrokey's `se05x` crate (0.4.2) behind a trait of our own.** It is `no_std`, implements T=1 over I2C and the applet's APDUs from a generated command table, and has what this design needs: `write_ec_key`/key generation on `EcCurve` constants including `NIST_P256`, `ecdsa_sign`, `eddsa_sign`, `read_attest_object`, `read_attributes_attest`, `write_symm_key`, `mac_one_shot_generate` (HMAC), `write_counter`, `read_id_list`, `delete_secure_object`, `export_object`/`import_object`, `get_version`, `get_random`, sessions, and `scp_initialize_update`/`scp_external_authenticate` for the second half. With embedded-hal 1.0, our HAL, no HAL-specific feature is needed.

**The license.** The crate is LGPL-3.0-only; the firmware is one static binary under MIT OR Apache-2.0. The LGPL permits that when a user can relink against a modified library, which for a Rust static binary means the build is reproducible from public sources; it is, the repo is public and `cargo build` reproduces the binary. The crate is used only from the firmware crate, behind a trait, so it can be replaced without moving anything above it. Conner raised no objection when this was put to him and asked for an uninterrupted run, so this is the decision: the crate, with the trait boundary and a note in the firmware README. Writing T=1, the APDUs and SCP03 ourselves is the fallback, about two weeks.

**New crate `crates/chip`** (`cahoots-chip`, `no_std` + `alloc`): the trait and two backends.

```rust
pub enum Curve { P256, Ed25519 }
pub struct KeyId(pub u32);
pub trait ChipOps {
    fn version(&mut self) -> Result<Version, ChipError>;
    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError>;
    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, ChipError>;   // public key, SEC1 compressed for P256
    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError>;
    fn delete(&mut self, id: KeyId) -> Result<(), ChipError>;
    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError>;
    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError>;
    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError>;
    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError>;
    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), ChipError>;  // usable, never readable
    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError>;
    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), ChipError>;
    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError>;
}
```

`Attested` holds the public key, the chip's unique id, the freshness echoed, the chip's monotonic timestamp, the attestation signature, the attestation key's public key and its certificate (DER) read from the chip's provisioned binary object. `Version` is the applet version triple. `ChipError` is `NoChip`, `Transport`, `Status(u16)`, `NoSuchKey`, `Busy`.

Backends: `Se05x<I2C, Delay>` wrapping the Nitrokey driver (firmware only, feature `se05x`); `SoftChip` in software with `p256`, `ed25519-dalek`, `hmac`, keeping objects in a `BTreeMap` and faking attestation with a test CA whose certificate is in the crate, so every app is tested in `cargo test`.

**Key ids.** 32-bit object ids from a Cahoots range above NXP's reserved ids: `0x4C41_0000 | kind << 8 | slot`. `kind` is `KeyKind`: `Seat = 1`, `Root = 2` (the circle root, an HMAC key), `Epoch = 3` (the circle's chat epoch counter), `Attest = 4` (NXP's provisioned attestation key is used; the id is theirs). `slot` is the circle's index in the badge's chip record, 0 to 255. The badge keeps a **chip record** in flash of its own, separate from any circle record: for each slot, the circle id and which objects were made. Forgetting a circle deletes its slot's objects; a slot present in the chip but not in the record (a wiped record, a reflashed badge) is deleted on boot and logged, never reused silently. Holding A at boot wipes records and chip objects alike.

### 2. Seat keys in the chip

`cahoots_core::SeatKey` stays as the software key. A new trait in core:

```rust
pub trait SeatSigner {
    fn public(&self) -> SeatPublic;
    fn sign(&mut self, msg: &[u8]) -> Signature;   // P-256 ECDSA over SHA-256(msg), as SeatKey::sign today
}
```

`SeatKey` implements it. The badge crate gets `ChipSeat { chip, id }` implementing it through `ChipOps::sign_p256` with the digest computed on the badge. Everywhere the ceremony, the manifest attestation phase and the objects crate take a `&SeatKey`, they take a `&mut dyn SeatSigner` (or generic). The ceremony's commit phase, which binds the seat public key into the commitment, asks the signer for `public()` first, so a chip key is generated at that moment: the badge shows "making this circle's key in the chip" for the second it takes.

**The circle root.** After the reveals, every member can compute `root = HKDF(SHA-256, all X-Wing reveal secrets in seat order, "cahoots/root/v1")`, the same value on every card. It is written into the chip as the slot's HMAC key and never read again; the software badge keeps it in the circle record. Pair keys for hello words are derived as today from the pairwise secrets, unchanged. Chat keys are `HMAC(root, "chat" || circle_id || epoch)` on demand (section 7 and the app design). The root's purpose in this design is only to exist and be testable; the app design says how a phone gets things derived from it.

**Backwards compatibility.** Records written by today's firmware (`CAH2`) carry a software seat key; they load as before and keep signing in software. A badge with a chip uses the chip for new circles only.

### 3. The custodian protocol: one new request family

New variants on `Request` and `Response` in `crates/custodian`, same CBOR array encoding, new indices, existing ones unchanged:

- `Request::Chip { circle: CircleId, op: ChipRequest }` and `Response::Chip { result: ChipResponse }`. Every chip request names a circle, because every key is a seat in a circle.
- `ChipRequest`:
  - `Describe`: the seat's public key, curve, whether it is in a chip, whether it can attest. No consent.
  - `Attest { freshness: [u8; 16] }`: the attested bundle for the seat key. No consent: public material only.
  - `SshSign { data: Vec<u8>, host: Option<SshHost>, content: Option<Vec<u8>> }`. `data` is the exact bytes OpenSSH asks an agent to sign. For a login it is the userauth request: session id, user name, service, the client's key; it carries no host key. `SshHost` is what the laptop learned from OpenSSH's `session-bind@openssh.com` message: the server's host key, the session id, the server's signature over that session id, and the host name the laptop resolved for that key from `known_hosts`. The badge checks the session id in `data` against the bound one and verifies the server's signature when the host key is Ed25519 or P-256 (both verify in firmware with crates already present); the screen says "ssh login as `user`" (verified) and "to `host name` (laptop)". An RSA host key or no binding shows "host not named". For git, `data` is the `SSHSIG` envelope: a namespace and a SHA-512 of the content, unreadable by a person; so the laptop also sends `content`, the commit text git handed it, when it fits the request limit; the badge hashes it, matches `data`, and shows the author and subject lines as verified. Content too large: the subject the laptop sent, marked "(laptop)". Consent required.
  - `Card { card: Vec<u8> }`: sign a contact card (section 6). Consent required.
  - `GrantSign { grant: Vec<u8> }`: sign a CWT grant payload (section 8). Consent required.
- `ChipResponse`: `Description(..)`, `Attested(Attested)`, `Signature(Vec<u8>)` (SSH wire format for `SshSign`; COSE_Sign1 bytes for `Card` and `GrantSign`). Errors fold into `Response::Error` with three new codes: `NoChip`, `ChipError`, `Unparseable`.
- `Response::Identity` gains `chip: bool`. The decoder re-encodes and rejects anything that differs, so an older laptop program will call a new badge's answer malformed. Everything ships from one repo and nothing else speaks the protocol yet; the break is accepted and the custodian README says both sides must be from the same release. A protocol version in `Identify` is the fix if that changes.
- `MAX_REQUEST_LEN` rises to 16 KiB for `SshSign` with content; other requests keep 4 KiB.
- `serve` asks `Consent` for every signing request; `Consent::allow` keeps its signature and the badge's `Answer` decides from the screen it showed.

A badge without a chip answers `Describe` and `Attest` honestly (software key, cannot attest) and signs the rest with its software seat key, so the whole family works on the host's simulated badge; only attestation is hardware-only.

### 4. SSH on the laptop: `cahoots-host agent`, `git-sign`, `roster`

- `cahoots-host agent --serial <port> --circle <name>` runs an SSH agent on a Unix socket (OpenSSH `PROTOCOL.agent`): identities are the circle's seat key as `ecdsa-sha2-nistp256` with comment `cahoots:<circle>:<seat>`; signature requests are forwarded as `ChipRequest::SshSign`, with `host` filled from the `session-bind@openssh.com` extension and `known_hosts` (plain and hashed entries). `--circle` may repeat; each circle's seat key is one identity. The agent never caches a signature and never signs without the badge; `Denied` becomes `SSH_AGENT_FAILURE`.
- `cahoots-host git-sign` is the `gpg.ssh.program` git calls: it reads the content git passes, builds the request with `content`, and writes the `SSHSIG` signature git expects. The README gives the config lines: `gpg.format ssh`, `user.signingkey` from `ssh-add -L`, `gpg.ssh.program`.
- `cahoots-host roster --ssh [--circle]` prints `authorized_keys` lines for every seat in a manifest the laptop holds (from the badge's `Identify` plus the manifest the app or station delivered), one line per seat, comment `cahoots:<circle>:<seat>`.
- Acceptance: `ssh -T git@github.com` with the badge, and a signed commit GitHub shows as verified.

### 5. Attestation

`ChipRequest::Attest` returns `Attested`. `cahoots-host attest --serial .. --circle ..` prints it as JSON and verifies it: the attestation signature over the attested payload by the certificate's key, the certificate's chain to NXP's published root for the SE051 attestation CA, the freshness echoed. The verifier is `crates/objects/src/attestation.rs` so the app and the station can use it, with NXP's roots vendored as PEM with source URLs and fingerprints in comments. The result feeds `docs/tiers.md`: a seat whose key is attested sits at a new rung "key, attested" between "key" (1) and "present" (2), and the manifest's declared tier can be checked against it after the fact. It proves a key lives in an SE051, not who holds the badge nor that the firmware is ours; the docs say so.

### 6. Contact cards

A card is a seat-signed object in `crates/objects` (`card.rs`): `{ circle_id, seat, name, handles: [(kind, value)], endpoints: [(purpose, value)], log }` as canonical CBOR in a COSE_Sign1 by the seat key. `kind` is a short text (`email`, `github`, `nostr`, `x`); `purpose` reuses `Purpose` (`WireGuard`, `Tls`, ...) plus `Ssh` and `Chat`, with `value` a text the laptop knows how to use (an endpoint, a host, a chat address). Sealed for the circle as `AES-256-GCM` under `HMAC(root, "card" || circle_id)` on the laptop side with the root released... no: the root never leaves the chip, so the card seal key is `HMAC(root, "seal/card")` computed by the chip through a `Request::Chip { op: SealKey { label } }` with consent, cached by the laptop for the session. (The app design reuses this for chat.) Cards travel as station envelopes on the circle's topic, or as files over a cable. `cahoots-host card set/show/fetch` builds, signs, publishes and fetches; `cahoots-host roster --ssh` and the WireGuard commands read endpoints from fetched cards when present. The badge's consent screen for a card shows the name and the count of handles and endpoints, and the badge keeps the signed card so the Card page in the menu can show it with a code (the app design decides the code's content).

### 7. Many circles on one badge

The circle record becomes a list: `records.rs` holds up to 16 entries on the Pico (the region grows to four sectors) and 4 on the Nano, each the record as today plus its chip slot. The menu gets a chooser: the first page after boot lists circles by name, Left/Right moves, Both enters that circle's menu as today, and Serve answers requests for any circle the badge holds, since each request names one. The custodian's `Identity` already lists circles. Forgetting is per circle. Nothing is derived across circles: separate seat keys, separate roots, separate slots, and the badge name is the only thing they share.

### 8. Grants as standard tokens

Grants are signed by the seat key, in the chip when there is one; the existing verifier ties a root grant to the manifest through the seat key, so no new key and no verifier change. The existing `Grant` payload (version 1, with published vectors) stays decodable; a version 2 payload is a CBOR Web Token (RFC 8392) with standard claims: `iss` the circle id as `cahoots:circle:<hex>`, `sub` the grantee's key id, `cnf` (RFC 8747) the grantee's public key so the token is proof-of-possession, `cti` the nonce, `authorization_details` (RFC 9396) a list of `{type, actions, locations, ...}` objects whose `type` values are our purposes under a `cahoots:` namespace, with the pad budget on the `cahoots:pad` type, and two private claims: `cahoots-log` for `not_after` as a log position, and `cahoots-parent` for the parent's hash. The chain rules do not change. The badge has no clock and keeps no log position, so the laptop supplies `not_after` and the screen shows "until log entry N (laptop)". A laptop-side token endpoint (OAuth 2.1, with RFC 8693 token exchange and RFC 9396 for the requested details) and an AuthZEN policy point are the Governed Decisions work and out of scope; the shapes are chosen so they fit.

### 9. Consent screens

In the style of the existing consent screen (title bar, body, footer naming A, A+B and B), A+B allows and A or B refuses:

- **making a key**: "this circle's key", "in the chip" or "in software"; shown during the ceremony's commit step.
- **ssh login**: "ssh login", "as `user`", "to `host` (laptop)" or "host not named", the circle name.
- **git signature**: "sign a git commit", author, subject (verified or "(laptop)"), the circle name.
- **contact card**: "sign your card", name, "N handles, M endpoints".
- **seal key**: "key for `label`", the circle name.
- **grant**: "grant", to whom (laptop), for what (up to three types, then "+N"), "until log entry N (laptop)", "may delegate" or "no delegation".
- **circle chooser**: the list of circle names with the chosen one marked.

Nothing on a consent screen comes from the request unmarked.

### 10. Testing

- `SoftChip` makes every app testable in `cargo test`: ceremony with chip seats, attestation against the test CA, the agent against a simulated badge, `git-sign` against a fixture commit, cards round trip, the chooser's state machine, the CWT grant round trip with `verify_chain`.
- The host's simulated badge gets `--chip soft`.
- Hardware acceptance, on the bench rig: a ceremony with the laptop pile where the Pico's seat is in the chip; `cahoots-host attest` verifies against NXP's root; `ssh -T git@github.com`; a verified commit.
- Vectors in `crates/vectors`: a real attested bundle captured once (public material only), a card, a grant v2, an SSH signing blob with its expected screen lines.

### 11. Firmware

`crates/rp2350` wires the `se05x` backend over I2C0 at 400 kHz, replaces today's boot probe with `ChipOps::version()` in the self-test ("se051: applet x.y.z, N circles"), holds the enable pin high, and grows the record region. Memory: the driver's buffers are a few kilobytes and the chip record is one sector. The 3-second wait for the laptop in the probe goes away.

## Constraints

- No panics on untrusted input: every request parser returns `Unparseable`.
- The chip is optional everywhere: a build without it passes every test, and a badge without it does everything but attest.
- Copy follows the repo's rules: plain words, no coined names, the person's buttons named on every screen, "(laptop)" on every unverified value.
- Docs: the custodian architecture spec points here; `docs/tiers.md` gains the attested rung; `docs/plan.md` M4f splits into this and "pad in the chip"; `crates/host/README.md` documents `agent`, `git-sign`, `roster`, `attest`, `card`; `docs/glossary.md` gains seat key (as identity), circle root, contact card, chip record.

## Left to the builder

- Exact CBOR indices, the `authorization_details` type names, the card's handle kinds.
- The SSH blob parser's coverage: userauth publickey requests and `SSHSIG` version 1 at minimum.
- Which NXP root certificates apply to the C2 variant's attestation chain; the first real bundle will say.
- Whether `se05x`'s default session suffices for `read_attest_object` on a freshly provisioned C2, or a user-id session is needed; the crate's examples use one.
- Screen wrapping of long host names and card names.
- The order of HKDF inputs for the root is seat order; the exact info string is above.

## Decided, and why

- Build on the `se05x` crate behind a trait: weeks saved, replaceable later.
- The seat key is the identity and the SSH key; no separate SSH key: one trust decision, the manifest, serves every protocol.
- Git signing sends the commit text so the screen shows author and subject: the hash alone would be blind signing.
- The badge verifies the SSH server's session binding for Ed25519 and P-256 host keys and marks the host name as the laptop's in every case.
- Grants keep the seat key as signer and gain a CWT payload as version 2.
- The `Identity` change is a protocol break accepted within one repo.
- A grant's expiry stays a log position until the token endpoint exists.
- Many circles per badge now, since the chip's slots and the chip record are designed for it from the start.

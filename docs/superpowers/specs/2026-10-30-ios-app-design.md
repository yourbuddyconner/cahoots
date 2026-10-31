# The iOS app: a seat in your pocket, and the screen for your card

*Written 2026-10-30 from a conversation with Conner. It supersedes the phone-app design of 2026-10-28, which assumed a tap on a card was needed to chat; the handoff of 2026-10-29 dropped that, and the grant work of 2026-10-29 changed how a second device gets keys. This is the design for the first version of a native iOS app. It is a design to plan from, not a plan.*

## The idea

The app does two jobs, and the design says which is which at every point.

**A seat in your pocket.** The phone is a full member of a circle on its own: it runs ceremonies with its camera, holds its seat key in the Secure Enclave, keeps the pad and the chat keys under iOS data protection, chats through the default relay, shows hello words, and issues grants to a laptop by showing a code. A circle of phones is a real circle, labelled for what it is.

**The screen and the network for a card.** When a person also has a badge, the phone pairs with it once, with both buttons on the badge, and from then on a tap on the card is how the phone gets the key for the chat on screen. The chip enforces what a tap may do; the phone cannot ask for more than it was given at pairing. The card never needs a cable, and the phone never holds the pad or the seat key of a card seat.

Native Swift, because everything that makes this different from the browser app is native-only on iOS: Core NFC to the chip, the Secure Enclave for keys, the camera at the frame rate a ceremony needs, and background fetch. The protocol is the Rust crates compiled for the phone and reached through UniFFI; Swift holds views, the NFC session, the camera, and the enclave calls, and nothing cryptographic.

## Goals

- A person installs the app from TestFlight, meets someone, and has a circle and a chat with them before they part, with no account and no badge.
- The app's seat key for each circle is made in the Secure Enclave and never leaves it; signatures for the manifest, grants and SSH-shaped requests come from there.
- Two phones, or a phone and the browser app, run one ceremony together with the same six words. A badge's circles reach the phone through pairing, not through a shared ceremony (section 7).
- Chat through `relay.cahoots.network` with the chat crate's epochs and rotation, readable history on the phone, and a background refresh within what iOS allows.
- A phone pairs with a badge with both buttons on the badge, and then a tap on the card fetches the chat key for the circle and epoch on screen, with the chip's own access control deciding what the phone may fetch.
- The app says plainly, on every circle, which members are phones, which are cards, and what its own seat's tier is.
- Everything cryptographic is in the Rust crates, tested on the laptop with the software chip and an in-memory relay; the Swift layer is thin and testable with XCTest.

## Non-goals, for this version

- Android. The Rust core is the part that carries over; the UI does not.
- Push notifications through Apple's service. They need a server that knows the device. The app polls when open and uses background refresh.
- Calls, media beyond images, reactions, threads, read receipts, typing indicators.
- Adding a person to an existing circle, or moving a circle from phones onto cards. Those need the circle's membership rules (C6) and fresh keys.
- The phone acting as a badge for a laptop over a cable. iPhones give apps no USB serial.
- Attestation of the phone's enclave key to another party. App Attest proves the app, not an arbitrary key; the manifest records the phone's tier as a claim, as the browser app's is.
- A relay that requires grants (DPoP). The app is built so its relay key is ready for that; the relay side is its own work.
- Grants issued by the phone. A phone cannot consume a laptop's commit requests, and nothing else consumes grants yet, so issuing them from the phone waits for the relay to become a consumer. The phone's key (section 6) is made now so that work needs no migration.
- A ceremony that mixes a phone with badges in a pile. iPhones have no serial port and badges have no camera; a laptop bridging the two is later work. A phone joins a card's circles by pairing with the card.

## Anti-goals

- A phone that holds a card's secrets. A tap fetches one epoch's chat key for one circle and nothing else; the pad, the seat key and the circle root never cross the air.
- Blind pairing. The badge names the phone, by the name the person typed on it, and both show the same four check words before the badge writes anything into the chip.
- A tier the phone did not earn. The seat key in the enclave is real custody; the pad in the keychain is OS protection and is labelled so.
- A server that learns who you are. The relay sees topics and ciphertext; the app never registers with anything.
- A second implementation of the protocol in Swift.

## What exists and is reused

- `crates/core` (ceremony, pad, check words, pair keys, seat keys, manifest, circle root), `crates/chat` (chat on root epochs), `crates/relay` (the station client, `HttpRelay` and `MemRelay`), `crates/objects` (grants, the sign grant, the evaluator, the proof, cards), `crates/custodian` (the wire protocol a badge speaks).
- `crates/wasm` proves the core compiles for a constrained target and shows the API the browser app uses: `Circle` with send and receive, `chat_send`, `chat_receive`, `rotate_epoch`, export and import, fountain-coded animated codes, bucket padding.
- `web/app/app.js`: the screens' logic to port: the ceremony's three rounds of codes, the late-joiner path, the chat screen, hello words, the relay sheet.
- The badge: `crates/badge/src/chip.rs` (the chip record, slots, `KeyKind::{Seat, Root, Epoch, Grant}`), `crates/chip/src/se05x.rs` (object ids `0x4C41_kkss`, policies, the applet), the menu and its consent prompts.
- The SE051's contactless interface: ISO 14443-4 to the same applet as over I2C, with sessions authenticated by an EC key the chip knows (an "ECKey session"), and per-object policies naming which authenticated identity may do what.
- The V1 board carries the chip's three-turn loop with a slide switch in it; the firmware keeps the contactless side off until the owner turns it on from the badge's menu (roadmap decision 16). The bench Click board has no antenna, so pairing and tap are tested on V1.

## The design

### 1. Structure

A SwiftUI app, iOS 17 and later, with one Rust static library built from a new crate, `crates/phone` (`cahoots-phone`), that depends on core, chat, relay, objects and custodian and exposes a UniFFI interface. The crate holds the app's state machine: circles, ceremonies, chats, keys, pairings, grants. Swift calls it and renders what it returns; Swift owns three things the Rust side asks for through callback interfaces: the Secure Enclave (sign with a named key, make a key), the NFC session (send an APDU, get the answer), and storage protection (where the files live). The Rust side is tested on the laptop with a software enclave (a `SeatKey` behind the same callback trait), a software chip, and the in-memory relay.

Storage: the Rust side writes CBOR records (circles, manifests, messages, the current epoch key per chat, pairings, grants) to files in the app's container, the way the badge writes its records; Swift opens the container with `NSFileProtectionComplete` so the files are unreadable while the phone is locked. Whether messages go in one append-only file per chat or in SQLite is left to the builder; the record formats are the Rust side's.

### 2. Keys and the tier

For every circle the phone is a seat in, the seat key is a P-256 key made in the Secure Enclave (`SecureEnclave.P256.Signing.PrivateKey`), stored as its encrypted representation in the keychain, and used only through a `SeatSigner` callback: the Rust ceremony asks Swift to sign the manifest with the key named for that circle, and Swift returns the signature. The public key goes into the commitment as today. The same mechanism signs grants the phone issues and any request the phone makes under a key, so the phone has one key per circle and one key for itself (section 6).

The pad, the circle root, the chat epoch keys and the message history are bytes, which the enclave cannot hold. They live in the protected files. The seat declares tier 0 in the manifest, as the browser app does, and the phone's `Describe` says the seat key is in an enclave (`in_chip: false`, with a new `key_store` field or a note in the name; the builder picks, the screen says "seat key in the Secure Enclave; secret protected by iOS"). The tiers page gains a sentence saying what an enclave seat is and is not.

### 3. Ceremonies

The app runs the ceremony as the browser app does: the phone is a seat, random bytes from the system's generator (labelled CSPRNG), contributions exchanged as animated codes, read by the camera through AVFoundation and shown on screen, the six words read aloud, Both replaced by a tap on "They match". Phones and cards mix in one circle, since the protocol already does. The pad proposal is the browser app's three sizes. A hardware randomness contribution from a badge or a file is accepted as the browser app accepts it, labelled as the laptop's or the badge's word.

Where a badge is in the room, the badge's seat is the badge's own as today; the phone does not stand in for a card's seat in this version. A ceremony with a paired card as the phone's seat is section 7's later step.

### 4. Chat, hello words, history

Chat uses the chat crate as the browser app does: the circle root seeds the epoch keys, each sender has its own chain, and a new epoch starts when a member sends in the next number. Chat is as secret as the root, so a phone that holds the root and is compromised reveals every message the relay still holds. The app reads the relay to the end, its own messages included, before its first send after a launch. The relay is `relay.cahoots.network` by default, with a per-circle override, the same setting the browser app has. The app long-polls when a chat is open, polls every circle when the app is in the foreground, and asks for background refresh (`BGAppRefreshTask`) to fetch new entries and set the badge count on the icon; iOS decides when that runs, and the app says so in its settings. History is local. Hello words are the browser app's screen.

For a circle where the phone's chat key comes from a card (section 7), the chat screen shows "tap your card" when it lacks the current epoch's key, and nothing else changes.

### 5. Grants, later

Grants issued by the phone are out of this version (see non-goals). When the relay consumes grants, the flow is two codes: the laptop shows a request code with its key, the action, the circle, a name, a budget and an expiry; the phone scans it, shows the consent fields in the badge's words, signs with the circle's enclave seat key, and shows the grant as a code the laptop reads. The phone's key in section 6 and the enclave `SeatSigner` in section 2 are what that needs, so nothing here is built twice.

### 6. The phone's own key, and how enclave keys behave

Besides a seat key per circle, the phone keeps one P-256 key for itself in the enclave: the key a badge binds a pairing to (section 7), and the key the relay will bind a DPoP proof to when the relay requires grants. It is made on first launch and shown on the settings screen as a short fingerprint. Nothing links it to a circle unless a badge or a grant names it.

Every enclave key, the seat keys and the phone's own, is created with user-presence access control: a signature or an authenticated tap asks for Face ID or the passcode. That is the phone's equivalent of both buttons, and it is why a grant screen or a pairing screen on the phone can honestly say the person approved.

Enclave keys cannot leave the device. A restored or replaced iPhone has no seat keys and no phone key, so the protected files are excluded from iCloud and iTunes backup (a pad with no seat key would be worse than nothing), "export" in settings is a readable archive of history and nothing secret, and the settings screen says in one sentence that this phone's seats do not move: a new phone means new circles. Pairings die with the phone and the badge's unpair page clears them.

The Secure Enclave signs with random nonces, not RFC 6979's deterministic ones, which the protocol specifies for seat signatures. The protocol is amended to require only that a signature verify, with deterministic signing recommended, and the phone caches each ceremony signature it makes so it re-sends the same bytes rather than signing again, since the ceremony refuses a second, different attestation for the same seat.

Files are protected in two classes: keys, the pad, the root, epoch keys and plaintext history under complete protection, unreadable while the phone is locked; ciphertext the relay already holds, relay cursors and per-circle settings under protection until first unlock, so background refresh can fetch new entries and count them for the icon while the phone is locked, and decrypt them when it is opened.

### 7. Pairing and the tap

**What has to be true of the chip first.** Today every object the badge makes in the chip carries a policy naming all sessions, because the chip was only ever reachable over the badge's own I2C bus. The moment the contactless side is on, that is no longer true: a policy that names all sessions would let any NFC reader sign with the seat key or compute HMACs with the root. So pairing is sequenced after the badge authenticates itself to the chip (an authenticated session from the Pico, which is the bus work in M4j), and from then on objects are created with policies that name the badge's own identity and, for roots and epoch counters, four reserved phone identities (authentication object ids `0x4C41_05pp`, `pp` 0 to 3) that pairing fills in later. The SE05x fixes an object's policy when the object is made, so circles made before that change cannot be paired; the badge says so on the pairing page, and the fix is a new ceremony. The pairing inbox and the public information object are created with the same care: the inbox writable by any contactless session and readable only by the badge, the public object readable by any session and writable only by the badge.

**The loop.** The chip's contactless side draws its power from the reader's field, so a tap works with the badge asleep or off; that is what makes a card a card. The firmware keeps the contactless interface disabled until the first phone is paired, enables it then, and leaves it on; the slide switch in the loop is the owner's hardware off. The firmware is not told about taps; the chip's policies are what hold while the badge sleeps. Unpairing every phone disables the interface again.

Pairing itself needs the badge awake, on its `pair a phone` page. On the badge, the `pair a phone` page polls a **pairing inbox** in the chip: a binary object with a policy that lets any contactless session write it and only the badge's own session read and delete it. On the phone, "Pair a card" asks for a name for this phone, then says "hold the phone to the card":

1. Over NFC (Core NFC `NFCISO7816Tag`, the applet's AID in the app's `Info.plist`, the tag-reading entitlement), the phone selects the applet and writes its own public key and the name into the pairing inbox, then reads the card's public pairing information (the badge's name and its cahoot face seed) from a public object the badge keeps for that purpose.
2. The badge sees the inbox filled, shows "pair with *Conner's phone*?" and four check words derived from both public keys; the phone shows the same four words. Both buttons on the badge accept.
3. The badge writes the phone's key into the chip as the next free reserved phone identity, which the policies of every pairable root and epoch counter already name for HMAC and for reading, and nothing else: the seat key's policy never names it, nor the pad key's. The badge also writes the public information the phone needs for each circle (the circle id, the relay address, the manifest's public part and the badge's cahoot seed) into the public object, so the phone can show the circle and fetch its ciphertext without being a seat. The chip record notes the phone's name and identity; up to four phones per badge; unpairing is a menu item on the badge that deletes the authentication object, which revokes every tap at once.
4. The badge clears the inbox.

**A tap.** The person opens a chat whose circle has a paired card. If the phone lacks the current epoch's key, the chat shows "tap your card". The tap opens an ECKey-authenticated session with the phone's key (the applet authenticates the identity with a challenge, so a recording of a tap replays nothing; the phone's enclave key asks for Face ID, which is the person's consent), reads the circle's epoch counter, asks the chip for `HMAC(root, "chat" || circle_id || epoch)`, closes, and the phone decrypts. One chat, one key, one tap; the phone never asks for a key it is not about to use. If the counter on the card is behind the epoch the chat has seen, the tap first advances the counter, which the policy allows only forward.

**The chat key a circle uses.** A card member's phone never holds the pad, so a circle with a card cannot run the chat crate's pad-seeded epochs. Chat in such a circle uses root epochs: the epoch key is `HMAC(root, "chat" || circle_id || n)` for a counter `n`, rotation is a message saying "epoch n+1 from here", and members who were offline catch up in order. Whether phone-only and browser circles also move to root epochs, so there is one chat scheme everywhere, or keep pad epochs with root epochs as a second mode, is the open decision below; either way the browser app gains root epochs, or it cannot be in a circle with a card.

**What the chip enforces and what it cannot.** The chip enforces which identity may use which object and how. It cannot show a screen or count a budget per phone, so a paired phone can fetch any epoch's key for any circle on that card until unpaired. That is the pairing's meaning, and the pairing screen says it: "this phone can read your chats on this card until you unpair it". Grants, which carry budgets and expiries, govern laptops and the relay, where a verifier can count; they do not govern taps.

**What a tap cannot do.** It cannot read the pad, the seat key or the root; it cannot sign; it cannot pair another phone. Those live behind policies that name only the badge's own session.

### 7a. Open decision: one chat scheme or two

Pad epochs spend 32 bytes of pad per rotation and tie the chat to the pad's randomness; root epochs spend nothing and tie it to the root, which is derived from the same contributions. Every member holds the root. One scheme (root epochs everywhere) means one code path, one set of vectors, the browser app and the phone interoperate by default, and the pad is kept for pure-pad messages, which is what it is for. Two schemes keep today's browser chat byte-for-byte and add a mode. Decided 2026-10-30 (Conner): one scheme. Chat moves to root epochs everywhere, the browser app included; the pad is spent only by pure-pad messages.

Decided the same day, after review: the epoch key is derived flat from the root (`HMAC(root, "chat" || circle_id || n)`), not from a ratchet. A ratchet (root(n+1) from root(n), old roots deleted) would give software members forward secrecy against a later device compromise, which pad epochs had and flat derivation does not. It was set aside because the chip cannot chain HMACs internally, so a paired phone would have to hold root(n) itself and could then derive every later epoch, and unpairing would no longer revoke future taps. Revocable taps are the card's promise; the loss of forward secrecy for software members is stated in docs/chat.md, and the ratchet stays recorded there as the fix if the badge can cheaply keep the chip's root advanced.

### 8. Screens

Circles (name, cahoot faces of the members, unread, and whether the phone is a seat or a paired card's screen), a chat (messages with sender faces, the epoch, rotate, the tap prompt), the roster (members, their tiers, hello words for a pair), the ceremony (as the browser app, plus the camera), settings (this phone's key fingerprint, paired cards, the relay address per circle, export, and the sentence about seats not moving). Copy in plain words; the badge's consent screens are the model for the grant screen; nothing says "tap" except the tap prompt.

### 9. Build and ship

The Rust library builds for `aarch64-apple-ios` and the simulator with `cargo build`, bindings with `uniffi-bindgen`, wrapped as an XCFramework the Xcode project links. CI builds the library and runs the Rust tests; the Xcode project builds in CI if a macOS runner is available, otherwise locally. TestFlight through Conner's developer account. The NFC entitlement and the AID are part of the plan, as is a privacy manifest (the app collects nothing).

### 10. Testing

- `crates/phone`: the state machine with a software enclave, a software chip and `MemRelay`: a two-phone ceremony, chat across three members with rotation and catch-up in root epochs, pairing (the inbox, the words, the identities filled in), a tap fetching the epoch key and refused for a key the chip did not pair, a lost-phone case (a new key cannot tap), a cached ceremony signature re-sent after a restart, and export of the store.
- The badge: the authenticated session from the Pico, objects created with policies naming the badge and the reserved phone identities, the `pair a phone` page, the inbox and public objects and their policies, unpairing, and the contactless interface enabled on first pairing, in the badge's test suites with the software chip extended to model object policies and session identities.
- The chat crate: root epochs with vectors, and the browser app speaking them.
- On device: TestFlight builds; a ceremony between two phones; chat through the live relay; the grant code with the browser app. Pairing and the tap on a V1 board when one is on the desk.

## Constraints

- iOS 17 and later; SwiftUI; Swift 5.10 or later; no third-party Swift packages unless a need is written down; the Rust crates as they are, with `crates/phone` added.
- Everything cryptographic in Rust; Swift holds keys only inside the enclave and signs through callbacks.
- Copy rules as the repository: plain words, consent fields a person can read, the four-word check on pairing, "(laptop)" or "(phone)" on anything the badge cannot check.
- The NFC path is designed against the SE051 applet's documented session and policy commands; the exact APDUs are confirmed on a V1 board the way the counter was confirmed on the bench, with a boot-time probe in bench firmware.

## Left to the builder

- Messages in SQLite or in per-chat append files; the export format.
- The UniFFI interface's exact shape (one `Phone` object with methods, or several), and how progress during a ceremony is reported to Swift (callbacks or polling).
- The camera pipeline's frame handling and the fountain-code decoder's threading.
- The background refresh cadence and what the icon badge counts.
- The pairing inbox and public objects' ids and the check-word derivation (reuse the ceremony's word list and `HMAC` over both keys).
- How the applet's ECKey session is driven from Swift (the `se05x` crate's command encoders can be reused through UniFFI so Swift only moves bytes).

## Decided, and why

- Native Swift with the Rust crates through UniFFI: NFC, the enclave, the camera and background fetch are native-only, and nothing is implemented twice.
- Two jobs stated separately: a phone is a seat on its own first; a card makes it better. The handoff dropped the card as a prerequisite, and the product needs circles before it needs cards.
- The seat key in the enclave, the pad under data protection, declared as tier 0 with an enclave key: it is what the hardware allows, and the label says it.
- Pairing with the badge's buttons and the chip's own policies: a tap talks to the chip, not the badge, so the badge's consent is spent once, at pairing, and the chip holds the line after. Grants stay on the laptop and relay paths, where budgets can be counted.
- Pairing after the badge authenticates to its own chip: turning the loop on with today's all-sessions policies would hand the seat key to any reader. This puts the tap after M4j's session work, and it means circles made before that cannot be paired.
- The contactless side stays on once a phone is paired: a card that only works while its badge is awake is not a card. The slide switch is the hardware off.
- User presence on every enclave key: Face ID is the phone's two buttons.
- Backups excluded and seats that do not move: a pad without its seat key is a liability, and the enclave does not migrate.
- Grants from the phone and mixed phone-and-badge ceremonies deferred: nothing consumes a phone's grant yet, and no channel joins a phone to a pile.
- A pairing inbox the phone writes and the badge polls: it is the only way the badge learns a phone is asking without a cable, and it keeps the loop off except while pairing.
- One chat, one key, one tap; a tap never over-fetches, and the person always knows what a tap was for.
- Grants by QR between the phone and a laptop: the phone has a camera and a seat key, the laptop has a screen, and a sign grant fits one code.

# The phone app: a group chat for your circle, with the card as its key

*Written 2026-10-28 from a conversation with Conner. The flagship of the network is a messaging app: a circle of people who met in person get a group chat that is better than Signal or Telegram for the things that matter most to them, and the card in their wallet holds the keys. This is the design for its first version. It is a design to plan from, not yet a plan; building it is its own run.*

## The idea

Today's browser app runs ceremonies and a pad-seeded chat on a phone that holds everything. The phone app keeps the ceremony and the chat and moves the keys onto the card. A member taps their card on their phone to get the key for the chat they are looking at; when the circle rotates its key, the rotation is a message everyone sees, and the next tap fetches the new one. The chip on the card computes chat keys from a circle root that never leaves silicon, so a lost or seized phone holds one epoch of one chat, and nothing that reaches the pad, the seat key or the other circles.

What the app is: a chat for circles, a wallet for circles, and a reader for cards. What it is not: an account, a contact list from a server, a social network. There is no sign-up; there is a ceremony.

## Goals

- A person with the app and a card opens a chat with their circle and reads and writes messages that only members can read, with the key taken from the card by a tap.
- A circle rotates its chat key by one member's action; every other member's next tap on that chat fetches the new key; old messages stay readable on the phone that received them.
- Pairing a phone to a card is a deliberate act with both buttons on the badge; a stranger's phone gets nothing from a tap.
- The app runs ceremonies with phones and cards mixed, as the browser app does today, and shows the same words.
- One app holds many circles; nothing in the app links two circles except the person holding the phone.
- Messages travel over the Numbers Station as sealed envelopes on a circle's topic, and over a cable or a code when there is no network.
- The protocol parts are the Rust crates compiled for the phone, not a second implementation.

## Non-goals, for this version

- Android. iOS first, since the audience and the NFC path are known there; the Rust core is the same.
- Calls, media beyond images, reactions, threads, read receipts, typing indicators.
- Push notifications through Apple's service: they need a server that knows who you are. The first version polls the station when open and in the background within what iOS allows.
- Contact discovery, invitations by link, anything that works without a ceremony.
- Circle widening. A later design.
- Attested readings, grants, SSH: those are laptop apps in the chip design; the phone shows a seat's attestation but does not run them.
- A phone without a card: the app works as today's browser app does, keys on the phone, and says so on every chat with a small open-lock mark. Nothing is hidden from such a member.

## Anti-goals

- A server that can read, forge, or map the social graph. The station stores sealed bytes by topic and nothing else.
- Keys that leave the card except the one the tap is for.
- Blind consent: the badge's pairing screen says which phone, by a name the person typed on the phone and a four-word check shown on both.
- Copying Signal's interface. The app is a terminal-like, dense, pixel-sharp thing, the same art direction as the site: dark, amber, one-bit sigils.

## What exists and is reused

- `crates/core`: ceremony, pad, check words, pair keys, seat keys, manifest; `crates/chat`: pad-seeded chat with per-sender forward secrecy; `crates/relay`: the station client; `crates/wasm` shows the core compiles for a constrained target; `web/app` has the ceremony and chat screens' logic to port.
- The chip design (`2026-10-28-chip-apps-design.md`): the circle root as an HMAC key in the chip, the epoch counter, chat keys as `HMAC(root, "chat" || circle_id || epoch)`, the contact card object, the chip record and slots.
- The SE051's contactless interface: ISO 14443-4 to the same IoT applet as over I2C, with sessions authenticated by an EC key the chip knows, and per-object policies that say which authenticated identity may do what.
- iOS: Core NFC's ISO 7816 tag session lets an app exchange APDUs with the chip; the app needs the NFC tag-reading entitlement and the chip's application identifier in its `Info.plist`.

## The design

### 1. Structure

A SwiftUI app with the Rust crates built as a static library through UniFFI: `cahoots-core`, `cahoots-chat`, `cahoots-relay`, `cahoots-objects`, and a thin `cahoots-phone` crate that holds the app's state machine (circles, chats, keys, pairing) so the Swift side is views and the NFC session only. The state machine is tested in Rust with a software chip and a fake station.

Local storage: an encrypted SQLite file (SQLCipher or the system data protection class) holding circles, manifests, messages, the current epoch key per chat, and the phone's pairing key. The pad lives on the phone only for unpaired seats.

### 2. Pairing a phone to a card

1. In the app, "Pair a card": the app makes a P-256 pairing key in the Secure Enclave and shows a name field ("Conner's phone").
2. The person holds the phone to the card. Over NFC, the app reads the card's public pairing information (the badge's name, its sigil) and sends its pairing public key and name.
3. The badge, through the Pico, shows "pair with Conner's phone?" and four check words derived from both public keys; the app shows the same four words. Both buttons on the badge accept.
4. The badge writes the phone's public key into the chip as an authentication object and sets policies: that identity may use each circle's root for HMAC, read each circle's epoch counter and contact card, and nothing else. The seat key's policy never names it.
5. The app stores the card's identity (the chip's unique id and the badge name); the badge's chip record stores the phone's name and key id. Up to four phones per card; unpairing is a menu item on the badge.

The pairing message over NFC is a plain APDU exchange with the IoT applet's session commands; the badge's consent is required because the write happens from the Pico side, not from the phone: the phone asks, the Pico writes after the buttons.

### 3. A tap

The person opens a chat. If the app lacks the current epoch's key (first open, or a rotation message arrived), the chat shows "tap your card". The tap opens an authenticated session over NFC with the pairing key, reads the circle's epoch counter, asks the chip for `HMAC(root, "chat" || circle_id || epoch)` and closes. One chat, one key, one tap. The app never asks for more than the chat on screen, by Conner's decision.

If the counter on the card is behind the epoch the chat has seen (another member rotated and this card has not heard), the tap first advances the card's counter to the chat's epoch, which the counter policy allows only forward, then fetches.

### 4. Rotation

Any member can rotate from the chat's menu. Rotation is a message in the chat, sealed under the old epoch key, saying "epoch n+1 from here". Every member's app that reads it marks the chat as needing a tap; their card advances on the next tap. Members who were offline catch up in order. Keys derive from the root, so there is no key distribution, only an agreement on the number.

Messages are sealed as today's chat crate does, with the epoch key as the chat's root key and per-sender ratchets below it, so forward secrecy per sender holds within an epoch and the epoch key itself is replaced by rotation.

### 5. Ceremonies on the phone

The app runs the ceremony as the browser app does: the phone is a seat, random bytes from the system's generator, contributions exchanged by animated codes or over the station's relay for the mixing step, the six words read aloud. When a card is paired, the phone offers "let the card take this seat": the card runs the ceremony's seat (it is in the pile, or it is wired), and the phone is its screen and network. The first version supports phone seats and card seats in the same circle, since the protocol already does.

### 6. Contact cards

After a ceremony the app asks each member to fill their card (name, handles, endpoints) and seals it to the circle under the card seal key from the chip; members' cards arrive over the station and show in the circle's roster, with the sigil the badge draws. A tap on a member's card in the roster shows their attestation if their seat is a card.

### 7. Screens

Circles list (name, sigil, members, unread), a chat (messages, sender sigils, the rotate action, the tap prompt), the roster (cards, attestation marks, the hello words for a pair when you tap a member), the ceremony flow (as the browser app), settings (pair a card, paired cards, the station address, export).

### 8. Testing

The state machine in Rust with a software chip and an in-memory station: pairing, tap, rotation across three members, catching up, a lost-phone scenario (a new pairing key cannot read old epochs). On device: a paired card, a two-circle setup, and a rotation between two phones.

## Constraints

- Everything cryptographic is in the Rust crates; Swift holds no keys except the Secure Enclave pairing key.
- No server-side identifiers: the station topic is derived from the circle id as today.
- Copy rules as the repo: plain words, a four-word check on pairing, "tap your card" as the one gesture.
- The NFC entitlement, the applet's AID in the plist, and Apple's review notes are part of the plan.

## Left to the builder

- UniFFI bindings shape, the SQLite schema, the background fetch cadence.
- The exact APDU sequence for an EC-authenticated session over the contactless interface; NXP's applet documentation specifies it and the Rust `se05x` crate implements the commands.
- Image sending: sealed blobs on the station with a size cap.

## Decided, and why

- One chat, one key, one tap: the gesture means one thing and a tap never over-fetches.
- Pairing needs the badge's buttons: the card has no other way to say yes.
- The phone app is iOS first with the Rust core shared: the NFC path and the audience are there, and nothing is implemented twice.
- Phones without cards keep working, marked: the network grows by meetings, not by hardware sales.

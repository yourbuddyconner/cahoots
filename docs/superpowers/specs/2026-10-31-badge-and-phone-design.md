# The badge and the phone: one person, one seat, and a ceremony any device can join

*Written 2026-10-31 from a conversation with Conner, after a bench test the same day in which an iPhone 11 reached the SE051's applet over NFC with the Pico unpowered. This design says how a person's badge and phone become one member of a circle, how a circle made in a pile and a circle made on phones differ, and how the ceremony's rounds move between a pile and the relay so that badges and phones can be in one circle when someone wants that. It replaces the pairing section of the [iOS app design](2026-10-30-ios-app-design.md) and the E1 row of the roadmap, and it corrects two things those documents say about the hardware.*

## The idea

A **seat** is one person's membership in a circle: one key, one declared tier, one line in the manifest. Its **custodian** is whatever holds the seat key and the circle root: a badge's chip, or a phone's enclave and files, or a browser. A **device** is anything else that person uses under the seat. It holds its own key and a grant the seat signed, it gets what it needs from the custodian, and it is never in the manifest. A phone paired to a badge is a device. A laptop on the badge's cable is a device.

**Progressive security** is then simply which custodian made the circle:

- A **phone circle** is phones and browsers, all seats, tier 0, as the app makes today.
- A **badge circle** is made in a pile. Its pad never touches a phone. Phones enter it only as devices of their owners' badges.
- A **mixed circle** has at least one phone seat and at least one badge seat. Its pad is tier 0 because the weakest link sets the pad's tier, and the app says so. Badge seats in it still sign from silicon, which is what approvals and grants care about.

The protocol knows **seats, channels and roles**, never device kinds. The three roles in a ceremony are the **host**, who opens the channel, scans each joiner and holds seat 0; the **joiner**, who takes a seat; and the **bridge**, which carries frames between two transports and adds nothing. A laptop bridges a pile onto the relay. A phone paired to a badge bridges that one badge, slowly, through the chip. Any device holds any role its hardware allows.

Everything a phone gets from a badge arrives through the chip's contactless side, which this morning's test proved reaches the same applet the badge reaches over I2C. The chip enforces who may do what, offline, with the badge asleep or off. The badge's own consent is spent once, at pairing, on its buttons.

## Goals

- A person with a badge taps their phone to it once, presses both buttons, and from then on reads and posts in the badge's circles from the phone, with the pad and the seat key never leaving the chip.
- A phone circle, a badge circle and a mixed circle each carry an honest label, and the app shows for every member which device holds the seat.
- A ceremony hosted on a phone can include badges when a laptop is at the table, with the same screens on the phone as a phone-only ceremony and the same pile for the badges.
- One face and one name per person, drawn from the device that holds the seat and shown the same on every member's screen and on the badge's own.
- A lost or unwanted phone is revoked with one press on the badge and locked out of everything after the next rotation.
- Every path is tested on the laptop with the simulated badge and the software chip, with the relay in memory, and on the bench with the Click board and a phone.

## Non-goals, for this version

- Moving a circle between kinds: a phone circle onto badges, or a badge circle admitting a phone seat. Both need the circle's membership rules (C6) and fresh keys.
- A badge joining a ceremony through a phone alone, with no laptop. The frames can travel through the chip's inbox and the e-paper, but at a few taps per round it is a fallback, not a plan, and it is not built here.
- A phone taking a seat whose key lives in a chip, the "badge as signer" form of the mixed circle. It is a small addition once pairing exists and is left for later.
- USB from a badge to a phone. iOS gives apps no serial port; the routes around that are unproven and the chip's contactless side covers the need.
- Bluetooth on the badge. The RP2350 was chosen for having no radio.
- Tap-to-sign with the seat key. A tap's display is the phone's screen, which the badge cannot vouch for. If a tap ever signs, it signs with a key of its own.
- The pad encrypted under a chip key and encrypted spends (the second half of M4j). This design needs only the first half: authenticated sessions and per-identity policies.
- Variable membership while a ceremony is open. The host chooses the count when it opens the ceremony, as today. Letting the count close later is a later refinement of the host's screen, not of the protocol.

## Anti-goals

- A phone that holds a badge's pad, seat key or root. A tap returns derived keys and public facts.
- Pairing without the badge's buttons. The inbox is open to any tap on purpose; nothing happens until the owner reads the name and the words on the badge and presses.
- A chip whose policies name every session while its antenna can be switched on. The first firmware that enables the contactless side ships with the policies in this document.
- A mixed circle that claims a tier above its phones.
- A bridge that is a member, or that can read what it carries.
- A second protocol for badges. The rounds are the same bytes on the wire, on the relay and through a bridge.

## What exists and is reused

- The ceremony (`crates/core`): commit, sealed reveal, attest, the manifest with seats and tiers, the root from the content keys, pair keys for hello words. The rounds are already the same bytes on every transport.
- The pile and ring drivers (`crates/hal/src/pile.rs`, `ring.rs`): seat discovery by position, the bottom card proposing parameters, the top card's `Count`, and frame flooding with dedupe. The bridge is a node on this wire.
- The relay ceremony channel (`crates/core/src/channel.rs`): the offer code, the per-seat encrypted messages, the join code and the host's gate. The bridge is a client of this channel.
- `cahoots-host`: the serial link to a board, the simulated cards, `logs`, the custodian client. The bridge is a new command in it.
- The badge's chip layer (`crates/badge/src/chip.rs`, `crates/chip`): the SE051 driver, the software chip, key kinds `Seat`, `Root`, `Epoch`, `Grant`, the slot per circle, `SealKey`, `GrantSign`. The policies and the phone slots go here.
- Grants and chains (`crates/objects`), the channel detail, the station gate and the channel crate's verification up a chain to an admin seat. The device grant is one more detail on the same token, verified by the same code.
- The chat crate's epochs and per-sender chains, the phone's NFC check (`ios/Cahoots/NfcDiagnosticsView.swift`), the contact card (`crates/objects/src/card.rs`), the faces (`crates/core/src/faces.rs`).

## The design

### 1. Words

A **seat**, a **custodian** and a **device** are defined above. A **slot** is one of the chip's reserved identities for a paired phone. A **tap** is one Core NFC session between the app and the chip. The **inbox** is the chip object any tap may write and only the badge may read. The **public object** is the chip object the badge writes and any tap may read. A **generation** is a run of the chat's keys under one random id; it changes only when a device is revoked. The **bridge** is defined above; the laptop running `cahoots-host bridge` is **the bridge** in a mixed ceremony.

### 2. The chip: identities and policies

Every object in the SE051 carries a policy fixed when it is made, naming which authenticated identity may do which operations. Today every object the badge makes names every session, because only the Pico could reach the chip. That changes as follows.

**The badge's identity.** The firmware holds one secret for authenticating to its chip and opens an authenticated session with it over I2C before any other command. The builder chooses between the chip's platform secure channel (AES keys, SCP03) and an EC key session, and where the secret lives on the RP2350: one-time-programmable memory where the board allows it, flash otherwise, recorded in the badge's `Describe`. The seat key, the pad key when M4j's second half lands, and the right to write the public object and read the inbox are reachable only in this session.

**Phone slots.** A badge has four slots, four authentication objects that belong to the badge and not to any circle, so a phone pairs with a badge once and reaches every circle on it. When a circle is kept, every object of that circle a phone may touch names the four slots in its policy at creation: the root, for HMAC only; the hello objects, for HMAC only; the public object, for read. The inbox, one per badge, is writable by any session. Pairing fills a slot with the phone's session key. Unpairing deletes the slot's authentication object, which ends every tap from that phone at once. A circle kept before this change has no slot policies and cannot be reached by a phone; the badge's pairing page says which circles a phone will get.

**The slot's session.** A slot holds the phone's enclave public key, and the chip authenticates the phone with its EC key session: the chip presents its own ephemeral key, the phone signs the challenge with the enclave key after Face ID, and the session's secure channel wraps every command that follows. Only Face ID on that phone can ever open the slot, and nothing extractable from the phone can. The `se05x` crate has these commands unimplemented today, so the phone crate adds them, with ECDH and ECDSA reaching the enclave through the signer callbacks the app already has, and the chip layer and the software chip learn to check the session. Decided 2026-10-31 (Conner), knowing the cost. The fallback, if the chip's EC session cannot be made to work on the bench, is the platform secure channel over AES keys made by the phone at pairing and kept behind Face ID in the keychain, which the crate already speaks; the slot's layout allows either kind of key.

**What the chip computes for a phone.** Three operations on a slot's session, all HMACs the chip already does:

- The chat generation key: `HMAC(root, "chat/gen" || circle_id || generation_id)`, where the generation id is 32 random bytes the phone learned from the bump message or the public object (section 6). Because the id is random and chosen when the generation begins, a phone cannot ask for a future generation's key ahead of time, and a phone whose slot is gone cannot ask at all. The chip keeps no generation counter; the one the badge writes today is deleted.
- Hello words. The words helper is HKDF over the pair key with no salt, so its extract step is `HMAC(zero salt, pair_key)` and its expand step is one HMAC under that result. At keep time the badge computes the extract step for each other seat and stores the result as an HMAC object, the hello object for that pair. A tap asks for the expand step, `HMAC(hello_object, "cahoots/words/v1" || speaker || counter || 0x01)`, and takes its first four bytes, which is exactly what a full seat computes. No words change for anyone; badges simply keep `n - 1` more objects per circle.
- The seal key for cards, `HMAC(root, "seal/card" || circle_id)`, so a paired phone can open and publish cards.

**The inbox and the public objects.** The inbox is one binary object per badge, a few hundred bytes, writable by any session and readable and clearable only by the badge. Each circle has a public object, a binary object of a few kilobytes, writable by the badge and readable by any session, written when the circle is kept and rewritten at pairing and unpairing. Its contents are in section 4.

**The software chip** (`crates/chip/src/soft.rs`) models identities, sessions and policies well enough that every test in this document runs without hardware, and refuses what the real chip would refuse.

### 3. The firmware: the enable pin, and the two interfaces

The SE05x chooses one interface when it boots: it looks for a field on its antenna pins first and for I2C second, and it keeps that choice until it is reset. Holding the enable pin high, as the firmware does today, boots the chip onto I2C once and leaves the antenna dead forever after.

So the firmware drops the enable pin when it has nothing to say to the chip, and raises it, which boots the chip onto I2C with no field present, only for the moments it needs the bus: at a ceremony, when signing, when pairing, when it reads the inbox. In the field the chip is powered by the phone and the badge may be asleep or off. The badge's menu keeps a switch for the contactless side as a matter of record, but with policies in place there is nothing to switch: a tap can only do what a slot allows.

Two consequences the builder designs around. The badge cannot watch the inbox while a phone writes it, so pairing is a tap and then a press, not a poll. And every I2C conversation begins with the chip's boot and the authenticated session, which the chip layer wraps so the rest of the badge sees one call.

### 4. Pairing

Pairing is between a phone and a badge, once, and brings every circle the badge holds. On the badge, the top-level menu gets a `pair a phone` page. On the phone, `Pair a card` is on Circles when the phone has no card and in Settings always.

1. The phone asks for a name for itself, "Conner's phone", then says "hold the phone to the card". It makes the slot's AES keys and writes them, its enclave public key and the name into the inbox. One tap, no authentication, since the inbox is open. On the same tap it reads each circle's public object, which any session may read, and so learns the badge's circles, their seat keys and the badge's card.
2. The owner opens `pair a phone` on the badge and presses both. The badge raises the enable pin, authenticates, reads the inbox, clears it, and shows "pair with Conner's phone?", the number of circles it will get, and four check words derived from the phone's enclave key and the badge's identity. The phone shows the same four words. Left declines. Both accepts.
3. The badge writes the AES keys into the first free slot. For each circle it signs a **device grant** (section 5) in the chip with that circle's seat key, one press for all of them. It rewrites each circle's public object: the circle id, the relay address, the manifest's sealed form, the badge's card (section 7), the device grants for every slot, the current generation id, and any pending revocations (section 5). It drops the enable pin.
4. The phone taps again, now opening the slot's session after Face ID, reads the public objects, and keeps each circle as a **card circle**: a circle record with no pad, no root and no seat key, whose seat is the badge's, with the device grant beside it. It publishes the badge's card to each circle's topic (section 7) and any pending revocations, fetches each chat's generation key, and reads the chats.

Up to four phones per badge. Unpairing is a menu item on the badge's `phones` page that deletes the slot, signs a revocation of that slot's device grants and a bump for each circle, and writes both into the public objects for the next tap by another device to publish. On the phone, `Forget this card` deletes its card circles. The pairing page says in one line what a paired phone can do: read and post in this badge's circles and manage their channels, until you unpair it.

The inbox is open on purpose. A stranger's tap can only put a name on the badge's screen, which the owner declines, and the inbox is not readable by a tap, so the AES keys in it are seen by the badge alone. The check words are what stop a swapped key: if the words differ, the phone that tapped is not the phone being paired.

### 5. The device grant

A grant on the existing token, signed by the seat key in the chip, to the phone's enclave key, with a new detail `cahoots:device` whose actions are `read`, `post`, `hello` and `manage`, scoped to one circle, and with the channel detail for the circle's channels at depth two, the shape a channel owner's self-grant has today. One grant per circle, signed together at pairing. No expiry in log positions, since the badge keeps no log position. Budget zero: a device spends no pad.

It is how the circle and the relay recognise the phone. Channels already verify a member key up a chain to an admin seat, so a paired phone makes channel member keys under this grant and is an admin of its circle's channels with no further admission. The relay's gate, when a relay requires grants, sees the same chain. Other members' apps verify anything a device signs against it: cards, generation bumps, later approvals at key assurance.

**Revocation.** Three things end a device, in this order of reach. The slot's deletion ends its taps at once, so it can never fetch another key. The generation bump (section 6) locks it out of the circle's chat from the next epoch. And a signed **revocation** of its device grants, which the badge produces at unpair, ends its channel membership and anything else that verifies the grant once it is published: the relay's gate refuses the key, members refuse its chain, and channel admins retire the channel keys it held through the channel crate's existing flow. The badge has no network, so the revocation and the bump wait in the public objects until another of the owner's devices taps, or a laptop on the cable reads them, and publishes. The pairing page and the unpair screen say exactly that: this phone's taps end now, its chat ends at the next bump, and its channel access ends when another of your devices next connects.

Chat itself verifies nothing, as the chat crate's documentation says, so the grant does not gate chat. The generation key does.

### 6. Chat by generation

Today the chat's epoch key is `HMAC(root, "chat" || circle_id || n)`, derived flat from the root, which means a device without the root needs the custodian at every rotation. Instead:

- A **generation** is named by a **generation id**, 32 random bytes. The first generation's id is derived, `SHA-256("cahoots/chat/gen0" || circle_id)`, so every member starts in the same place with nothing exchanged.
- A **generation key** is `HMAC(root, "chat/gen" || circle_id || generation_id)`.
- An **epoch key** is `HMAC(generation_key, "chat/epoch" || n)`, and everything below it, the per-sender chains and message keys, is as today.
- Members with the root compute both. A device holds the generation key for the current generation and rotates epochs for free.
- A **generation bump** is a control message carrying the new generation id, sealed under the old generation's current epoch key so the relay sees nothing, and signed by the sender's seat key or device key so members verify it against the manifest or a chain to it. Members with the root move at once. Devices need their custodian: a paired phone taps and asks the chip for the key under the id it just read. A phone whose badge is not to hand reads nothing new until it taps, and the app says so.

The id is random so that a device cannot ask its chip for a future generation's key before the bump exists, and a device whose slot is gone cannot ask at all. That is the whole of the lockout; the chip keeps no counter and enforces no order. A bump is rare: it is how a device is revoked. Rotation within a generation costs nothing and keeps today's hygiene. The epoch id in the message header carries a short tag of the generation id and `n`; the builder chooses the encoding and bumps the chat state version. The web app and the phone move together, as they did for root epochs; the browser gains generation keys or it cannot share a circle with a card's phone. The flat-derivation decision of 2026-10-30 stands in spirit: a revoked device is locked out by a bump, not by a ratchet.

The sender byte in a chat message is the seat, so two devices of one seat chatting at once would collide on a chain. The badge and the laptop never chat, so one chatting device per seat holds for now and is recorded as a limit.

### 7. Cards and faces

The contact card gains two fields: a **face seed** of 32 bytes and a **device kind** (badge, phone, browser). The badge draws its face seed once from its own randomness and keeps it (the idle screen, M4i). The phone draws one at first launch and keeps it. The browser the same. The card is signed by the seat key, sealed under the circle's card seal key, and published on the circle's topic as a station envelope, which replaces "cards travel as files". The phone publishes its own card when a ceremony completes and the badge's card after pairing, so the badge's card reaches the circle without a laptop.

Rosters, chat headers and the badge's own idle screen show the card's name and face, and fall back to today's interim face from the seat key and the local seat name until a card arrives. A person's face links their circles for members only, which is the point of a face; a per-circle fresh face is a setting on the device.

The manifest does not change. "Circle mark", which the roadmap names and nothing defines, is a mark drawn from the circle id, as the browser already draws one, and is left to the builder.

### 8. The ceremony: roles and the bridge

The rounds do not change. What changes is who may be on the wire.

**Roles.** The host opens the channel, sets the parameters including the count, scans every joiner's join code, and holds seat 0, as today. Joiners take seats. A bridge is on two transports at once and forwards frames between them: it holds no seat, is not counted, makes no key and cannot open what it carries. The phone hosts every circle it is in, including mixed ones. The laptop is a bridge and nothing else.

**The pile with a bridge at the bottom.** The `Seat` frame's `agent` byte becomes a flags byte: bit 0 agent, bit 1 bridged. A bridge sends `Seat` upward with the seat below the first badge's, the host's parameters and the bridged flag. A card that receives a bridged `Seat` takes its seat and parameters as today, marks itself bridged, passes the flag up, accepts a `Count` from below as well as from above, and does not offer its button as the start. The count comes from the host through the bridge. Nothing else in the pile changes, and an unbridged pile is byte-for-byte what it was.

**Seats in a mixed circle.** The host is seat 0. Badges take seats 1 to `m`, by position in the pile. Phones take seats above `m`. The host types the number of badges when it opens the ceremony, beside the number of members, and the offer code carries `m`, which bumps the offer to `cahoots:ceremony:v2:` and the channel message version with it. A joining phone's seat picker shows seats 1 to `m` as cards and does not offer them. The bridge reads `m` from the offer and refuses a pile with a different count. The host's screen shows the badges as seats like any other.

**The bridge's job.** `cahoots-host bridge --serial PORT --offer CODE` takes the offer the host phone shows, as text pasted or, later, scanned by the browser app, joins the channel as a client, and:

- sends the bridged `Seat` and the `Count` up the pile from the host's parameters;
- forwards every ceremony frame that comes down the pile to the channel as that seat's message, and every channel message for a non-badge seat up the pile as a frame, with the ring's dedupe so copies do not loop;
- checks seat 0's commitment against the hash in the offer before forwarding it, as a joiner does;
- prints, for each badge's commitment, that badge's join code as a code the host's camera reads from the laptop's screen. The pile's wire is the physical check that the badge is in the room; when the badge shows its own join code on the e-paper, later, the host scans that instead.

The ring expects a `Params` frame from every index. The bridge synthesises them for the non-badge seats from the host's parameters, or the ring learns to accept the host's parameters once from a bridged `Seat`; the builder chooses.

**What the phones see.** Nothing new. The host scans join codes and presses continue; joiners pick seats and wait. The mixed circle's label and tier come from the manifest's tiers, which badges declare as today and phones declare as 0.

### 9. The app

- **Circles** lists phone circles, mixed circles and card circles alike, each with its kind in words and the member faces. A card circle shows its card's face and name on its row.
- **Pair a card** is on Circles when the phone has no card and in Settings always. It pairs the phone with a badge, not with a circle, and brings every circle the badge holds.
- **Chat in a card circle** reads and posts under the generation key. When a bump arrives and the phone lacks the new key, the composer is replaced by "Tap your card to catch up".
- **People** shows each member's card name and face, which device holds the seat, and the seat's declared tier.
- **Hello words** in a card circle come from a tap, once per other member, and are cached.
- **The NFC check** stays in Settings under Bench.
- A card circle is stored under complete protection like any other, with no pad file and no root in its record.

### 10. The badge

- A `pair a phone` page per circle, and a `phones` page that lists slots by name and unpairs on both.
- A `card` page that shows the badge's face and name, with the join code of a ceremony in progress shown there later.
- `Describe` says where the badge's identity lives and whether the circle has slots.
- The declared tier stays 1 until the pad is under a chip key.

### 11. Testing

- Software chip: policies refuse a slot signing or reading the root; a slot derives the generation key and the hello key; deleting a slot ends its session; a circle without slots refuses pairing.
- Simulated badge and a phone-side test harness over a socket standing in for NFC: the pairing flow end to end, the public object round trip, the device grant verifying against the manifest, a channel member key under it admitted with no join request.
- Chat crate: vectors for generation keys, a bump moving members and locking out a device without the new key, the web and phone state versions.
- Pile: an unbridged pile unchanged against today's vectors; a bridged pile taking a `Count` from below and refusing the top card's start.
- Bridge: a mixed ceremony in memory, host and one phone seat as relay clients, two simulated cards on socket links under a bridge, all seats signing the manifest and every device showing the same six words. The same with the badge on the bench.
- Phone and two simulators: a mixed ceremony hosted on one simulator with the bridge and a simulated card.
- Bench, with the Click board: the enable pin dropped and a tap reaching the applet with the Pico powered; pairing; a chat read by tap; an unpair ending the next tap.

## Constraints

- The chip's policies are fixed at object creation, so slot policies are written when a circle is kept, and circles kept before that cannot be paired.
- The chip serves one interface per boot. The badge cycles the enable pin to speak to it and leaves the pin low otherwise.
- The badge has no clock and no log position. Device grants carry neither, and revocation is the slot and the generation.
- The inbox and the public object are small. Everything a phone needs at pairing fits in a few kilobytes; nothing larger goes through the chip.
- The sender byte in chat is the seat, so one chatting device per seat.
- The pile's `Seat` payload keeps its length; the flags byte is the old agent byte.
- A mixed circle's count is chosen by the host when it opens the ceremony, as today.

## Left to the builder

- The authentication scheme between the Pico and the chip, and where the secret lives on the RP2350.
- The object ids for slots, pair keys, the inbox and the public object; the encoding of the public object; the wire form of the inbox.
- The epoch id encoding for `(g, n)` and the chat state version bump.
- The signed form of the generation bump and where it travels on the relay.
- Whether the ring accepts parameters once from a bridged `Seat` or the bridge synthesises `Params` frames.
- The text form of the offer for `cahoots-host bridge` and how the browser app later scans it.
- The face seed's place in the badge's flash record, and the circle mark.
- The Core NFC session structure for a two-tap pairing and the one-tap reads.

## Calls made in this document

- The seat is the person and the phone is a device of it, rather than the phone being a second seat. Two seats for one person would lie about the circle's size and tier, and the grant chain already exists to say "this key acts for that seat".
- The chip's contactless side is the phone's transport, with optical as an unbuilt fallback, because the bench showed it working today with the Pico off and because it alone needs no laptop.
- Pairing is tap then press, not a poll, because the chip serves one interface per boot.
- Chat moves to generation keys so a device taps at pairing and at revocation, not at every rotation.
- Faces and names live in the card, not the manifest, so no ceremony version changes and a face can be redrawn.
- The host stays a member at seat 0, the phone hosts every circle, and the laptop is a pure bridge. Separating the host from seat 0 was considered and set aside: it would change the offer and the gate for a case that does not arise once the laptop never hosts.
- Badges take the seats after the host and before the phones, so the pile's by-position numbering needs no change.
- Tap-to-sign with the seat key is refused in this version.
- M4j is split and only its first half, sessions and policies, is a dependency here.

Calls made after the builder's review of 2026-10-31:

- A generation is named by a random id, not a counter, because the chip computes an HMAC over any input and its counters can be written as well as incremented, so a counter could not have stopped a paired phone asking for future keys early. The random id is the lockout.
- Pairing is per badge, with one inbox and four badge-wide slots, and a public object per circle, because a person taps and presses once and gets every circle, and because one circle's manifest and card fill a public object on their own.
- The slot's session is the chip's EC key session against the phone's enclave key, built now although the `se05x` crate lacks it, because a slot only Face ID can open is the property the card promises. SCP03 over AES keys is the fallback if the chip's EC session cannot be made to work on the bench.
- Hello words for a device come from a stored extract step and a tapped expand step, so no words change for any device.
- The host types the badge count and the offer carries it, so joining phones never pick a badge's seat and nothing new travels on the channel.
- Unpairing produces a signed revocation and a bump that another device publishes, so channel access ends too, and the screens say when.

## Corrections to earlier documents

- The iOS app design says the bench Click board has no antenna. It has one, with a switch; pairing and the tap are tested on it.
- The V1 board notes say the firmware keeps the contactless interface disabled. The firmware has never touched it; the interface was starved by the enable pin being held high, which boots the chip onto I2C.
- The roadmap's E1 row says scoped enrollment replaces the tap. It is the tap. The row and product row 3 are rewritten to this design.

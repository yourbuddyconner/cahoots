# The badge and the phone: one person, one seat, and a ceremony any device can join

*Written 2026-10-31 from a conversation with Conner, after a bench test the same day in which an iPhone 11 reached the SE051's applet over NFC with the Pico unpowered. This design says how a person's badge and phone become one member of a circle, how a circle made in a pile and a circle made on phones differ, and how the ceremony's rounds move between a pile and the relay so that badges and phones can be in one circle when someone wants that. It replaces the pairing section of the [iOS app design](2026-10-30-ios-app-design.md) and the E1 row of the roadmap, and it corrects two things those documents say about the hardware.*

*Updated 2026-10-31 after the build on the `chip-pairing` branch. Sections 2, 3, 4, 5, 6, 9, 10 and 11 say what was built, in passages marked "built 2026-10-31" or "as built". Where the build differs, the earlier wording is kept beside it for the record.*

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

A **seat**, a **custodian** and a **device** are defined above. A **slot** is one of the chip's reserved identities for a paired phone. A **tap** is one Core NFC session between the app and the chip. The **inbox** is the chip object any tap may write and only the badge may read. The **public object** is the chip object the badge writes and only the badge and its paired phone may read. A **generation** is a run of the chat's keys under one random id. It changes only when a device is revoked. The **bridge** is defined above. The laptop running `cahoots-host bridge` is **the bridge** in a mixed ceremony.

### 2. The chip: identities and policies

Every object in the SE051 carries a policy fixed when it is made, naming which authenticated identity may do which operations. Today every object the badge makes names every session, because only the Pico could reach the chip. That changes as follows.

**The badge's identity (built 2026-10-31).** The badge owns its chip through a user-id authentication object, the owner object. The firmware opens the owner session over I2C before any other command. The owner secret is derived from the board's identity: HKDF-SHA256 over the RP2350's device id and wafer id and the 128-bit random id the factory writes into its one-time-programmable memory (RANDID, OTP rows 0x008 to 0x00F), with the salt `cahoots/owner/v1`, 16 bytes. Nothing is stored, so a reflash recovers it. The boot ROM reports the device and wafer ids to any program and over USB, and it does not report RANDID. But in BOOTSEL mode the USB interface can read the whole of that OTP page, RANDID included, and a USB host can reboot the badge into BOOTSEL. Anyone who learns the secret's inputs can open the owner session over NFC for good. So a badge that was plugged into a USB host that is not trusted must be retired, as one whose board id has leaked must be. A chip owned under the first derivation, from the device and wafer ids alone, does not open with the new secret and cannot be keyed again, because nobody may delete its owner object. So the firmware tries that older secret when the new one does not open the owner object, uses the chip under it, and logs "se051: owner session open with the older secret" once per boot. Such a chip keeps the older, weaker derivation for good. The only one is the bench chip. A chip whose owner object opens with neither secret is not used, and the badge says "this chip belongs to another badge or an older build. It cannot be used." The bench showed that the chip has no factory reset object and refuses DeleteAll in every session. So a tap cannot wipe a badge, and a lost owner secret cannot be recovered. Nobody may delete the owner object, not even its own session. The seat key, the pad key when M4j's second half lands, the right to write the public objects and the right to read the inbox are reachable only in the owner session.

The spec's earlier wording, kept for the record: "The builder chooses between the chip's platform secure channel (AES keys, SCP03) and an EC key session, and where the secret lives on the RP2350: one-time-programmable memory where the board allows it, flash otherwise, recorded in the badge's `Describe`." `Describe` does not say where the identity lives. It is the same on every badge of this firmware.

**The upgrade wipe (built 2026-10-31).** Every object the badge made before this firmware gave every right to every session, so a tap could have used it. The first boot of this firmware therefore deletes every Cahoots object in the chip, forgets the circles whose keys were in the chip and their grants, and then makes the owner object, the phone slot, the inbox and the grant issue counter. It shows "updated for phones. Make a new circle." once. Circles with software keys are kept. If an owner object is there and does not open with this board's secret, the badge touches nothing and does not use the chip.

**No antenna-facing id is ever empty, and none is adopted (built 2026-10-31).** The bench showed that any session, a tap included, can make a binary object at an empty id, and that object gets every right for every session. The ids that matter are the phone slot, the inbox, the grant issue counter, the pairing object and each circle's public object. The badge never leaves one of them empty and never uses an object at one of them that it did not make in the same conversation. The pairing object is made at the first boot with no pairing open and kept made, and the badge writes over it in place.

A probe on 2026-10-31 (SE051, applet 7.2.0, `cahoots-host chip raw` in no session, each at an empty id) found that a tap can make every kind of object, not only a binary one. WriteSymmKey made an HMAC key (`9000`), an AES key as an authentication object (`9000`, and that object could not then be deleted in no session, `6986`) and a plain AES key (`9000`). WriteECKey generated a P-256 key pair (`9000`). WriteCounter made an 8-byte counter (`9000`). None of this reaches a key of the badge's: the badge flags its record before it empties an antenna-facing id, deletes whatever stands at an id before it makes its own there, and a phone reads only the circle slots the pairing object names.

In detail:

- A conversation that empties one of these ids for a moment first stores its chip record with a flag. A boot that finds the flag deletes the four badge-wide objects and makes them again with the pin high, in one owner conversation. A boot that finds one of them missing stores the flag first, then deletes and makes them, then clears the flag, so a power cut between the delete and the make leaves the next boot to do it again.
- If an object at the slot, the inbox, the issue counter or the pairing object cannot be deleted, the badge refuses the chip for good and says "the chip was touched or could not be set up. This chip cannot be used. Circles on this badge keep software keys."
- A failure that would leave the slot empty holds the chip: the pin stays high, so the antenna is dead, until the badge is turned off and on. The list then says "chip needs a restart".
- The badge reads back a circle's public object only when its record says the badge wrote that object. Otherwise it deletes the object and writes a new one in the same conversation. A read of one it wrote that fails stops the pairing or unpairing with "the chip did not answer. Try again.", and nothing is changed. If a planted object there cannot be deleted, the badge marks the circle's slot stuck and leaves that circle out of pairing. At an unpair it drops that circle's bump and says "a circle could not be updated. Forget it to clear this."
- The pairing object names the circle slots whose public objects the badge wrote, as a 16-bit mask. The badge rewrites it at every keep, forget, pairing and unpairing, and at boot. A phone reads only those slots, and refuses two that name one circle with "This card's records disagree. Unpair and pair again."

**One phone slot (built 2026-10-31).** A badge has one slot: one AES-128 authentication object that belongs to the badge and not to any circle, so a phone pairs with a badge once and reaches every circle on it. While no phone is paired, the slot holds a random key that nobody knows, under a policy that lets only the owner delete it. Pairing puts the phone's key in its place. Unpairing puts a new random key back, which ends every tap from that phone at once. One phone per badge holds until the chat separates senders per device (section 6).

When a circle is kept, every object of that circle a phone may touch names the slot in its policy at creation. The root and the hello objects allow an HMAC in the slot's session under secure messaging. The public object allows a read in the owner's session and in the slot's session under secure messaging, and in no other. The inbox, one per badge, is writable by any session. A circle kept before this firmware was wiped at the upgrade, so every circle on a badge can be reached by its phone.

The spec's earlier wording, kept for the record: "A badge has four slots, four authentication objects". Four phones would chat as one seat on one sender chain, so the build keeps one.

**The policies (built 2026-10-31).** Every object's policy names the owner session, the slot's session and no session. The applet keeps one entry for each identity a policy names. A session the policy names gets only its own entry's rights. A session the policy does not name gets the rights of the no-session entry (bench, applet 7.2.0). A policy refusal is status word `6986`.

| Object | Owner session | Slot session | No session (any tap) |
|---|---|---|---|
| Owner object | nothing | nothing | read its attributes |
| Seat key | sign, read its public key, attest, delete | nothing | nothing |
| Root, hello object | HMAC, delete | HMAC, under SCP03 only | nothing |
| Epoch counter, grant counter, issue counter | read, move, delete | nothing | nothing |
| Public object | read, write, delete | read, under SCP03 only | nothing |
| Pairing object | read, write, delete | read, under SCP03 only | read |
| Inbox | read, write, delete | write, under SCP03 only | write |
| Phone slot | delete | nothing | nothing |

The applet refuses a counter whose policy carries sign or attest rights, so counters have their own policy. The object ids are in the [protocol](../../protocol.md), section 9.

**The slot's session (built 2026-10-31).** The phone opens the slot with the chip's platform secure channel over an AES key, SCP03, and not with the EC key session this section first chose. The applet's EC key session is undocumented here, its key model is not what this spec assumed, and nothing on the bench could prove it before the build. The EC session stays the intended upgrade.

As built, the phone draws a fresh 16-byte AES key at each pairing and the badge puts it in the slot. The phone keeps it in the keychain behind user presence, so Face ID or the passcode releases it for each tap after pairing. A tap that opens the slot sends CreateSession for the slot, then the SCP03 handshake inside the session, at the full security level: command and response MACs, and command and response encryption. The same key serves for encryption, MAC and key encryption. The phone's APDUs are encoded by hand in the phone crate, so the LGPL `se05x` library stays in the firmware only.

The property this gives up: the slot key is a keychain item, not an enclave key. Face ID guards it, but it is a secret that exists outside the Secure Enclave, where the EC session would have needed the enclave key itself.

The spec's earlier wording, kept for the record: "A slot holds the phone's enclave public key, and the chip authenticates the phone with its EC key session: the chip presents its own ephemeral key, the phone signs the challenge with the enclave key after Face ID, and the session's secure channel wraps every command that follows. Only Face ID on that phone can ever open the slot, and nothing extractable from the phone can."

**What the chip computes for a phone.** Three operations on a slot's session, all HMACs the chip already does. The chip does not look at the data, so a paired phone may ask for any HMAC under the root or a hello object. These three are what the app asks for:

- The chat generation key: `HMAC(root, "chat/gen" || circle_id || generation_id)`, where the generation id is 32 random bytes the phone learned from the bump message or the public object (section 6). Because the id is random and chosen when the generation begins, a phone cannot ask for a future generation's key ahead of time, and a phone whose slot is gone cannot ask at all. The chip keeps no generation counter, and the one the badge writes today is deleted. As built (2026-10-31), the keep still makes the circle's epoch counter, for the owner session alone, and nothing a phone does reads or moves it.
- Hello words. The words helper is HKDF over the pair key with no salt, so its extract step is `HMAC(zero salt, pair_key)` and its expand step is one HMAC under that result. At keep time the badge computes the extract step for each other seat and stores the result as an HMAC object, the hello object for that pair. A tap asks for the expand step, `HMAC(hello_object, "cahoots/words/v1" || speaker || counter || 0x01)`, and takes its first four bytes, which is exactly what a full seat computes. No words change for anyone; badges simply keep `n - 1` more objects per circle.
- The seal key for cards, `HMAC(root, "seal/card" || circle_id)`, so a paired phone can open and publish cards. As built, cards travel in the chat (section 7), so the app does not ask for this one.

**The inbox and the public objects (built 2026-10-31).** The inbox is one binary object per badge, 256 bytes, writable by any session and readable and clearable only by the badge. Each circle has a public object, writable by the badge and readable by the badge and its paired phone only, written when the circle is kept and rewritten at pairing and unpairing. A new public object is its first contents plus 1 KB of room, for the device grant, the pending bumps, the phone's key and a card written later, and at most 8 KB. A badge-wide pairing object of 128 bytes, readable by any session, carries the open pairing's nonce and the badge's key for it (section 4), the badge's name, and which circle slots the badge vouches for. The encodings are in the [protocol](../../protocol.md), section 9.

The first build let any session read the public objects, so a stranger's tap could read each circle's manifest, its card and the paired phone's key. The final review moved them behind the owner and the slot. The first boot of the firmware that did so makes every public object again under the new policy, with the same contents, and the chip record says which policy the objects were made under. The bench chip's two circles were made again that way on 2026-11-01.

**How many circles a chip holds (built 2026-10-31).** On the bench SE051, each circle took about 2.7 KB of the chip's memory: its seat key, root, counter, hello objects and public object. The bench chip held six circles before it refused a seventh, and about seven is the estimate for an empty chip of this kind. The Pico has 16 places for circles, which does not mean 16 circles in the chip. The public object's room was first 2 KB, and the bench chip then ran out of memory at five circles. When the chip refuses a keep for want of memory (status word `6A84`), the badge does not keep the circle, deletes the seat key it made for it, and says "the chip is full. Forget a circle first."

**A known limit: a tap can use up circle slots (recorded 2026-10-31).** Any session can make an object at an empty id, and it can make one whose policy lets nobody delete it. A tap can fill the ids of a circle slot no circle uses. The badge then passes that slot over when it keeps a circle, or marks it stuck. Sixteen such slots leave no room for a new circle's keys, and the remedy is a new chip. An undeletable object planted at the pairing object's id before the badge's first boot refuses the chip, as one at the slot, the inbox or the issue counter does. Afterwards that id is never empty. This is a denial of service and leaks no key.

**The software chip** (`crates/chip/src/soft.rs`) models identities, sessions and policies well enough that every test in this document runs without hardware, and refuses what the real chip would refuse.

### 3. The firmware: the enable pin, and the two interfaces

The SE05x chooses one interface when it boots: it looks for a field on its antenna pins first and for I2C second, and it keeps that choice until it is reset. Holding the enable pin high, as the firmware does today, boots the chip onto I2C once and leaves the antenna dead forever after.

So the firmware drops the enable pin when it has nothing to say to the chip, and raises it, which boots the chip onto I2C with no field present, only for the moments it needs the bus: at a ceremony, when signing, when pairing, when it reads the inbox. In the field the chip is powered by the phone and the badge may be asleep or off. The badge's menu keeps a switch for the contactless side as a matter of record, but with policies in place there is nothing to switch: a tap can only do what a slot allows.

Two consequences the builder designs around. The badge cannot watch the inbox while a phone writes it, so pairing is a tap and then a press, not a poll. And every I2C conversation begins with the chip's boot and the authenticated session, which the chip layer wraps so the rest of the badge sees one call.

**As built (2026-10-31).** The pin rests low. It rises once at power-on to find the chip. Each conversation raises the pin, waits 10 ms, resynchronises and soft-resets the chip, selects the applet and opens the owner session. At the end it closes the session and drops the pin. The badge ends its conversation at the end of each step, except during a ceremony. No menu switch for the contactless side was built. The V1 board's slide switch in the antenna line is the only switch, and the slot's policies decide what a tap may do.

Commands over 254 bytes failed about half the time on the bench over I2C, a defect in the `se05x` driver's chained writes. The badge writes binary objects in 160-byte pieces, so every command fits one frame.

A bench build of the firmware also passes APDUs from the laptop to the chip over I2C in no session, with the pin raised for the exchange and no owner session open, as a phone's tap finds the chip. Frame kinds 32 and 33 carry them. `cahoots-host chip` runs the phone's own chip client over it, which is how pairing was proved on the real chip before any phone had the app (section 11).

### 4. Pairing

Pairing is between a phone and a badge, once, and brings every circle the badge holds.

**The flow as built (2026-10-31): open the page, tap, press, tap again.**

1. The owner opens `pair a phone` on the badge. The badge draws a fresh 16-byte pairing nonce and an ephemeral P-256 key pair, keeps both in RAM, empties the inbox, and writes the nonce and the key's public half to the pairing object, which any tap may read. It shows "pair a phone", "hold the phone to the chip", "then press both".
2. On the phone, the person opens Settings, `Pair a card`, types a name for the phone, such as "Conner's phone", and holds the phone to the chip. In this one tap, in no session, the phone reads the pairing object: the nonce, the badge's key, the badge's name and which circle slots the badge holds. It reads no public object, since no session may. It draws a fresh AES key for the slot, seals it to the badge's ephemeral key, signs the request with its enclave key, which is where Face ID asks, and writes it to the inbox. The phone then shows the badge's name, or "your badge" when it has none, how many circles it holds, and six words, and says "On the badge, press both to see its words. If they match, press both again, then hold the phone to the card."
3. The owner presses both. The badge reads the inbox in its owner session and checks the phone's signature against the open page's nonce and key. Then it opens the sealed key and shows "pair with Conner's phone?" and six words, in two rows of three. The words come from the nonce, the badge's ephemeral key, the phone's public key and the AES key. The owner checks that the words match the phone's and presses both again. Left or right declines and empties the inbox.
4. The badge signs one **device grant** (section 5) per circle with that circle's seat key in the chip. It rewrites each circle's public object with the grant, the phone's public key, the current chat generation and any pending bumps (section 6). It puts the AES key in the slot, empties the inbox, writes a zero nonce over the pairing object and drops the ephemeral key. It shows "paired with Conner's phone" and "tap the phone again".
5. The person holds the phone to the chip again. The phone opens the slot's session with the AES key it still holds in memory from the first tap, so this tap asks nothing. In the session it reads the pairing object again and the public object of each circle slot it names, and no other. Each one must name this phone's key, and no circle may be named twice. The phone verifies each circle's manifest and its device grant against it, then keeps each circle as a **card circle**: a circle record with no pad, no root and no seat key, whose seat is the badge's. In the same tap it fetches the generation key for the circle's current generation and for the generation each pending bump moves to. Only then does it keep the AES key in the keychain. It posts the pending bumps in order, publishes the badge's card, and shows "Paired with" the badge and the circles it kept.

A second tap before the owner has pressed both says "Press both buttons on the badge first, then hold the phone again." A request that does not verify gets "that phone's request is not for this badge. Try again." on the badge, and pressing both with nothing in the inbox gets "no phone has tapped yet". A phone refuses a badge that holds no circles, with "This badge holds no circles yet. Make a circle with it first." On the badge, `pair a phone` is an entry in the list of circles, which a badge reaches once it holds a circle. Pairing from the pile screen of a badge with no circles is not built. A brand-new badge makes its circle first.

**One phone per badge (built 2026-10-31).** A second pairing while a phone is paired gets "unpair the current phone first".

**Unpairing (built 2026-10-31).** While a phone is paired, the badge's list has a `phone` entry. It shows the phone's name and "reads and posts in this badge's circles" and "unpairing ends its taps". Both unpairs. The badge signs a bump for each circle with the seat key and stores the unpair in its chip record before it touches anything. It puts a fresh random key in the slot, which ends the phone's taps at once. Then it adds each bump to its circle's public object and clears the grant and the phone key there. A conversation cut off anywhere in this is finished at the next boot. The screen says "phone unpaired" and "its taps end now". On the phone, `Forget this card` in Settings deletes its card circles and the phone's key for the card, and the badge keeps its slot until it is unpaired there.

**Six words, not four (decided at the final review, 2026-10-31).** The first build showed four words, 44 bits. An NFC relay between the phone and the badge could try that many keys of its own in the time the owner takes to compare, so the words are six, drawn as the ceremony's six are: the first 66 bits of the hash.

**Why the inbox is safe to leave open.** A stranger's tap can only put a name on the badge's screen, which the owner declines. A request is signed over the nonce of the page now open, so a request recorded at an earlier pairing does not verify. The AES key in the request is sealed to the badge's ephemeral key, so an antenna that listens to the tap learns nothing it can open. The words cover the nonce, both keys and the AES key, so if the words differ, the phone that tapped is not the phone being paired, or its key was swapped. The seal was added in the build after a review found that the first build sent the AES key in the clear.

**The flow this section first gave, kept for the record.** On the badge, a top-level `pair a phone` page. On the phone, `Pair a card` on Circles when the phone has no card and in Settings always.

1. The phone asks for a name for itself, "Conner's phone", then says "hold the phone to the card". It makes the slot's AES keys and writes them, its enclave public key and the name into the inbox. One tap, no authentication, since the inbox is open. On the same tap it reads each circle's public object, which any session may read, and so learns the badge's circles, their seat keys and the badge's card.
2. The owner opens `pair a phone` on the badge and presses both. The badge raises the enable pin, authenticates, reads the inbox, clears it, and shows "pair with Conner's phone?", the number of circles it will get, and four check words derived from the phone's enclave key and the badge's identity. The phone shows the same four words. Left declines. Both accepts.
3. The badge writes the AES keys into the first free slot. For each circle it signs a **device grant** (section 5) in the chip with that circle's seat key, one press for all of them. It rewrites each circle's public object: the circle id, the relay address, the manifest's sealed form, the badge's card (section 7), the device grants for every slot, the current generation id, and any pending revocations (section 5). It drops the enable pin.
4. The phone taps again, now opening the slot's session after Face ID, reads the public objects, and keeps each circle as a **card circle**: a circle record with no pad, no root and no seat key, whose seat is the badge's, with the device grant beside it. It publishes the badge's card to each circle's topic (section 7) and any pending revocations, fetches each chat's generation key, and reads the chats.

That flow let a tap write the request before the badge had a nonce for it, so the words could not bind the AES key, and the key travelled in the clear. It also said "Up to four phones per badge" and had unpairing sign a revocation of the slot's device grants. Its step 4 put Face ID at the second tap. As built, Face ID asks at the first tap, where the phone signs the request, and the second tap asks nothing. Face ID asks again at each later tap, which reads the slot key from the keychain.

### 5. The device grant

**As built (2026-10-31).** A version 2 grant on the existing token, signed by the seat key in the chip under kid `[seat]`, to the phone's enclave key, with one detail `{"type": "cahoots:device", "actions": ["read", "post", "hello"]}`, scoped to one circle by its issuer. It names the seat as its grantor, has depth 0, and carries no `aud`, no expiry, no log position, no parent and no base. A device spends no pad. One grant per circle, signed together at pairing, and one for a circle kept later while a phone is paired. The exact claims are in the [protocol](../../protocol.md), section 9.

It carries no channel detail and no `manage` action, and nothing verifies it yet except the phone itself. At the second tap the phone checks each grant against the circle's manifest and its own key, and takes the badge's seat from it. Channel management from a card circle, the relay's gate and other members' checks of a device's chain are deferred. A chain check refuses a device grant today, because it accepts only pad grants.

Unpairing does not revoke the grant. It has no expiry and no revocation yet. Unpairing clears the grant and the phone's key from each public object at once, so any future verifier must also check that the circle's public object still names the grantee as its paired phone.

**Revocation as built.** Two things end a device. Replacing the slot's key ends its taps at once, so it can never fetch another key. The generation bump (section 6) locks it out of the circle's chat once members follow it. The badge has no network, so the bumps wait in the public objects until the next phone that taps posts them. Until then the circle chats in the old generation, which the unpaired phone can still read. The unpair screen says "its taps end now".

**The design this section first gave, kept for the record.** "A grant on the existing token, signed by the seat key in the chip, to the phone's enclave key, with a new detail `cahoots:device` whose actions are `read`, `post`, `hello` and `manage`, scoped to one circle, and with the channel detail for the circle's channels at depth two, the shape a channel owner's self-grant has today. One grant per circle, signed together at pairing. No expiry in log positions, since the badge keeps no log position. Budget zero: a device spends no pad."

It went on: the grant is how the circle and the relay recognise the phone. A paired phone makes channel member keys under it and is an admin of its circle's channels with no further admission. The relay's gate sees the same chain. Other members' apps verify anything a device signs against it. And a signed **revocation** of the device grants, produced at unpair and published by another of the owner's devices, ends the device's channel membership. None of that is built.

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

**The badge's bumps, as built (2026-10-31).** A badge signs a bump for each circle when it unpairs a phone. The bump is the chat crate's payload, `from_id || to_id || COSE_Sign1` by the seat key. The badge has no network, so the next phone that taps forwards it. The chat crate lets a device forward a seat-signed bump: the device must be in the generation the bump moves from, and the bump must move to a generation the device does not hold. After sending it, the device waits for the new key, and a tap fetches it. The phone forwards the bumps at the end of its first poll after pairing, in order, and fetches the keys for them in the pairing's second tap.

The public object keeps the pending bumps as a short list, oldest first, each moving from where the one before moves to. Field 5 is the generation after the last of them. The badge never clears the list, because it cannot know whether a device posted it. At the next unpair, with W→X the last bump and X the generation the badge recorded:

- If no phone was paired since that bump, nobody can have posted it. The badge replaces it with W→Y, signed fresh, and members still in W follow it in one step.
- If a phone was paired since, it may have posted W→X and members may be in X. The badge appends X→Y.
- With no list, or a last bump that does not end at X, the list starts again as X→Y.

The list holds at most four bumps. At the cap the oldest two, A→B and B→C, are joined into A→C, signed fresh. That strands members who followed A→B and not B→C. It needs four cycles of pairing a phone that never taps and unpairing it, with nobody posting in between. A reset by any member with the root recovers them. The phone that pairs reads the whole list and starts from its first generation.

The badge records each circle's generation in its chip record as the generation after its pending bumps.

**Known limits, recorded 2026-10-31.**

- The badge never reads the relay. If a member resets the chat after the badge last saw the circle, the bump the badge signs at unpair moves from a generation the circle has left. Members ignore it as stale, and the unpaired phone keeps the current generation's key until someone resets again. The fix, for a phone to tell the badge the current generation through the inbox, is deferred.
- Two members who reset at once from the same generation end in the lower id, as the chat crate rules. A device that reads both before its next tap waits for the lower one too, whichever it read first, so it ends where the members with the root end (built at the final review). Three resets at once from one generation are not tested, and a member could be stranded by them until someone resets again. A card circle waits for one generation at a time, so if two bumps in a row, one after the other, arrive before a tap, the phone fetches the key it waits for, and a message sealed in a generation between them may be passed over.
- One chatting device per seat, as above. That is why a badge takes one phone.

### 7. Cards and faces

The contact card gains two optional fields: a **face seed** of 32 bytes under key 7 and a **device kind** under key 8 (0 phone, 1 badge, 2 browser, 3 laptop). Key 6 stays the log position. A card made before these keys reads the same. A name is 1 to 64 characters after trimming, and the card allows up to 256 bytes so that 64 characters of four bytes each fit. The badge draws its face seed once from its own randomness and keeps it (the idle screen, M4i). The phone draws one at first launch and keeps it. The browser does the same and keeps it in its storage beside the circles.

The card is signed by the seat key and travels in the circle's chat as a card message (message type 3). Its plaintext is the signed card, bucket-padded like a text, so the length of the name does not show. This replaces "cards travel as files" and the station envelope this section first named. A reader removes the padding, verifies the card against the manifest, and refuses a card whose seat is not the seat that sent the message. It keeps one card per seat and replaces it only with a card read at the same or a later relay position. A card is never shown as a message. A device publishes its own card when a ceremony completes and again after a name change or a new face. The phone publishes the badge's card after pairing, so the badge's card reaches the circle without a laptop.

Rosters, chat headers and the badge's own idle screen show the card's name and face, and fall back to today's interim face from the seat key and the local seat name until a card arrives. A person's face links their circles for members only, which is the point of a face; a per-circle fresh face is a setting on the device.

The manifest does not change. "Circle mark", which the roadmap names and nothing defines, is a mark drawn from the circle id, as the browser already draws one, and is left to the builder.

### 8. The ceremony: roles and the bridge

The rounds do not change. What changes is who may be on the wire.

**Roles.** The host opens the channel, sets the parameters including the count, scans every joiner's join code, and holds seat 0, as today. Joiners take seats. A bridge is on two transports at once and forwards frames between them: it holds no seat, is not counted, makes no key and cannot open what it carries. The phone hosts every circle it is in, including mixed ones. The laptop is a bridge and nothing else.

**The pile with a bridge at the bottom.** The `Seat` frame's `agent` byte becomes a flags byte: bit 0 agent, bit 1 bridged. A bridge sends `Seat` upward with the seat below the first badge's, the host's parameters and the bridged flag. A card that receives a bridged `Seat` takes its seat and parameters as today, marks itself bridged, passes the flag up, accepts a `Count` from below as well as from above, and does not offer its button as the start. The count comes from the host through the bridge. Nothing else in the pile changes, and an unbridged pile is byte-for-byte what it was.

**Seats in a mixed circle.** The host is seat 0. Badges take seats 1 to `m`, by position in the pile. Phones take seats above `m`. The host types the number of badges when it opens the ceremony, beside the number of members, and the offer code carries `m` as a version 2 code, `cahoots:ceremony:v2:`. A circle with no badges keeps the version 1 code, so updated and older apps still make ordinary circles together, and the channel message version does not change (decided at the final review, 2026-10-31). A joining phone's seat picker shows seats 1 to `m` as cards and does not offer them. The bridge reads `m` from the offer and refuses a pile with a different count. The host's screen shows the badges as seats like any other.

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

**As built (2026-10-31).**

- **Circles** shows a card circle with the badge's face and the line "Card circle on" the badge's name, then who else is in it. The row drops the seat number. It adds "Tap your card to catch up." when the phone lacks a key. The phone has no mixed circles yet. The crate calls a circle's kind "seat" or "card".
- **Pair a card** is a section of Settings, always, and a link on Circles while the phone has no card circle. Its three steps are the name, the words, and done. The words step shows the six words and how many circles the badge holds, and has "Start over". The circles' names come with the second tap, inside the slot's session.
- **Forget this card** is in Settings while the phone has card circles. It asks first, then deletes the card circles and the phone's key for the card. The badge keeps its slot until it is unpaired there.
- **Chat in a card circle** shows "Tap your card to catch up" in place of the composer while the phone lacks a key. The button taps, fetches the key, and reads on. A tap is needed only when there is a key to fetch. The chat reads the circle list every 3 seconds to learn this.
- **People** shows "your card" for the card circle's own seat, with the badge's name and face, and no pencil.
- **Hello words** in a card circle come from a tap each time. The number moves only after a tap succeeds.
- After an unpair, the next tap, for hello words or for a key, says "This card no longer knows this phone."

**What the phone keeps for a card circle (built 2026-10-31).** The circle record, under complete protection, holds the badge's name and public key, its circle slot, the current generation id, the label of the keychain item that holds the slot key, and any bumps still to post. It holds the generation key itself only until the chat state is first saved, and the chat state then holds what it needs to read and post in that generation. Keys fetched ahead for pending bumps are kept in memory only, so a restart before they are used costs another tap. The slot's AES key is one keychain item per badge, behind user presence. So a phone unpaired from its badge can still read and post in the current generation until a bump moves the circle on, and a phone that is lost before the owner unpairs it holds a key that opens the slot once Face ID or the passcode releases it.

The spec's earlier list, above, said `Pair a card` is on Circles when the phone has no card. As built it is a link there, and the section is in Settings.

### 10. The badge

- A `pair a phone` page per circle, and a `phones` page that lists slots by name and unpairs on both.
- A `card` page that shows the badge's face and name, with the join code of a ceremony in progress shown there later.
- `Describe` says where the badge's identity lives and whether the circle has slots.
- The declared tier stays 1 until the pad is under a chip key.

**As built (2026-10-31).** The badge's list of circles gains `pair a phone` on any badge whose chip it owns, and `phone` while a phone is paired. These are badge-wide entries, after the circles, not pages per circle. The `phone` page shows the paired phone's name and unpairs on both. A badge with no circles runs the pile screen, which has no way to the list, so it pairs once it has a circle. The badge shows a phone's name in printable ASCII only, because a tap writes it. While the badge holds its chip after a failure, the list's title is "chip needs a restart" and it has no `pair a phone`. The `card` page and the change to `Describe` are not built. The pairing object carries an empty badge name, because the badge has no name yet, and the phone calls it "your badge". The declared tier stays 1.

### 11. Testing

- Software chip: policies refuse a slot signing or reading the root; a slot derives the generation key and the hello key; deleting a slot ends its session; a circle without slots refuses pairing.
- Simulated badge and a phone-side test harness over a socket standing in for NFC: the pairing flow end to end, the public object round trip, the device grant verifying against the manifest, a channel member key under it admitted with no join request.
- Chat crate: vectors for generation keys, a bump moving members and locking out a device without the new key, the web and phone state versions.
- Pile: an unbridged pile unchanged against today's vectors; a bridged pile taking a `Count` from below and refusing the top card's start.
- Bridge: a mixed ceremony in memory, host and one phone seat as relay clients, two simulated cards on socket links under a bridge, all seats signing the manifest and every device showing the same six words. The same with the badge on the bench.
- Phone and two simulators: a mixed ceremony hosted on one simulator with the bridge and a simulated card.
- Bench, with the Click board: the enable pin dropped and a tap reaching the applet with the Pico powered; pairing; a chat read by tap; an unpair ending the next tap.

**What ran (2026-10-31).**

- Software chip and badge: the policies and the applet's rules as measured on the bench. A slot session derives the generation key and the hello words and is refused a signature, the seat's public key, reading the inbox, moving the epoch counter, writing a public object, and deleting the owner object, the slot or the root. No session reads a public object. The pairing object is made at the first boot and names the circles, a planted one before the first boot refuses the chip, a remake cut off between its deletes and its makes is done again at the next boot with nothing a tap made adopted, public objects of the first policy are made again once, a failed read of the badge's own public object changes nothing, and a stuck circle at unpair is dropped. Replacing the slot's key ends its session. The pairing flow, refused requests (a swapped AES key, another badge's key, an earlier nonce, a bad signature), unpairs cut off at every step and finished at boot, the bump list rules, planted objects at every antenna-facing id, and a keep on a full chip.
- Phone: the SCP03 host side against RFC 4493's AES-CMAC examples and the worked example in Osmocom pySim, and against the software chip's own SCP03, which was written separately. Card circles end to end over a fake NFC reader on the software chip and a relay in memory: pairing, with only the circle count before the press, the slots the badge vouches for read in the session and a circle named twice refused, chat, a bump and the tap after it, hello words from two chips, forwarding a pending bump, a relay out of reach at the second tap, and an unpair ending the next tap.
- Bench, with the Pico 2 and its SE051, applet 7.2.0: `scripts/pair-bench.sh` ran the phone's chip client on a laptop through the bench pass-through. The bench also measured the chip's memory (section 2). The last run, on 2026-11-01, used the final review's firmware with the first owner derivation, since the bench chip was owned under it. At boot the badge made the chip's two older public objects again under the new policy. The run kept a circle, read the pairing object naming slots 0, 1 and 2, and found each of their public objects refused in no session. An unpaired tap was refused ten operations with `6986`, reading the public object among them. The laptop paired as a phone with the same six words on both (theory wave elite still setup eyebrow), opened the slot's SCP03 session, got an HMAC from the root, and in that session read every public object with the grant and the laptop's key. After an unpair, the old key no longer opened the slot, and the badge's log said it wrote each public object with a pending bump and no phone. The run ended "pair bench: ok". Then the firmware with RANDID in the owner secret was flashed alone, and the bench chip's owner session did not open (`BadSession`). The firmware now tries the older secret too, and the bench passed again on it (section 2).
- iOS: the pairing screens' model, a blank name, a second tap before the press, the card circle's row and "your card" in People, as unit tests. The simulator has no NFC, so no tap has been tried from the app, and the walk only shows the "Pair a card" link.

Not yet tried: a real tap from the app on a phone, and a chat read by tap on the bench. A channel member key under a device grant is not built.

## Constraints

- The chip's policies are fixed at object creation, so slot policies are written when a circle is kept, and circles kept before that cannot be paired. As built, the first boot of this firmware wipes them instead.
- The chip serves one interface per boot. The badge cycles the enable pin to speak to it and leaves the pin low otherwise.
- The badge has no clock and no log position. Device grants carry neither, and revocation is the slot and the generation.
- The inbox and the public object are small. Everything a phone needs at pairing fits in a few kilobytes; nothing larger goes through the chip.
- The sender byte in chat is the seat, so one chatting device per seat, and so one phone per badge.
- Any session can make an object at an empty id, so the badge never leaves an antenna-facing id empty and never uses an object there it did not make.
- The chip's memory, not the Pico's 16 places, sets how many circles a badge keeps in it: about seven on an empty SE051, by estimate. The bench chip held six before it refused a seventh.
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

Calls made after the builder's review of 2026-10-31. The ones on the slot's session, four slots and the revocation at unpair are superseded by the calls made while building, below:

- A generation is named by a random id, not a counter, because the chip computes an HMAC over any input and its counters can be written as well as incremented, so a counter could not have stopped a paired phone asking for future keys early. The random id is the lockout.
- Pairing is per badge, with one inbox and four badge-wide slots, and a public object per circle, because a person taps and presses once and gets every circle, and because one circle's manifest and card fill a public object on their own.
- The slot's session is the chip's EC key session against the phone's enclave key, built now although the `se05x` crate lacks it, because a slot only Face ID can open is the property the card promises. SCP03 over AES keys is the fallback if the chip's EC session cannot be made to work on the bench.
- Hello words for a device come from a stored extract step and a tapped expand step, so no words change for any device.
- The host types the badge count and the offer carries it, so joining phones never pick a badge's seat and nothing new travels on the channel.
- Unpairing produces a signed revocation and a bump that another device publishes, so channel access ends too, and the screens say when.

Calls made while building, 2026-10-31. Each replaces the call above it that it names:

- The slot's session is SCP03 over an AES key, not the EC key session, because the applet's EC session is undocumented here, its key model is not what this spec assumed, and nothing on the bench could prove it before the build. The EC session stays the intended upgrade.
- One phone per badge, not four, until the chat separates senders per device.
- The device grant carries no channel detail and nothing verifies it yet. Channel management from a card circle is deferred. Unpairing signs no revocation of it.
- A badge's bump at unpair is forwarded by the next phone that taps, and the chat crate lets a device forward a seat-signed bump.
- Pending bumps are a list of at most four, with the collapse rule and the cap of section 6.
- The first boot of this firmware wipes old circles and the chip, since their objects would be reachable by a tap.
- The owner secret derives from the board's unique id so a reflash recovers it, with the risk written down in section 2.
- The badge never uses an object at an antenna-facing id that it did not make in the same conversation, and never leaves one of those ids empty.
- Pairing opens the page first, then the tap, then the press: a fresh nonce and an ephemeral badge key in the pairing object, an inbox signed by the phone and sealed to that key, and words over all of it.
- A public object leaves 1 KB of room, and a keep the chip refuses for memory fails closed with "the chip is full. Forget a circle first."
- No `se05x` crate in the phone app. Its APDUs are encoded by hand, so the LGPL library stays in the firmware only.

Calls made at the final review, 2026-10-31. Each replaces the call above it that it names:

- Public objects are read by the owner and the slot only, and a phone reads only the circle slots the pairing object names, so a stranger's tap learns no circle's manifest, card or phone. The first tap shows the badge's name and how many circles it holds, not their names.
- The pairing object is made at the first boot and kept made, so its id is never empty. It replaces deleting and making it fresh at each `pair a phone`.
- The owner secret mixes in the RP2350's factory random id from OTP, and an untrusted USB host retires the badge. It replaces the derivation from the board's unique id alone, which the firmware still tries for a chip owned before 2026-11-01, the bench chip.
- A device that reads two bumps from one generation before a tap waits for the lower, as a root holder does.
- A read of the badge's own public object that fails stops the change, and a missing antenna id is flagged before the remake deletes anything.
- Six pairing words, not four.

## Corrections to earlier documents

- The iOS app design says the bench Click board has no antenna. It has one, with a switch; pairing and the tap are tested on it.
- The V1 board notes say the firmware keeps the contactless interface disabled. The firmware has never touched it; the interface was starved by the enable pin being held high, which boots the chip onto I2C.
- The roadmap's E1 row says scoped enrollment replaces the tap. It is the tap. The row and product row 3 are rewritten to this design.

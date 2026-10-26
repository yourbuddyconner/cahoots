# Attested readings

*A survey, written 2026-10-26, before any design. It lists what a network of attested sensor data could be used for, what the badge's key store chip gives us towards it, and what has to be decided before V1's next board spin. Nothing on this page is built.*

## 1. The primitive

The NXP SE051 planned for V1 ([roadmap M4f](plan.md)) has a second I2C bus on which it is the controller. The badge's microcontroller can ask the chip to read a sensor on that bus. The chip does the read itself and returns the bytes together with a signature over them, a counter from inside the chip, and a nonce the badge supplied. The signing key lives in the chip and never leaves it. NXP calls this an attested read.

What the signature proves: these bytes came over the chip's own wires from a device at this address, in answer to this nonce, after every reading with a lower counter. The microcontroller between the sensor and the world never had a chance to change them, reorder them or replay them.

What it does not prove: that the device at that address is a real sensor rather than something pretending to be one, that the sensor measured the truth, or what the wall-clock time was. The chip trusts its wires. Everything on this page inherits that limit, and the answer to it is physical: the sensor and the chip in one sealed unit.

## 2. What Cahoots adds

Three things turn a signed reading into something a group can act on.

**The nonce binds the reading to the circle.** The badge sets the nonce to a hash of the circle id and the position in the circle's message log. The attestation then says "taken for this circle, after this message". A reading cannot be lifted out of one circle's log and presented in another, and the circle's log gives every reading a place in time that the chip alone cannot.

**The circle meets the chip.** At the ceremony each badge's chip presents its public key and the chip's own proof that the key lives inside it. The circle pins those keys the way it pins the check words. From then on a reading is verified against a key the members saw in the room. NXP's certificate chain is used once, at the ceremony, and never again. That is the only place a manufacturer's root appears, and the circle can refuse it and pin on sight instead.

**The pad carries it.** A reading sealed with the pad is confidential to the circle. A reading sealed with the pair key from the sealed reveals ([protocol §3.3](protocol.md)) goes to one member. A reading posted in the clear with its signature is public and still attributable. The signature also gives the circle something the pad cannot: a member cannot forge another member's readings, because only that member's chip holds the key.

## 3. Use cases

Each family names who would use it, what plugs in, what the attestation proves, where today's answer falls short, and the honest hole. "Private" means sealed to a circle; "public" means posted in the clear with the signature.

### Environment and citizen science

Air quality (particulates, CO2, volatile compounds), radiation (a Geiger tube with an I2C front end), noise, weather, water quality, soil. Community networks like Safecast, PurpleAir and Sensor.Community exist and are useful, and their weakness is provenance: anyone can post a number from anywhere. A circle of attested nodes is the version where each reading is from a known chip that the other publishers met. Mostly public, with the circle as the set of publishers who vouched for each other. The hole: a heat lamp next to the thermometer. Attestation proves the chip's wires, not the air.

### Energy, water and shared utilities

Current and voltage on a shared solar array, a battery's state, a well's flow meter, a tank's level, a meter on a shared line. Cooperatives settle bills and harvests on these numbers, today from someone's spreadsheet or a vendor's cloud. Attested readings make the settlement a matter of record that no single member controls. Private, with non-repudiation. The hole: the meter itself, which is why utilities seal theirs.

### Agriculture

Greenhouse climate, soil moisture, cold storage, irrigation, livestock tags read by an I2C RFID reader. The uses are a cooperative's shared records, a buyer's conditions ("kept below eight degrees"), and parametric insurance, where a payout triggers on a reading rather than a claim. Private, sometimes shared with one outside party by pair key. The hole: liveness. The node's owner decides when readings are taken and can switch it off. Gaps show in the counter, and the circle can expect a heartbeat, but silence is not a reading.

### Cold chain, track and trace

A crate carries a node: temperature, a light sensor that sees the lid open, an accelerometer for shocks, GPS for where, a real-time clock for when. Custody changes hands as a transaction in the circle: the node posts its attested state, the receiving member approves on their badge, and both are in the log. The whole journey is a chain of readings and approvals that no party could have written alone. Private to the parties in the chain, with a reading or two posted in the clear when a regulator wants proof. The hole: GPS can be fooled from the sky, and no chip can tell. The mitigations are in section 4.

### Counterfeits and provenance of things

The chip's identity is the item's identity. A tap over NFC, with the antenna on, reads the chip's attestation of its own key, and the circle that sealed the item can say whether this is the crate, the bottle or the part they know. The chip runs on the reader's field, so this works with no battery. For small producers, art, spare parts, and anything that gets copied. Public verification of a private seal. The hole: the chip proves itself, not what it is glued to. A node potted into the item closes most of that.

### Seals, custody and audit trails

A reed switch on a door, a light sensor inside a box, a tamper line across a lid. "Not opened between these two messages" as a signed fact: evidence lockers, cash boxes, equipment cages, a ballot box, a cabinet of controlled substances. The log is the audit trail, and the auditor can be a member of the circle or someone handed the pinned keys and the clear-text readings. Private. The hole: again, the bond between switch and chip.

### Health

Pulse, oxygen, temperature from wearable front ends; a bed sensor or motion for someone living alone; the device readings in a clinical trial, where a signed raw reading is worth more than a value typed into a form. The pad matters here more than anywhere, because the readings are the most private data a person has and today they go through a vendor's cloud. Private, to a circle of family or a clinician, or to one person by pair key. The hole: the sensor's own accuracy, which no signature improves.

### Vehicles, fleets and deliveries

GPS and an accelerometer in a vehicle, an I2C bridge to the engine's diagnostics, a door switch on a van. Attested trip logs for mileage, proof of delivery as a position plus a door opening at a counter value, a drone's flight log. Private between the operator and the customer, with a reading posted in the clear when there is a dispute. The hole: GPS again, and the node's owner controls the power.

### Industry, buildings and infrastructure

Vibration on a pump, strain on a beam, pressure in a pipe, temperature in a server room, a tank level at a depot. The readings feed maintenance today; attested, they also settle warranty and liability disputes, where the question is always "what did the sensor say before it failed". Compliance logs are the quiet win: food safety rules require temperature logs that are mostly clipboards, and an attested log is a clipboard nobody can backfill. Private.

### Science and the field

A freezer's log, an incubator, an instrument's output, acoustic and camera traps in the field. Reproducibility starts with raw readings that are known to be raw. Large data goes by digest (section 4). Private to the group, public when published.

### Journalism, witnesses and human rights

A photo or a recording with a signed digest, a position, a counter and a place in a circle's log, from a chip the circle met in person. This is the problem C2PA tries to solve with a certificate authority at the top; here the root is the room. A witness network is a circle of such nodes. Private until the holder decides, then public with the signature. The hole: a photo of a screen, which no chip can tell from a scene. Attestation says who took it and when, not what it depicts.

### Commerce between members

A scale front end, a people counter, a flow meter on a pump, an hour meter on a shared tool, a dispenser. A signed scale reading is a receipt the scale wrote. Usage of shared things, settled from attested counts, in a tool library, a market, a workshop. Private with non-repudiation.

### Disaster response and offline operation

River levels, shelter occupancy, radiation after an incident, generator fuel. Networks are down and the readings still have to be trusted when they arrive. Nodes keep attested logs and sync when a link appears, over whatever carries the circle's messages. This is where the project's offline direction and this page meet. Public or private by situation.

### Proof of meeting

Two badges tapped together, each reading the other's chip over NFC with a nonce from its own log, give each a signed statement that the other chip was within a few centimetres at that point. No GPS, no clock, no server. Attendance, a handover, a two-person rule, a meeting that both sides can later prove happened. This is not a sensor use, but it falls out of the same parts and it is the mitigation for the GPS hole in the deliveries above.

### Attested input

A keypad, a switch panel, a card reader or a fingerprint module on the chip's bus. The chip signs what a person entered, and the firmware could not have entered it for them. A vote cast on a keypad, an approval that must come from a physical act, a count made by hand. The badge's own buttons are on the microcontroller, so they are not this. An attested keypad on the chip's bus would be.

### Attested time

The chip's counter orders readings but knows no wall-clock time. A real-time clock on the chip's bus gives attested timestamps, with the caveat that the clock can be set through the same bus by the node's owner. So an attested timestamp from a node says "the chip read this from a clock its owner controls": good enough for a log, useless as the clock a grant expires on. The clock that no single party controls is the circle's log, and section 4 says how to use it.

### Delegation grants and agents

The chip signs a grant, after both buttons on a screen that shows it: this key may ask for these things in this circle, up to this many bytes, until this position in the log, and may hand a narrower grant onward. One format covers three needs: a laptop joins the badge's trusted list with a scope instead of blanket trust, a screenless node receives its policy at the ceremony, and a program acting for a person carries proof of what it may do. The badge is the root of its owner's agent keys, and the consent screen is the only place a grant is born. Revocation is a message in the log.

### Cameras

A camera module hashes each frame or clip and exposes the digest as a register; the chip signs the digest with the log nonce; the footage is bulk-encrypted under keys the pad derives per period, since video would spend a pad in an hour. The attestation is what makes a clip count as evidence. What makes such a network different from a commercial one is the governance the design gives for free: footage opens only with the approval of several members on their badges, the keys of a period are destroyed when it ends so retention is enforced by the design rather than by a policy, and there is no operator holding an archive. A camera network the people under the cameras could consent to. It needs the approvals work, the node, the digest module and the period keys, so it comes last.

### Agents acting on readings

The hosted seat ([roadmap M3](plan.md)) and the approvals work give a program a policy over a circle's messages. With attested readings as input, the policy can act on the world: release a payment when the scale and the position say delivered, open a valve when the tank says empty, raise an alarm when a seal says opened. The policy acts on readings that no single member could have forged, which is the difference between an automation and a trap.

## 4. Patterns that recur

**Digests for big data.** The chip's bus is slow and its attested read is small. A camera, a microphone or any module with data to spare takes its own reading, hashes it, and exposes the hash as a register. The chip signs the hash with the nonce; the data travels separately over the pad. The claim is "this chip read this digest from that module at this point in the log", which is as much as any camera system honestly claims.

**Co-location against spoofing.** Where one reading can be faked at the source, GPS above all, two chips attesting each other's proximity is the check. A delivery is a position plus a tap.

**The sealed unit.** Every hole on this page is the bond between a sensor and the chip. The node is the sensor and the chip in one potted or sealed package, built so that opening it is visible. That is a hardware design question for a node, not a protocol one.

**Heartbeats.** The host controls when readings happen. A circle that expects a reading every interval sees silence as information, and the chip's counter shows how many readings were taken but never posted.

**Time is the log.** Anything that must expire or come into force, a grant, a seal, a period key, is bound to a position in the circle's log or to a chip's counter, which no grantee can move. Wall-clock time is the verifier's own clock, used as an outer bound and never taken from the party that benefits. Revocation is a message in the log.

**Pinned in the room.** The manufacturer's certificate is used once, at the ceremony, to check that a key is in a chip. After that the circle trusts the keys it pinned. A circle may skip the manufacturer entirely and pin on sight.

## 5. What has to be decided before designing

- **A second bus on V1.** The Qwiic socket is on the Pico's I2C0 today. A sensor that the chip attests must hang off the chip's own master bus, or the Pico is back in the middle. Either the socket moves to the chip's master pins or V1 gets a second socket marked for attested modules. This is the one item that has to land before the board spins.
- **A node variant.** Most uses above want no screen and no buttons: a node that is sealed, battery- or solar-powered, and posts on its own. That is V1 without the display, and a different enclosure.
- **Spends without a button.** A node cannot press both buttons for every reading. The circle grants it a policy at the ceremony: which readings, how often, for how long. That is the manifest from [open decision 13](plan.md), with a new kind of entry.
- **Pad budget.** A reading a minute at a hundred bytes is 140 KB a day, and a megabyte pad lasts a week. Readings should be sealed under keys the pad derives for a period, the way tunnel keys are, with the pad spent per key rather than per byte. The chat spec already has the per-sender key schedule to borrow.
- **Module descriptions.** The badge needs to know, per module, the address, the registers, the byte count and how to turn bytes into a value, so the consent screen and the circle can show a reading as a reading. A small catalogue, versioned, shipped with the firmware.
- **Public mode.** Posting readings in the clear with a signature is a different transport from the circle's feed and needs its own rules on what a public reading carries.
- **NFC proximity.** Proof of meeting needs the contactless side on and a defined exchange between two chips. It also needs an answer to what a stranger's tap may read, which is open decision 16.
- **Verification off the badge.** Members verify readings on phones and laptops, so the attestation format and the pinned-key list must be readable there. The app's wasm core is where that lands.
## 6. A first order

Not every family on this page needs building. The order that each step makes the next cheaper:

1. **Delegation grants with the chip's signer.** Reuses M4f and nothing else, and is the trusted list, the node policy and the agent identity in one format.
2. **Proof of meeting.** A tap and a nonce; the fix for position spoofing everywhere else.
3. **Readings, with the counter and the log as their time.** Cold chain and track and trace first, since it exercises grants, custody approvals and seals together; public environmental monitoring second, since the existing networks have exactly the gap this fills. A real-time clock joins for logging, under the caveat above.
4. **The camera node**, once approvals and period keys exist.

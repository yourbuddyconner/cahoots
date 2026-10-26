# Cards for every gathering

*A concept, written 2026-10-21. The parts marked built exist today. The rest is a direction, not a plan.*

## 1. The picture

Everyone carries a card. When a group of people meets, for dinner, a project, a board meeting, a protest, they touch cards and make a pad. Five minutes, and the group has a secret that nobody outside the room has seen. Do that for years and a card holds hundreds of circles: one for every room you were ever in with those people.

Two facts make the rest of this page possible. Each circle is a channel that only that room can read, for as long as its pad lasts. And each card holds a key with every person it has ever shared a room with, because the ceremony's sealed reveals leave every pair with a key that only those two can compute ([protocol §3.3](protocol.md)).

## 2. Does a card have room?

The Baochip-1x in V2 has 4 MiB of RRAM on chip, and the Pico 2 on V1 has 4 MB of flash ([Build a badge](diy-badge.md)), with the firmware in part of it. Call it 3 MiB for pads.

| Pad size per gathering | Pads in 3 MiB on-chip | Pads in a $1 16 MiB flash chip | Pads in a $4 128 MiB flash chip |
|---|---|---|---|
| 256 KiB, the app's "large" | about 12 | 64 | 500 |
| 16 KiB | about 190 | 1,000 | 8,000 |
| 4 KiB | about 770 | 4,000 | 32,000 |

Three conclusions.

- **Size the pad at the ceremony.** The card should ask how long the group will last. A dinner gets 4 KiB, which is still a few dozen unbreakable messages per person, or 32 fresh session keys. A team that will chat for a year gets 256 KiB. The app already offers three sizes; the card should ask the same question.
- **A card for every gathering wants an external flash chip.** That is a line item, and the design stays the same. V2 on the Baochip-1x is planned to store pads encrypted under a key that never leaves the chip's key store, so more flash is more ciphertext. The ESP32-S3 firmware today writes the pad to flash in the clear, and V1 on the Pico 2 will do the same. The circle IDs, the ledgers and the pair keys stay on-chip. They are a few hundred bytes per circle.
- **Old pads need refreshing.** Flash keeps data for ten to twenty years. A pad that must still work in 2040 has to be rewritten in between. The firmware should rewrite stored pads every few years as a background job. Boring, and real.

## 3. A message from years ago

A message arrives on the feed for a circle made in 2027. Nobody has used it in years. What can the card say?

- **The seal verifies, so the sender holds that pad.** This is unconditional: no computer can forge it.
- **The bytes came from seat 3's region of the pad.** That is only a claim. Everyone in the room holds the whole pad, and anyone with a copy could spend from seat 3's region.
- **Which member it is can be settled by the pair key.** Hello words do it live, on a call (built). For a written message, the same key can add a sixteen-byte tag per recipient, so that you, specifically, know seat 3 wrote it. Nobody else in the room can forge that tag. It is as strong as X-Wing rather than unconditional. This is the sender proof that the federation profile ([use cases §5.2](use-cases.md)) promised, without the extra touches it was going to cost. *Proposed: a `tags` extension to the message format.*

So the mystery has a shape. A message that verifies under the pad came from someone in the room, or from someone who took a card. A message that also verifies under a pair key came from that member, or from whoever holds that member's card and knows its PIN. The more rooms you shared with someone, the more cards an impostor would need.

Two small features make dormant circles less ambiguous ahead of time.

- **Expiry.** A circle can carry a date after which the card erases it, unless the owner marked it keep. Casual circles expire. Kept ones are the ones a message can still come from.
- **Proof of life.** A kept circle can carry a daily sealed counter from each member ([use cases §7.4](use-cases.md)). Silence then means something.

## 4. The web of trust a card accumulates

A card is a graph. Each room is a group of people, and each person you have shared a room with is a pair key. Three things fall out, with no server, no directory, and no long-lived identity key.

**Identity continuity.** When a "Bob" shows up in a new circle, he can prove he is the Bob from your 2027 circle with one tag under that old pair key, over the new circle's ID and his seat. Your card then carries your name for him into the new circle on its own, and can tell you "seven rooms since 2027". Names stay local to your card. There is no global username.

**Introductions.** Alice met Bob in one room and Carol in another. Bob and Carol have never met. Alice can vouch to Carol for Bob's key in a new circle, under the key that only Alice and Carol hold. Carol trusts Bob's key because she trusts Alice, and she met Alice. This is the PGP web of trust with in-person edges only. There is nothing to scrape, because no edge exists outside the two cards that hold it.

**Compromise becomes quantitative.** One old circle answering is a mystery. Someone who answers under pair keys from several old circles would have had to steal several cards. Three is a person.

What it is not: there is no identity that outlives the cards, no reputation score, and no way to look someone up. You can only reach people you have met, or people they vouch for.

## 5. A social network with no accounts

The idea is not fully formed. This is the shape it would have if every piece above existed.

| In a social network | In cahoots |
|---|---|
| Account | None. A card, and the rooms it has been in. |
| Friend | Someone you have shared a room with. The edge is a pair key. |
| Group | A circle. Made in person, sized at the ceremony. |
| Post | A message to a circle on the feed. Unbreakable when short, hedged when long. |
| Follow | Read a circle's feed. Only its members can. |
| Profile | Nothing. Each member keeps their own names for the others. |
| Discovery | Introductions by people you have met. There is no search. |
| What the server sees | Circle IDs and ciphertext that looks like noise. It cannot tell who posted, or to whom. |
| Moderation | The room. Make a new circle without someone, and they are out. |
| Delete | Erase the pad bytes. After that no transcript can prove what was said ([use cases §7.3](use-cases.md)). |

The things it cannot do are the point of it. No stranger can message you. Nobody can scrape the graph, because the graph is in the cards. No company holds the accounts, because there are none. Spam is impossible for the same reason discovery is hard: you can only be reached through a room you were in.

Open questions:

- **Reach.** Everything above is bounded by rooms you were in. Is that the product, or does trust need to extend further than one introduction?
- **Scale.** A circle is the people who were in a room. A group bigger than a room needs several circles, or a structure of sub-ceremonies, and each circle's feed is a public log that grows.
- **The feed.** The Numbers Station is a bulletin board, one circle at a time. A card in three hundred circles needs one request that covers all of its IDs.
- **The phone.** The card holds the pads and the phone shows the feed. The USB link between them (roadmap M4d) is what makes this usable without putting pads on a phone.
- **Joining later.** Today a circle cannot be joined after the ceremony, and that is a feature. A social network may want a way for the room to add someone, which is a new ceremony with the old pad as one input.

## 6. In what order

1. Per-recipient sender tags. Small, and everything above needs them.
2. Pad size chosen at the ceremony, the external flash layout, and the refresh job.
3. Expiry, keep, and proof of life for circles.
4. Identity continuity in the app: the "same Bob" proof, and names that travel.
5. Introductions.
6. Only then, experiments with the social layer, on top of what the first five made true.

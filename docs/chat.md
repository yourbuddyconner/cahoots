# Chat by generation, version 3

End-to-end encrypted group chat for circle members. The circle root (`docs/protocol.md` section 3.6) seeds the keys, and a standard AEAD encrypts the messages. The security claim is computational. The UI MUST NOT call it "unbreakable". Pure-pad text (`docs/protocol.md` section 4) stays available for the lines that deserve the strongest claim.

All integers are little-endian unless a line says otherwise. HKDF is HKDF-SHA256 (RFC 5869). HMAC is HMAC-SHA256. AEAD is ChaCha20-Poly1305 (RFC 8439).

## 1. Generations and epochs

A **generation** has a 32-byte id and a key. Generation 0's id is derived from the circle id, so every circle starts in it and no member has to do anything. Every later generation has a random id, named by a signed bump (below). A member who holds the root derives the key of any generation from its id:

```
gen0_id        = SHA-256("cahoots/chat/gen0" || circle_id)
generation_key = HMAC-SHA256(root, "chat/gen" || circle_id || generation_id)
```

A device that holds one generation key and not the root can chat in that generation. It cannot derive the next one, so a bump leaves it behind until someone gives it the next key. This is how a device is revoked: bump, and do not give it the key.

An **epoch** is a numbered period inside a generation, with its own key. Epochs are numbered from 1. Whoever holds the generation key derives the key of any of its epochs, and starting one spends nothing:

```
epoch_key = HMAC-SHA256(generation_key, "chat/epoch" || n as u32 big-endian)
tag       = the first 2 bytes of SHA-256(generation_id)
epoch_id  = 0xFE (u8) | tag (2 B) | n (u16)
```

The tag lets a receiver pick the generation without the id on the wire. Two generations a session holds could share a tag by chance, one time in 65536. A receiver then tries each, and the tag check of the message decides.

**Bump.** A member moves the circle to a new generation by sending a message of type `2` (bump) in the current epoch. Its plaintext is:

```
payload = from_id (32 B) || to_id (32 B) || COSE_Sign1
signed  = "cahoots/chat/bump/v1" || circle_id || from_id || to_id
```

The COSE_Sign1 is a detached ES256 seat signature over `signed`, with kid `[seat]` and empty external AAD. A receiver MUST verify it against the seat keys of the circle's manifest before it moves. A bump that does not verify is ignored, and the chat continues in the generation it was in. `from_id` is the generation the bump moves from, and the sender seals the bump in that generation. A receiver moves only when the bump was sealed in `from_id` and `from_id` is its current generation. There is one exception, for two members who bump at the same time. If the receiver's current generation was itself entered from `from_id` by another bump, the lower of the two new ids wins. If `to_id` is lower, the receiver moves to it and keeps the generation it leaves, so nothing sealed there is lost. A member whose own unposted bump lost does not post it. If the receiver's id is lower, it stays, and a root holder keeps `to_id` too, so what the other member sent before it followed still opens. Any other bump is stale and is ignored. A member who holds the root then derives the key of `to_id` and moves. Its epoch 1 starts at once. A device without the root has to fetch the new key. Until it does, a message in the new generation is refused as `NeedsGeneration`, with the id.

A receiver keeps the keys of the current generation and the one before it, so a message sent by a member who had not yet read the bump still opens. A message from two or more generations back is refused as `NeedsGeneration`. A bump read again later, or sealed again by a member in a later generation, names a generation the circle has left, so it moves nobody. A session never moves back to a generation it has held or dropped.

**Root epochs, the old form.** Before generations, the epoch key came from the root directly, and the epoch id began with `0xFF`:

```
epoch_id  = 0xFF (u8) | n (u32)
epoch_key = HMAC-SHA256(root, "chat" || circle_id || n as u64 big-endian)
```

This is the chat key of `docs/protocol.md` section 3.6. No new message uses a root epoch. A receiver that holds the root still opens them, so old history reads. A chat state saved before generations imports as generation 0, with its root epochs kept for reading.

Pad epochs are retired as of 2026-10-30. In version 1 of this document, an epoch was keyed by 32 bytes spent from a member's slice, and its id was the slice and offset of that spend. A receiver MUST refuse an epoch id whose first byte is not `0xFE` or `0xFF`, as `PadEpoch`, before it checks the tag. The pad is now spent only by pure-pad messages.

There is no separate rotation message. Every chat message carries its `epoch_id` in the header. A member starts a new epoch by sending in epoch `h + 1` of the current generation, where `h` is the highest epoch of that generation it has seen or started. A receiver that sees an unknown epoch `n` of a generation it holds:

1. refuses it as `UnknownEpoch` if `n` is 0, or if `n` is at or below an epoch of that generation whose state it has dropped,
2. derives `epoch_key` and the sender chains below, and keeps them only if the message decrypts.

There is no bound on how far ahead `n` may be. Each attempt costs one HMAC, and a message whose tag fails installs nothing. A member who missed many epochs reads the latest one directly.

A member SHOULD start a new epoch after `ROTATE_AFTER = 1000` of its own messages in the current epoch, or after 24 hours, whichever comes first. The time rule is not implemented. `cahoots-chat` rotates on count only. Members SHOULD switch to the newest epoch they have seen. Two epochs MAY be live at once while members switch. Messages from older epochs still open if their epoch is above the floor, that is, above every epoch of that generation whose state this member has dropped. A receiver keeps at most `MAX_EPOCHS = 4` epoch states per generation and drops the lowest. It does not derive a dropped epoch again, because fresh chains would accept the old messages a second time.

## 2. Sender chains

Each sender has its own chain inside an epoch. Chains advance one step per message, and each step is deleted once used. This bounds what one leaked message key reveals. It does not protect old messages from a device that still holds the root (section 5).

```
chain[i][0]      = HKDF-Expand(epoch_key, info = "cahoots/chat/chain/v1" || sender i (u8), 32)
msg_key[i][n]    = HKDF-Expand(chain[i][n], info = "cahoots/chat/msg/v1", 32)
chain[i][n+1]    = HKDF-Expand(chain[i][n], info = "cahoots/chat/next/v1", 32)
```

After computing `msg_key[i][n]` and `chain[i][n+1]`, the implementation MUST zeroize `chain[i][n]`. After decrypting message `n`, it MUST zeroize `msg_key[i][n]`.

Out-of-order delivery: a receiver that gets counter `n` while expecting `m < n` advances the chain to `n`, keeping the skipped message keys for `m..n` in a bounded cache (`MAX_SKIP = 256` per sender per epoch). Beyond that bound, the message is rejected as `TooFarAhead`.

## 3. Message format

```
header = 2 (u8) | circle_id (16 B) | epoch_id (5 B) | sender (u8) | counter (u32) | len (u32) | type (u8)
nonce  = sender (u8) | counter (u32) | 0x00 * 7         // 12 bytes; unique per message key anyway
ct     = ChaCha20-Poly1305(key = msg_key[sender][counter], nonce, aad = header, plaintext)
message = header || ct                                  // ct includes the 16-byte AEAD tag
```

The first byte is `2` in every chat message, the version the relay checks. It distinguishes chat messages from pure-pad messages (version `1`) and ceremony messages (version `3`) on the same feed. A client reads the chat version from the epoch id instead: `3` for a `0xFE` epoch, `2` for an old `0xFF` root epoch. Do not write `3` into the first byte. The relay would read the message as a ceremony message and refuse it.

`type`:

| Value | Type | Plaintext |
|---|---|---|
| `0` | text | The message text |
| `1` | control | Reserved |
| `2` | bump | `from_id`, `to_id` and a seat's signature, section 1 |
| `3` | card | A seat-signed contact card |

`len` is the plaintext length. Plaintext SHOULD be bucket-padded to 64, 256, 1024, or 4096 bytes.

The `sender` field is a claim. Any member can compute any sender's chain. It identifies a chain, not a person, as slices do for pure-pad messages.

## 4. Your own sending position

Every key depends only on the generation, the epoch, the sender and the counter. A device that sent from a counter and then lost or rolled back its chat state would send from that counter again, under the same key and nonce. That leaks the XOR of two plaintexts and allows forgeries. Two rules prevent it:

- A device writes its chat state before it posts a message, and never rolls it back.
- After a restart or a restore, a device reads the relay to the end before its first send. It feeds its own messages to its session too. The session does not decrypt them. It checks the tag and moves its own chain past each one. A session refuses to send with `NotSynced` until the caller marks a full read done (`mark_synced`).

## 5. Security properties

- **Chat is as secret as the root.** Confidentiality and integrity are computational, from ChaCha20-Poly1305 under keys derived from the circle root. The root comes from the content keys of the ceremony's reveals, which carry the pad contributions.
- **Any member's device reveals everything.** Every member's device holds the root, so whoever compromises one member's device can read every message the relay still holds, past and future. The relay keeps all ciphertext.
- **No forward secrecy and no post-compromise security.** Chain steps bound what one message key reveals and nothing more. A new epoch does not help against an attacker who has the root. A new ceremony makes a new root.
- **Not provided**: sender authenticity among members, and protection against a member who leaks the root or an epoch key.

**Considered and set aside.** A ratcheted root would restore forward secrecy for software members: `root_{n+1} = HMAC(root_n, "cahoots/chat/next")`, with old roots deleted after every member has caught up. It was set aside because a card's chip cannot follow it: the chip cannot chain HMACs inside itself, so a paired phone would have to hold the ratchet state, could then derive every later epoch, and unpairing would no longer revoke future taps. It stays recorded here in case the badge can keep the chip's root advanced cheaply.

## 6. Cost

An epoch costs nothing. The pad is not spent by chat.

## 7. The relay

Chat messages travel over the Numbers Station: an append-only, hash-chained public log that stores ciphertext, orders it, and refuses pure-pad range reuse. It does not check the epoch id of a chat message, so generations change nothing for it. Replay protection lives in each member's chains. The relay's API and log format are in `crates/station/README.md`.

## 8. Vectors

`crates/vectors/chat.json` covers epoch 7 of generation 0 for the root and circle of `crates/vectors/root.json`: the generation id and key, the epoch id and key, chain and message keys for two senders, three encrypted messages including one out of order, and a tampered message that MUST fail. Its generation key equals the one `root.json` publishes for generation 0. It covers a bump from generation 0, signed by seat 1 of the ceremony in `crates/vectors/ceremony.json`, with a tampered bump that MUST fail. Under `root_epochs` it keeps the same set for root epoch 7, the old form, whose epoch key equals the chat key `root.json` publishes for epoch 7.

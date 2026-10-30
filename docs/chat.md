# Chat on root epochs, version 2

End-to-end encrypted group chat for circle members. The circle root (`docs/protocol.md` section 3.6) seeds the keys, and a standard AEAD encrypts the messages. The security claim is computational. The UI MUST NOT call it "unbreakable". Pure-pad text (`docs/protocol.md` section 4) stays available for the lines that deserve the strongest claim.

All integers are little-endian unless a line says otherwise. HKDF is HKDF-SHA256 (RFC 5869). HMAC is HMAC-SHA256. AEAD is ChaCha20-Poly1305 (RFC 8439).

## 1. Epochs

An **epoch** is a numbered period with its own key. Epochs are numbered from 1. Every member holds the circle root, so every member can derive the key of any epoch, and starting one spends nothing:

```
epoch_id  = 0xFF (u8) | n (u32)
epoch_key = HMAC-SHA256(root, "chat" || circle_id || n as u64 big-endian)
```

This is the chat key of `docs/protocol.md` section 3.6. The first byte of the epoch id is `0xFF` for every root epoch.

Pad epochs are retired as of 2026-10-30. In version 1 of this document, an epoch was keyed by 32 bytes spent from a member's slice, and its id was the slice and offset of that spend. A receiver MUST refuse an epoch id whose first byte is not `0xFF`, as `PadEpoch`, before it checks the tag. The pad is now spent only by pure-pad messages.

There is no separate rotation message. Every chat message carries its `epoch_id` in the header. A member starts a new epoch by sending in epoch `h + 1`, where `h` is the highest epoch it has seen or started. A receiver that sees an unknown epoch `n`:

1. refuses it as `UnknownEpoch` if `n` is 0, or if `n` is at or below an epoch whose state it has dropped;
2. refuses it as `EpochTooFarAhead` if `n` is more than `MAX_AHEAD = 64` past the highest epoch it has seen;
3. derives `epoch_key` and the sender chains below, and keeps them only if the message decrypts.

A member SHOULD start a new epoch after `ROTATE_AFTER = 1000` of its own messages in the current epoch, or after 24 hours, whichever comes first. The time rule is not implemented. `cahoots-chat` rotates on count only. Members SHOULD switch to the newest epoch they have seen. Two epochs MAY be live at once while members switch. A member who was offline reads the epochs it missed in order. A receiver keeps at most `MAX_EPOCHS = 4` epoch states and drops the lowest. It does not derive a dropped epoch again, because fresh chains would accept the old messages a second time.

## 2. Sender chains

Each sender has its own chain inside an epoch. Chains advance one step per message, and each step is deleted once used, which gives forward secrecy inside the epoch.

```
chain[i][0]      = HKDF-Expand(epoch_key, info = "cahoots/chat/chain/v1" || sender i (u8), 32)
msg_key[i][n]    = HKDF-Expand(chain[i][n], info = "cahoots/chat/msg/v1", 32)
chain[i][n+1]    = HKDF-Expand(chain[i][n], info = "cahoots/chat/next/v1", 32)
```

After computing `msg_key[i][n]` and `chain[i][n+1]`, the implementation MUST zeroize `chain[i][n]`. After decrypting message `n`, it MUST zeroize `msg_key[i][n]`.

Out-of-order delivery: a receiver that gets counter `n` while expecting `m < n` advances the chain to `n`, keeping the skipped message keys for `m..n` in a bounded cache (`MAX_SKIP = 256` per sender per epoch). Beyond that bound, the message is rejected as `TooFarAhead`.

## 3. Message format

```
header = version (u8 = 2) | circle_id (16 B) | epoch_id (5 B) | sender (u8) | counter (u32) | len (u32) | type (u8)
nonce  = sender (u8) | counter (u32) | 0x00 * 7         // 12 bytes; unique per message key anyway
ct     = ChaCha20-Poly1305(key = msg_key[sender][counter], nonce, aad = header, plaintext)
message = header || ct                                  // ct includes the 16-byte AEAD tag
```

Version `2` distinguishes chat messages from pure-pad messages (version `1`) on the same feed. `type`: `0` text, `1` control (reserved). `len` is the plaintext length. Plaintext SHOULD be bucket-padded to 64, 256, 1024, or 4096 bytes.

The `sender` field is a claim. Any member can compute any sender's chain. It identifies a chain, not a person, as slices do for pure-pad messages.

## 4. Security properties

- **Confidentiality and integrity**: computational, from ChaCha20-Poly1305 with keys an attacker can get only by knowing the circle root. The root comes from the content keys of the ceremony's reveals, which carry the pad contributions.
- **Forward secrecy inside an epoch**: deleted chain and message keys cannot be recovered from a later memory dump.
- **No post-compromise security**: a device whose memory was dumped with the root in it has lost every epoch, past and future. A new epoch does not help. A new ceremony makes a new root.
- **Not provided**: sender authenticity among members, and protection against a member who leaks the root or an epoch key.

## 5. Cost

An epoch costs nothing. The pad is not spent by chat.

## 6. The relay

Chat messages travel over the Numbers Station: an append-only, hash-chained public log that stores ciphertext, orders it, and refuses pure-pad range reuse. It does not check the epoch id of a chat message. Replay protection lives in each member's chains. The relay's API and log format are in `crates/station/README.md`.

## 7. Vectors

`crates/vectors/chat.json` covers the key of root epoch 7 for the root and circle of `crates/vectors/root.json`, chain and message keys for two senders, three encrypted messages including one out of order, and a tampered message that MUST fail. Its epoch key equals the chat key `root.json` publishes for epoch 7.

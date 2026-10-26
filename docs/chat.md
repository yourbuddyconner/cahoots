# Pad-seeded chat, version 1

End-to-end encrypted group chat for circle members, where the pad seeds the keys and a standard AEAD encrypts the messages. This is PSK mode (32 pad bytes become one session key) applied to chat. The security claim is computational, hedged by the pad. The UI MUST label it "end-to-end encrypted, pad-hedged", never "unbreakable". Pure-pad text (`docs/protocol.md` section 4) stays available for the lines that deserve the strongest claim.

All integers are little-endian. HKDF is HKDF-SHA256 (RFC 5869). AEAD is ChaCha20-Poly1305 (RFC 8439).

## 1. Epochs

An **epoch** is a period keyed by one 32-byte pad spend. Any member may start one by spending 32 bytes from its own slice. The epoch is named by the address of that spend, which is public:

```
epoch_id  = slice (u8) | offset (u32)
psk       = PAD_slice[offset .. offset+32]            // spent and erased on first use, by everyone
epoch_key = HKDF-Extract-and-Expand(
              salt = "cahoots/chat/epoch/v1" || circle_id || epoch_id,
              ikm  = psk,
              info = "",
              len  = 32)
```

There is no separate rotation message. Every chat message carries its `epoch_id` in the header. A receiver that sees an unknown `epoch_id`:

1. checks with its ledger that the 32-byte range is unspent, and rejects the message as `Reuse` if it is not;
2. spends and erases the range;
3. derives `epoch_key` and the sender chains below.

A member SHOULD start a new epoch after `ROTATE_AFTER = 1000` of its own messages in the current epoch, or after 24 hours, whichever comes first. The time rule is not implemented; `cahoots-chat` rotates on count only. Members SHOULD switch to the newest epoch they have seen. Two epochs MAY be live at once while members switch. A receiver keeps at most `MAX_EPOCHS = 4` epoch states and drops the oldest.

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

- **Confidentiality and integrity**: computational, from ChaCha20-Poly1305 with keys an attacker can get only by knowing the pad bytes of the epoch. There is no public-key step to break.
- **Forward secrecy inside an epoch**: deleted chain and message keys cannot be recovered from a later memory dump.
- **Post-compromise security at the next epoch**: a device whose memory was dumped is clean again after the next epoch, as long as the pad region was not taken. This comes from the pad, not from a fresh key exchange.
- **Not provided**: sender authenticity among members, and protection against a member who leaks the epoch key.

## 5. Cost

One epoch costs 32 pad bytes for the whole circle, however many messages it carries. A 256 KiB pad is about 8,000 epochs. At one epoch a day that is twenty years.

## 6. The relay

Chat messages travel over the Numbers Station: an append-only, hash-chained public log that stores ciphertext, orders it, and refuses pure-pad range reuse. It cannot refuse chat reuse, because an epoch pointer legitimately appears in many messages; reuse protection for epochs lives in each member's ledger. The relay's API and log format are in `crates/station/README.md`.

## 7. Vectors

`crates/vectors/chat.json` covers epoch key derivation, chain and message keys for two senders, three encrypted messages including one out of order, and a tampered message that MUST fail.

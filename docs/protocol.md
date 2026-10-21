# Cahoots protocol, version 1

This is the normative description of what `cahoots-core` implements. The words MUST, MUST NOT, SHOULD, and MAY have their RFC 2119 meanings. All integers are little-endian.

## 1. Terms

- **Contribution `R_i`:** the random bytes member device `i` generates for a ceremony.
- **Pad:** `PAD = R_1 XOR R_2 XOR ... XOR R_n`.
- **Circle:** the set of members that hold one pad.
- **Slice:** a fixed, non-overlapping part of the pad assigned to one sender.
- **Spent:** pad bytes that were used to encrypt or decrypt a message. They MUST be erased and never used again.

## 2. Parameters

| Name | Default | Notes |
|---|---|---|
| `n` | 2 to 8 members | |
| `L` | 256 KiB | On devices the maximum depends on free non-volatile memory. |
| Slices | `n`, plus 1 if the circle seats an agent | Equal sizes. Slice `k` starts at `k * floor(L / slices)`. |

Every slice MUST hold at least 32 bytes.

### 2.1 Encoding

The parameters are encoded once and bound into every commitment and the check words:

```
params = version (u8 = 1) | n (u8) | L (u32) | agent_seat (u8: 0 or 1) | name_len (u8) | name (UTF-8)
```

Implementation: `core/src/params.rs`.

## 3. Ceremony

Commit-then-reveal. Without it, the last member to reveal could choose a contribution after seeing everyone else's and bias the pad.

1. **Setup.** The leader chooses the parameters. They are sent to every device and shown for confirmation.
2. **Commit.** Each device generates `R_i` and computes `C_i = SHA-256("cahoots/commit/v1" || params || i || R_i)`. Every device sends `C_i` to every other device. A device MUST NOT send `R_i` until it holds all `n` commitments.
3. **Reveal.** Each device sends `R_i`. Each received `R_j` is checked against `C_j` in constant time. A mismatch aborts the ceremony on that device.
4. **Combine.** `PAD = XOR of all R_j`. A device MAY XOR a second independent source into its own `R_i` before committing. That source MUST NOT replace the device's own TRNG output.
5. **Check words.** `check = SHA-256("cahoots/check/v1" || params || PAD)`. The first 66 bits are read as six 11-bit big-endian indices into the BIP-39 English wordlist. Every device shows the six words. Members read them aloud. A mismatch means the ceremony restarts.
6. **Erase.** Each device erases every `R_j` and keeps only `PAD`.
7. **Circle ID.** `circle_id = SHA-256("cahoots/circle-id/v1" || PAD)[0..16]`.

Implementation: `core/src/ceremony.rs`, `commit.rs`, `combine.rs`, `checkwords.rs`, `circle_id.rs`.

### 3.1 Touch-link framing

```
frame = type (u8) | seq (u32) | len (u32) | payload | CRC-32
```

The CRC is IEEE 802.3 (the same polynomial as zlib) over every byte before it. It catches transmission errors only. Commitments and check words provide integrity. Devices form a ring and forward every commitment and reveal they receive until all devices hold all `n` of each. The touch link MUST NOT use any radio.

Implementation: `core/src/frame.rs`.

## 4. Messages

Each message spends `32 + len` bytes from the sender's slice, starting at the sender's next unused offset `o`.

```
header  = version (u8 = 1) | circle_id (16 B) | slice (u8) | offset o (u32) | len (u16) | type (u8)
key     = PAD_slice[o .. o+32]
stream  = PAD_slice[o+32 .. o+32+len]
ct      = plaintext XOR stream
tag     = Poly1305(key, header || ct)       // 16 bytes; key is used exactly once
message = header || ct || tag
```

`type`: `0` text, `1` approve, `2` vouch, `3` control.

Poly1305 with a one-time key is a Wegman-Carter authenticator. It is information-theoretically secure for one use. Clients SHOULD pad plaintext to buckets of 32, 64, 128, or 256 bytes to hide its length. The bucket format places the plaintext first and the plaintext length in the last byte.

For posting, a message is encoded as base64url text, or as a QR code for single messages.

Implementation: `core/src/message.rs`, `core/src/pad.rs` (`bucket_pad`).

### 4.1 Sending

1. Compute `o` from the ledger: one past the highest spent offset in the sender's slice.
2. Read the `32 + len` byte range.
3. Mark the range spent. The ledger write MUST be durable before step 4.
4. Erase the range and verify by reading back.
5. Seal and return the message.

A crash between steps 3 and 5 loses a message. It never leaves a range reusable.

### 4.2 Receiving

1. Find the circle by `circle_id`.
2. Reject the message if its slice or range lies outside the pad.
3. Reject the message if any byte in `[o, o+32+len)` of that slice is already spent. Flag it as possible reuse. This check happens before the tag check so a replayed, tampered message is still reported as reuse.
4. Verify the tag in constant time. On failure, reject and mark nothing spent.
5. Decrypt, mark the range spent, and erase those bytes.

Implementation: `core/src/pad.rs`, `core/src/ledger.rs`.

## 5. Storage and erasure

- The pad lives in a dedicated region of non-volatile memory, not in a file on a filesystem with a translation layer.
- The spent-range ledger is updated with power-fail safety: write-ahead record, then erase.
- Erasure overwrites the bytes, then verifies by reading back.
- Phone storage gives no erasure guarantee. The phone client MUST say erasure is best effort.

## 6. Test vectors

`vectors/*.json` are generated from `cahoots-core` by `cargo run -p cahoots-vectors`. They cover the ceremony (including a mismatch abort), check words, circle IDs, message encode and decode, a tag failure, reuse rejection, spend ordering, and framing. Every client MUST reproduce them.

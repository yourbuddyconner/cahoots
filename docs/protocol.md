# Cahoots protocol, version 1

This is the normative description of what `cahoots-core` implements. The words MUST, MUST NOT, SHOULD, and MAY have their RFC 2119 meanings. All integers are little-endian.

## 1. Terms

- **Contribution `R_i`:** the random bytes member device `i` generates for a ceremony.
- **Pad:** `PAD = R_1 XOR R_2 XOR ... XOR R_n`.
- **Circle:** the set of members that hold one pad.
- **Slice:** a fixed, non-overlapping part of the pad assigned to one sender.
- **Spent:** pad bytes that were used to encrypt or decrypt a message. They MUST be erased and never used again.
- **Sealed reveal:** a contribution encrypted to every other member of the ceremony (section 3.2). The link never carries a contribution in the clear.

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

Implementation: `crates/core/src/params.rs`.

## 3. Ceremony

Commit-then-reveal. Without it, the last member to reveal could choose a contribution after seeing everyone else's and bias the pad.

![The ceremony in five steps](diagrams/ceremony.svg)

1. **Setup.** The leader chooses the parameters. They are sent to every device and shown for confirmation.
2. **Commit.** Each device generates `R_i` and a KEM keypair `(dk_i, pk_i)` from 32 more random bytes, and computes `C_i = SHA-256("cahoots/commit/v1" || params || i || pk_i || R_i)`. Every device sends `(C_i, pk_i)` to every other device. Both are public. A device MUST NOT reveal until it holds all `n` commitments.
3. **Reveal.** Each device sends its sealed reveal (section 3.2): `R_i` encrypted to every other member. Each received reveal is opened and the contribution checked against `C_j`, using the `pk_j` held with the commitment, in constant time. A reveal that fails to open is refused and nothing changes. A reveal that opens but does not match aborts the ceremony on that device. A device that holds no commitment for `j` yet MAY take a sealed reveal from `j` as both: the commitment is derived from the opened contribution and the key in the blob. That is safe because the device's own contribution was fixed before it sent anything.
4. **Combine.** `PAD = XOR of all R_j`. A device MAY XOR a second independent source into its own `R_i` before committing. That source MUST NOT replace the device's own TRNG output.
5. **Check words.** `check = SHA-256("cahoots/check/v1" || params || PAD)`. The first 66 bits are read as six 11-bit big-endian indices into the BIP-39 English wordlist. Every device shows the six words. Members read them aloud. A mismatch means the ceremony restarts.
6. **Erase.** Each device erases every `R_j` and keeps only `PAD`.
7. **Circle ID.** `circle_id = SHA-256("cahoots/circle-id/v1" || PAD)[0..16]`.

Implementation: `crates/core/src/ceremony.rs`, `commit.rs`, `reveal.rs`, `combine.rs`, `checkwords.rs`, `circle_id.rs`.

### 3.1 Touch-link framing

```
frame = type (u8) | seq (u32) | len (u32) | payload | CRC-32
```

The CRC is IEEE 802.3 (the same polynomial as zlib) over every byte before it. It catches transmission errors only. Commitments and check words provide integrity. Devices form a ring and forward every commitment and reveal they receive until all devices hold all `n` of each. The touch link MUST NOT use any radio.

Implementation: `crates/core/src/frame.rs`.

### 3.2 Sealed reveals

The ceremony link radiates: a wire, a USB cable, or a phone screen can be read by an antenna or a camera nearby. Without this section, an attacker in range during the ceremony records every contribution and holds the pad. With it, that attacker has to break X-Wing (X25519 and ML-KEM-768, both) for every member. Nobody else is affected: the pad is still the XOR of the contributions and never depends on the key exchange, so the claim against an attacker who was not in the room is unchanged.

The KEM is X-Wing (draft 06). `pk_i` is the 1216-byte encapsulation key derived from a 32-byte seed. Binding `pk_i` into `C_i` is what stops someone spliced into the cable from substituting keys: a reveal sealed with the real key then fails the commitment check and the ceremony aborts.

A sealed reveal from member `i` to the other members is:

```
version (u8 = 1) | i (u8) | pk_i (1216) | k (u8)
k × [ j (u8) | ct_j (1120) | wrap_j (48) ]
body = ChaCha20-Poly1305(K_i, nonce 0, aad = "cahoots/reveal/v1" || params || i || pk_i, R_i)
```

- `K_i` is a fresh 32-byte content key.
- For each other member `j`, in ascending order: `(ct_j, ss_j) = XWing.Encaps(pk_j)` with 64 bytes of randomness, `wrap_key_j = HKDF-SHA256(salt = "cahoots/reveal-wrap/v1", ikm = ss_j, info = params || i || j)`, and `wrap_j = ChaCha20-Poly1305(wrap_key_j, nonce 0, aad = i || j, K_i)`.
- Every key is used once, so every nonce is zero.
- Randomness is drawn in this order: `K_i`, then 64 bytes per recipient. `randomness_len(n) = 32 + 64 (n - 1)`.

Member `j` opens it by finding its entry, decapsulating `ct_j` with `dk_j`, unwrapping `K_i`, and decrypting the body. A failure anywhere is `corrupted`: nothing is learned and nothing changes. The opened `R_i` is then checked against `C_i`.

On the ring, sealed reveals are forwarded unchanged: every hop is a member and can open its own copy, so no per-hop session is needed. On phones, one animation carries the reveal to every scanner, since each scanner unwraps its own entry.

Implementation: `crates/core/src/reveal.rs`.

### 3.3 Pair keys and hello words

The sealed reveals leave every pair of members with two X-Wing shared secrets that nobody else can compute: the one `a` encapsulated to `b`, and the one `b` encapsulated to `a`. For members `lo < hi`:

```
pair_key(lo, hi) = HKDF-SHA256(salt = "cahoots/pair/v1",
                               ikm  = ss(lo → hi) || ss(hi → lo),
                               info = params || lo || hi)        32 bytes
```

Both members hold both secrets, so both derive the same key, and no third member can. Unlike the pad, this key is computational (as strong as X-Wing). Unlike the pad, it names an individual: something produced under `pair_key(a, b)` came from `a` or `b`, not from "some member". A device keeps `n - 1` pair keys next to the pad. A circle made before sealed reveals has none.

Hello words are the spoken form. Each pair keeps a counter, and for each counter value each member has two words to say:

```
v = LE32(HKDF-SHA256(no salt, ikm = pair_key, info = "cahoots/words/v1" || speaker || LE32(counter))[0..4])
words(speaker, counter) = WORDLIST[v & 0x7ff], WORDLIST[(v >> 11) & 0x7ff]
```

One member says their pair, the other answers with theirs. A listener who records one side's words cannot answer as the other side, and either side moving the counter on makes everything recorded so far useless. The counter is not sent anywhere; both members move it, and the apps show its number so they can resynchronise by saying it. The words are a function of the key, so there is nothing to rotate but the counter: a key only changes when the circle is made again.

Implementation: `crates/core/src/pairwise.rs`. Vectors: `pair_keys` and `hello_words` in `ceremony.json`.

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

Implementation: `crates/core/src/message.rs`, `crates/core/src/pad.rs` (`bucket_pad`).

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

Implementation: `crates/core/src/pad.rs`, `crates/core/src/ledger.rs`.

### 4.3 Control messages

A `control` message (type `3`) bootstraps a network. Its plaintext is one subtype byte, then the fields below. All integers little-endian.

| Subtype | Name | Payload |
|---|---|---|
| `0` | `peer` | `proto (u8) \| pubkey_len (u8) \| pubkey \| endpoint_len (u8) \| endpoint (UTF-8) \| allowed_len (u8) \| allowed_ips (UTF-8)`. `proto`: `0` WireGuard, `1` Reticulum, `2` Yggdrasil, `3` Meshtastic. |
| `1` | `psk` | `purpose (u8) \| peer_a (u8) \| peer_b (u8) \| offset (u32)`. The issuer's slice is the message's own slice; `peer_a` MUST equal it. `purpose`: `0` WireGuard, `1` TLS, `2` MLS, `3` Wi-Fi, `4` Meshtastic, `5` other. |
| `2` | `rotate` | `purpose (u8) \| peer_a (u8) \| peer_b (u8) \| not_after (u64 Unix seconds)` |
| `3` | `slice-transfer` | `member (u8)`. The member's slice now lives on a node, not a badge. |

A pre-shared key named by a `psk` message is the 32 bytes at `offset` of the issuer's slice, derived with the context `purpose || peer_a || peer_b`, so a key issued for WireGuard between A and B cannot be reused for another purpose or another pair. The acceptor marks that range spent in its own ledger; a second `psk` message for the same range is refused as reuse. `peer_b = 255` means every member: a group key, such as a Meshtastic channel key, which every member accepts from the same message.

Control messages are ordinary messages: posted anywhere, including the Numbers Station, and readable or forgeable only by members. The feed is a channel, not a party.

Implementation: `crates/core/src/control.rs`; the WireGuard use is `crates/wireguard`.

## 5. Storage and erasure

- The pad lives in a dedicated region of non-volatile memory, not in a file on a filesystem with a translation layer.
- The spent-range ledger is updated with power-fail safety: write-ahead record, then erase.
- Erasure overwrites the bytes, then verifies by reading back.
- Phone storage gives no erasure guarantee. The phone client MUST say erasure is best effort.

## 6. Test vectors

`crates/vectors/*.json` are generated from `cahoots-core` by `cargo run -p cahoots-vectors`. They cover the ceremony (KEM seeds, public keys, sealing randomness, sealed reveals, a mismatch abort, and a garbled reveal), control messages, check words, circle IDs, message encode and decode, a tag failure, reuse rejection, spend ordering, and framing. Every client MUST reproduce them.

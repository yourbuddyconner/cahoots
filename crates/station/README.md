# cahoots-station: the Numbers Station

An append-only, hash-chained public relay for circle ciphertext. Anyone can read it. Only pad holders can decrypt what is on it. It is the public feed every circle posts to and the relay for pad-seeded chat (`docs/chat.md`).

## What it does

- Stores every accepted message as one line of JSONL, in order, and never edits or removes a line.
- Chains the lines: each entry's hash commits to the previous hash, so the head is a 32-byte summary of the whole history. On start it replays the file and re-verifies every hash. A broken chain refuses to start.
- Refuses pure-pad address reuse. A pure-pad message names the pad bytes it spent as `(circle, slice, offset, length)`. If that range overlaps any earlier accepted entry for the same circle and slice, the station answers `409 {"error":"reuse"}`. It cannot verify a tag, because it holds no pad, but it can refuse a double spend at the address level, and a buggy client cannot get past it.
- Accepts chat messages (version byte 2) without a reuse check. An epoch pointer appears in many chat messages by design, so reuse protection for epochs lives in each member's ledger.
- Serves a read-only live feed page at `/` for a big screen, with no external assets.

## What it does not do

- Rate limiting per circle or per client. Add it at the front proxy for now.
- Any decryption, tag verification, or membership check. It has no key material at all.
- Authentication of posters. Anyone can post bytes that parse. The circle's tags make a message real; the station only orders and keeps it.

## Run

```sh
cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --log ./station.jsonl
```

Docker, from the workspace root:

```sh
docker build -f crates/station/Dockerfile -t cahoots-station .
docker run -p 8080:8080 -v station-data:/data cahoots-station
```

## API

| Method and path | Body or query | Returns |
|---|---|---|
| `POST /v1/circles/{circle_id}/messages` | raw message bytes, at most 64 KiB | `201 {"seq", "hash"}`; `409 {"error":"reuse","prior_seq"}`; `400 {"error":"malformed"}` |
| `GET /v1/circles/{circle_id}/messages` | `after=<seq>`, `wait=<ms>` (long-poll, max 30000) | `{"entries":[...], "head_seq"}` |
| `GET /v1/circles/{circle_id}/head` | | `{"seq", "hash"}` for that circle's latest entry |
| `GET /v1/head` | | global `{"seq", "hash"}`; `seq` is `null` when empty |
| `GET /v1/circles` | | `{"circles":[{"circle_id","count","last_seq"}]}` |
| `GET /v1/feed` | `after=<seq>` | up to 500 entries across all circles, for the display page |
| `GET /healthz` | | `ok` |

`circle_id` is 32 hex characters and must match the circle ID inside the message header.

An entry on the wire and on disk:

```json
{"seq":12,"received_at":1760900000000,"circle_id":"6f86…","message":"AQ…","hash":"3c1e…"}
```

`message` is base64url without padding. `received_at` is Unix milliseconds. `hash` is hex.

## The chain

```
hash[0]   = SHA-256(0x00 * 32 || seq=0 (u64 LE) || received_at (u64 LE) || message)
hash[n]   = SHA-256(hash[n-1] || seq=n (u64 LE) || received_at (u64 LE) || message)
```

Clients that keep the last head they saw can detect a rollback: a later head whose history does not include the remembered hash means the log was rewritten. A member can cross-check its own chained spend ledger against this chain to prove that a message was dropped or reordered.

## Accepted message shapes

- **Pure pad, version byte 1.** Parsed with `cahoots_core::message::parse`. The spent range is `[offset, offset + 32 + len)`.
- **Chat, version byte 2.** `version | circle_id (16) | epoch_id (5) | sender (1) | counter (u32) | len (u32) | type (1) | ciphertext`, where the ciphertext is exactly `len + 16` bytes. Only the length arithmetic is checked.

## Deploying as a [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) workload

The station is built as an OCI image so it can run inside [Turnkey Verifiable Cloud](https://docs.turnkey.com/features/verifiable-cloud/overview), where the log head is anchored and the operator cannot roll the log back or show different readers different histories. **That deploy has not been done yet.** Until it is, this binary runs anywhere, and anchoring depends on the operator's word rather than on the enclave.

The deploy record must pin:

- the image digest of `cahoots-station`;
- the log file location inside the sealed volume (`/data/station.jsonl`);
- the policy hash. The station has no policy today, so this is a placeholder for the hosted-member policy that will share the workload (`appManifestDigest`).

What anchoring adds once deployed: the station periodically commits its head hash to the [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) log. A reader who verifies the anchored head against the chain knows the history is the one everyone else sees.

## CORS

The API allows any origin for `GET`, `POST`, and `OPTIONS` with a `Content-Type` header. The feed is public ciphertext by design, so there is nothing to protect with an origin check.

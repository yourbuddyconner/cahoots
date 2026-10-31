# cahoots-station: the Numbers Station

An append-only, hash-chained public relay for circle ciphertext. Anyone can read it. Only pad holders can decrypt what is on it. It is the public feed every circle posts to and the relay for chat (`docs/chat.md`).

## What it does

- Stores every accepted message as one row of a SQLite database (one file, WAL mode), in order, and never edits or removes a row.
- Chains the entries: each entry's hash commits to the previous hash, so the head is a 32-byte summary of the whole history. On start it replays every entry in order and re-verifies every hash. A broken chain refuses to start.
- Refuses pure-pad address reuse. A pure-pad message names the pad bytes it spent as `(circle, slice, offset, length)`. If that range overlaps any earlier accepted entry for the same circle and slice, the station answers `409 {"error":"reuse"}`. It cannot verify a tag, because it holds no pad, but it can refuse a double spend at the address level, and a buggy client cannot get past it.
- Accepts chat messages (version byte 2) without a reuse check. An epoch id appears in many chat messages by design. Replay protection for chat lives in each member's chains.
- Accepts ceremony messages (version byte 3) without a reuse check. A ceremony held through the relay posts its three rounds here, sealed under a key the station never sees. They are left out of `/v1/circles` and `/v1/feed`.
- Serves a read-only live feed page at `/` for a big screen, with no external assets (feature `feed-page`, on by default).
- Reports its version, compiled features and limits at `GET /v1/info`.

- Limits each client address to a burst and a rate of posts and of reads (`--posts-per-minute`, default 120; `--reads-per-minute`, default 600; 0 for none), answering `429 {"error":"rate"}` with `Retry-After: 10` past them. Behind a proxy, `--trust-forwarded` takes the client address from `Fly-Client-IP` or the first `X-Forwarded-For` entry; without it those headers are ignored, so a client cannot pick its own bucket.
- Refuses posts with `507 {"error":"full"}` once the database file and its write-ahead log together reach `--max-log-bytes` (0, the default, for no cap). Reads keep working. The log is never truncated.

## What it does not do

- Rate limiting per circle. The limit is per client address.
- Any decryption, tag verification, or membership check. It has no key material at all.
- Authentication of posters. Anyone can post bytes that parse. The circle's tags make a message real; the station only orders and keeps it.

## Run

```sh
cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --db ./station.db
```

`--db` names the SQLite database (default `./station.db`). It replaces the old `--log` flag. The old JSONL log is not read and there is no migration: the project is pre-release; the previous log is discarded.

Docker, from the workspace root:

```sh
docker build -f crates/station/Dockerfile -t cahoots-station .
docker run -p 8080:8080 -v station-data:/data cahoots-station
```

The image is built with the features `objects,channels,gate` and keeps its database at `/data/station.db`.

## Features

Cargo features choose what the station serves. The default features reproduce the plain relay, so an operator who changes nothing runs what they ran before.

| Feature | Adds | Needs |
|---|---|---|
| (base) | circles, messages, hash chain, rate limits, size cap, `/v1/head`, `/v1/info`, `/healthz` | |
| `feed-page` | the live feed page at `/` | base. On by default |
| `objects` | manifests, channel objects, policies, grants, revocations, deposits, join requests: stored, served, listed by channel | base |
| `channels` | version 4 messages chained per channel, per-channel heads, long-poll by channel | `objects` |
| `gate` | read and post checks: a gate token or a verified grant to read a channel; a verified post grant, signature, budget and quorum to post | `channels` |
| `issuer` | the relay checks evidence and issues grants and deposits under its own key | `gate`, and the TVC build only |
| `anchor` | periodic head anchoring to the TVC log | base |

Only `feed-page` changes behaviour today. The other features are declared so that `/v1/info` can name them; their routes come in later changes.

## API

| Method and path | Body or query | Returns |
|---|---|---|
| `POST /v1/circles/{circle_id}/messages` | raw message bytes, at most 64 KiB | `201 {"seq", "hash"}`; `409 {"error":"reuse","prior_seq"}`; `400 {"error":"malformed"}`; `413 {"error":"too_large"}` for a body over 64 KiB; `400 {"error":"circle_id"}` for a path id that is not 32 hex chars. Error responses also carry a `detail` field |
| `GET /v1/circles/{circle_id}/messages` | `after=<seq>`, `wait=<ms>` (long-poll, max 30000) | `{"entries":[...], "head_seq"}` |
| `GET /v1/circles/{circle_id}/head` | | `{"seq", "hash"}` for that circle's latest entry |
| `GET /v1/head` | | global `{"seq", "hash"}`; `seq` is `null` when empty |
| `GET /v1/circles` | | `{"circles":[{"circle_id","count","last_seq"}]}` |
| `GET /v1/feed` | `after=<seq>` | up to 500 entries across all circles, for the display page |
| `GET /v1/info` | | `{"version", "features":[...], "limits":{"max_body":65536, "posts_per_minute", "reads_per_minute"}}`; `features` lists the compiled features in the order `feed-page, objects, channels, gate, issuer, anchor`; a limit that is off is `null` |
| `GET /healthz` | | `ok` |

Any request under `/v1` can also answer `429 {"error":"rate"}` (over the per-client limit) and a post `507 {"error":"full"}` (the log has reached its cap).

`circle_id` is 32 hex characters and must match the circle ID inside the message header. A ceremony message carries its channel id in that place, so the path names the channel id.

An entry on the wire:

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
- **Ceremony channel, version byte 3.** `version | channel_id (16) | len (u32) | sealed`, where `sealed` is exactly `len` bytes. Only the length is checked. The channel id is random and comes from the code the host shows. The station stores these like chat messages, under the channel id, and they count against the same rate limit and size cap. They are left out of `/v1/circles` and `/v1/feed`, so only someone who holds the channel id reads them. See [protocol](../../docs/protocol.md) section 3.7.

## Deploying as the default relay

The app ships pointing at `relay.cahoots.network`. The recipe for that host, and for a relay of your own, is in [deploy/relay](../../deploy/relay/README.md).

## Deploying as a [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) workload

The station is built as an OCI image so it can run inside [Turnkey Verifiable Cloud](https://docs.turnkey.com/features/verifiable-cloud/overview), where the log head is anchored and the operator cannot roll the log back or show different readers different histories. **That deploy has not been done yet.** Until it is, this binary runs anywhere, and anchoring depends on the operator's word rather than on the enclave.

The deploy record must pin:

- the image digest of `cahoots-station`;
- the database location inside the sealed volume (`/data/station.db`);
- the policy hash. The station has no policy today, so this is a placeholder for the hosted-member policy that will share the workload (`appManifestDigest`).

What anchoring adds once deployed: the station periodically commits its head hash to the [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) log. A reader who verifies the anchored head against the chain knows the history is the one everyone else sees.

## CORS

The API allows any origin for `GET`, `POST`, and `OPTIONS` with a `Content-Type` header. The feed is public ciphertext by design, so there is nothing to protect with an origin check.

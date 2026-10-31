# cahoots-station: the Numbers Station

An append-only, hash-chained public relay for circle ciphertext. Anyone can read it. Only pad holders can decrypt what is on it. It is the public feed every circle posts to, the relay for chat (`docs/chat.md`), and, with the channel features, the relay for channels (`docs/protocol.md` section 8.9).

## What it does

- Stores every accepted message as one row of a SQLite database (one file, WAL mode), in order, and never edits or removes a row.
- Chains the entries: each entry's hash commits to the previous hash, so the head is a 32-byte summary of the whole history. On start it replays every entry in order and re-verifies every hash. A broken chain refuses to start.
- Refuses pure-pad address reuse. A pure-pad message names the pad bytes it spent as `(circle, slice, offset, length)`. If that range overlaps any earlier accepted entry for the same circle and slice, the station answers `409 {"error":"reuse"}`. It cannot verify a tag, because it holds no pad, but it can refuse a double spend at the address level, and a buggy client cannot get past it.
- Accepts chat messages (version byte 2) without a reuse check. An epoch id appears in many chat messages by design. Replay protection for chat lives in each member's chains.
- Accepts ceremony messages (version byte 3) without a reuse check. A ceremony held through the relay posts its three rounds here, sealed under a key the station never sees. They are left out of `/v1/circles` and `/v1/feed`.
- With the features `objects`, `channels` and `gate`, stores channel objects and serves channel messages (version byte 4), and checks who may read and post (see [Channels](#channels)).
- Serves a read-only live feed page at `/` for a big screen, with no external assets (feature `feed-page`, on by default).
- Reports its version, compiled features and limits at `GET /v1/info`.

- Limits each client address to a burst and a rate of posts and of reads (`--posts-per-minute`, default 120; `--reads-per-minute`, default 600; 0 for none), answering `429 {"error":"rate"}` with `Retry-After: 10` past them. Behind a proxy, `--trust-forwarded` takes the client address from `Fly-Client-IP` or the first `X-Forwarded-For` entry; without it those headers are ignored, so a client cannot pick its own bucket.
- Refuses posts with `507 {"error":"full"}` once the database file and its write-ahead log together reach `--max-log-bytes` (0, the default, for no cap). Reads keep working. The log is never truncated.

## What it does not do

- Rate limiting per circle. The limit is per client address, and with `channels` also per channel for posts.
- Any decryption or tag verification. It has no key material at all, with any feature.
- Authentication of circle posters. Anyone can post bytes that parse to a circle. The circle's tags make a message real; the station only orders and keeps it. With `gate`, the station checks channel grants, which are public signed data.

## Run

```sh
cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --db ./station.db
```

`--db` names the SQLite database (default `./station.db`). It replaces the old `--log` flag. With `channels`, `--channel-posts-per-minute` (default 300, 0 for none) limits the messages any one channel accepts, whoever sends them. The old JSONL log is not read and there is no migration: the project is pre-release; the previous log is discarded.

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

`issuer` and `anchor` are declared so that `/v1/info` can name them; neither has any code yet. The default relay runs `objects`, `channels` and `gate`, which is what the Docker image builds.

Build with features:

```sh
cargo run -p cahoots-station --features objects,channels,gate -- --db ./station.db
```

## API

| Method and path | Body or query | Returns |
|---|---|---|
| `POST /v1/circles/{circle_id}/messages` | raw message bytes, at most 64 KiB | `201 {"seq", "hash"}`; `409 {"error":"reuse","prior_seq"}`; `400 {"error":"malformed"}`; `413 {"error":"too_large"}` for a body over 64 KiB; `400 {"error":"circle_id"}` for a path id that is not 32 hex chars. Error responses also carry a `detail` field |
| `GET /v1/circles/{circle_id}/messages` | `after=<seq>`, `wait=<ms>` (long-poll, max 30000) | `{"entries":[...], "head_seq"}` |
| `GET /v1/circles/{circle_id}/head` | | `{"seq", "hash"}` for that circle's latest entry |
| `GET /v1/head` | | global `{"seq", "hash"}`; `seq` is `null` when empty |
| `GET /v1/circles` | | `{"circles":[{"circle_id","count","last_seq"}]}` |
| `GET /v1/feed` | `after=<seq>` | up to 500 entries across all circles, for the display page |
| `GET /v1/info` | | `{"version", "features":[...], "limits":{"max_body":65536, "posts_per_minute", "reads_per_minute"}}`, plus `channel_posts_per_minute` with `channels`; `features` lists the compiled features in the order `feed-page, objects, channels, gate, issuer, anchor`; a limit that is off is `null` |
| `GET /healthz` | | `ok` |

Any request under `/v1` can also answer `429 {"error":"rate"}` (over the per-client limit) and a post `507 {"error":"full"}` (the log has reached its cap).

`circle_id` is 32 hex characters and must match the circle ID inside the message header. A ceremony message carries its channel id in that place, so the path names the channel id.

An entry on the wire:

```json
{"seq":12,"received_at":1760900000000,"circle_id":"6f86…","message":"AQ…","hash":"3c1e…"}
```

`message` is base64url without padding. `received_at` is Unix milliseconds. `hash` is hex.

## Channels

A channel is a stream a circle publishes for people outside it (`docs/protocol.md` section 8.9). It is not the ceremony channel above. Three features carry it: `objects` stores the signed objects, `channels` stores and serves the messages, and `gate` checks who may read and post. The station never holds a channel key, a link secret or a seal key under any feature.

The channel's messages are entries in the same log and the same hash chain as circle messages. Every position a channel names is the log's global `seq`. A channel's head is its latest entry. The circle routes answer `404 {"error":"channel"}` for a channel id, and `/v1/circles` and `/v1/feed` leave channels out.

All bodies are raw bytes, at most 64 KiB. Ids in paths are lowercase hex. Byte strings in JSON answers are base64url without padding. Every write can also answer `507 full` and `429 rate`.

| Method and path | Feature | Body or query | Returns |
|---|---|---|---|
| `PUT /v1/circles/{circle_id}/manifest` | objects | a sealed manifest | `201 {"circle_id"}`, also for the same bytes again; `409 exists` for different bytes; `400 circle_id` for a manifest of another circle |
| `GET /v1/circles/{circle_id}/manifest` | objects | | the manifest bytes; `404 manifest` |
| `POST /v1/channels` | objects | a signed channel object | `201 {"channel_id"}` after verifying it against the stored manifest; `404 manifest`; `403 signature`; `409 stale` for an object no later than the one held |
| `GET /v1/channels/{channel_id}` | objects | | `{"channel", "policy", "manifest"}`: the current channel object, its current policy (or `null`) and the owner's manifest |
| `GET /v1/channels` | channels | | `{"channels":[{"channel_id","count","last_seq"}]}`, most recently active first |
| `POST /v1/channels/{channel_id}/policy` | objects | `[signed policy, [approval, ...]]` | `201 {"policy_id"}`; `409 stale` for a policy whose `log` is not greater than the current one's; with `gate`, `403 grant` (`quorum`) when a later policy lacks the govern approvals |
| `POST /v1/channels/{channel_id}/grants` | objects | a grant chain, `[link, ...]` root first | `201 {"grant_id"}`, the leaf's `cti`; with `gate`, `403 grant` (`chain`) when the chain does not verify against the owner's manifest |
| `GET /v1/channels/{channel_id}/grants?kid=<hex>` | objects | | `{"grants":[chain, ...]}`, the chains for that member key; gated |
| `GET /v1/channels/{channel_id}/grants/{cti}` | objects | | `{"grant": chain}`; `404 grant`; gated |
| `POST /v1/channels/{channel_id}/revocations` | objects | `[grant_id, statement]` | `201 {"grant_id"}`; the statement is a seat's COSE_Sign1 under `cahoots/revoke/v1` |
| `GET /v1/channels/{channel_id}/revocations` | objects | | `{"revocations":[{"grant_id","seq"}]}` |
| `POST /v1/channels/{channel_id}/deposits` | objects | an encoded deposit | `201 {"deposit_id"}` |
| `GET /v1/channels/{channel_id}/deposits?to=<hex>&epoch=<hex>` | objects | both filters optional | `{"deposits":[...]}`, empty when none; gated |
| `POST /v1/channels/{channel_id}/requests` | objects | a join request, any non-empty bytes | `201 {"request_id","seq"}`; open to anyone |
| `GET /v1/channels/{channel_id}/requests?after=<seq>` | objects | | `{"requests":[{"id","body","seq"}]}`, at most 500; gated, and needs `admit` |
| `POST /v1/channels/{channel_id}/gates` | objects | a seat's COSE_Sign1 under `cahoots/gate/v1` over `{0: channel_id, 1: gate_token}` | `201 {"token_hash"}`; the station stores only the SHA-256 of the token |
| `DELETE /v1/channels/{channel_id}/gates/{token_hash}` | objects | the same map signed under `cahoots/gate-retire/v1` | `204` |
| `POST /v1/channels/{channel_id}/messages` | channels | a version 4 message | `201 {"seq","hash"}`; `404 channel` for a channel the station does not hold; `400 malformed` for a message that names another channel; `429 rate` past the per-channel limit; with `gate`, `403 grant` and `409 replay` (below) |
| `GET /v1/channels/{channel_id}/messages` | channels | `after=<seq>`, `wait=<ms>` | `{"entries":[{"seq","received_at","channel_id","message","hash"}], "head_seq"}`; gated |
| `GET /v1/channels/{channel_id}/head` | channels | | `{"seq","hash"}` of the channel's latest entry; gated |
| `GET /v1/nonce` | gate | | `{"nonce"}`, 16 bytes in hex, good once for 300 seconds |

Objects are verified against the owner circle's stored manifest. A policy must name the circle that owns the channel.

### Reading a gated channel

With `gate`, the routes marked gated answer `401 {"error":"gate"}` unless the request carries one of these:

- `Cahoots-Gate: <hex of the gate token>`. The station checks that the token's SHA-256 is registered for this channel. It does not open `/requests`.
- `Cahoots-Proof: <base64url COSE_Sign1>` and `Cahoots-Kid: <hex of the member key id>`. The proof is the protocol's proof of possession (`cahoots/proof/v1`, payload `[data_hash, nonce]`) by the member key, over a nonce from `GET /v1/nonce`. `data_hash` is the SHA-256 of `METHOD " " PATH "?" QUERY` as in the request line, with the `?` written even when the query is empty, such as `GET /v1/channels/<id>/head?`. A stored chain for that key must verify, allow `read` at the channel's head, and not be revoked. `/requests` also needs `admit`. Each nonce is spent when it is presented.

An unknown channel answers `404 {"error":"channel"}` on every gated route.

### Posting to a gated channel

A message's `kid` must have a grant chain stored on the station. The station tries each chain for that key, and refuses the post with `403 {"error":"grant","reason"}` when none allows it. The reasons are:

| Reason | Meaning |
|---|---|
| `missing` | no chain is stored for the key |
| `chain` | the chain does not verify against the owner's manifest |
| `signature` | the message's signature is not by the grant's key |
| `revoked` | a grant in the chain is revoked |
| `expired` | the channel's head is at or past the grant's `cahoots-log` |
| `budget` | the grant's posts are spent |
| `action` | the grant does not carry the action: `post` for text and announcements, `govern` for control messages |
| `quorum` | an announcement's approvals do not reach the post rule's quorum |

A message whose `(kid, epoch_id, counter)` the channel already holds answers `409 {"error":"replay"}`, even with a different body. A client that lost a `201` and sends again gets `409` and can count the message as sent.

## The chain

```
hash[0]   = SHA-256(0x00 * 32 || seq=0 (u64 LE) || received_at (u64 LE) || message)
hash[n]   = SHA-256(hash[n-1] || seq=n (u64 LE) || received_at (u64 LE) || message)
```

Clients that keep the last head they saw can detect a rollback: a later head whose history does not include the remembered hash means the log was rewritten. A member can cross-check its own chained spend ledger against this chain to prove that a message was dropped or reordered.

## Accepted message shapes

- **Pure pad, version byte 1.** Parsed with `cahoots_core::message::parse`. The spent range is `[offset, offset + 32 + len)`.
- **Chat, version byte 2.** `version | circle_id (16) | epoch_id (5) | sender (1) | counter (u32) | len (u32) | type (1) | ciphertext`, where the ciphertext is exactly `len + 16` bytes. Only the length arithmetic is checked.
- **Channel, version byte 4** (feature `channels`). `version | channel_id (16) | epoch_id (20) | kid (32) | counter (u32) | len (u32) | type (1) | ciphertext | signature`. Accepted only on `/v1/channels/{channel_id}/messages`, for a channel the station holds; the circle routes refuse it. See [protocol](../../docs/protocol.md) section 8.9.
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

The API allows any origin for `GET`, `POST`, and `OPTIONS` with a `Content-Type` header. With `objects` it also allows `PUT` and `DELETE`, and with `gate` the headers `Cahoots-Gate`, `Cahoots-Proof` and `Cahoots-Kid`. The feed is public ciphertext by design, so there is nothing to protect with an origin check.

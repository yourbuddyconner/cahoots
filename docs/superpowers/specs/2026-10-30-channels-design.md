# Channels: a group chat anyone with a link can join, on a relay that keeps only ciphertext

*Written 2026-10-30 from a conversation with Conner. A circle today is closed: its members met, and nobody joins later. This design adds the channel, a stream a circle publishes that people outside the circle can read and, when allowed, post to. The first version is the invite link: a person opens a link, the app opens, and they are in a group chat whose relay never held a key. The same objects carry the later versions, where the ticket in is a passport proof, membership of another circle, or an outside account checked in an enclave. It is a design to plan from, not a plan.*

## The idea

A circle is the unit of trust in Cahoots. It is made in a room and it does not grow. That is right for the pad and wrong for a community: a person with a circle wants people who were not in the room to hear from them, talk back, and bring their friends. Today they have nothing to send such people, so there is no reason to install the app.

A **channel** is what the circle sends them. It is a stream owned by a circle, keyed separately from the circle's own chat, and governed by a **policy** the owner writes: who may read, who may post, who may admit others, and what a post needs. Each of those is answered by a list of **evidence** the policy accepts. The first and simplest evidence is a secret carried in a link. The others, a proof from a passport, a seat in another circle, an account checked in an enclave, slot into the same list later without changing a reader.

Everything a member of a channel holds is a **grant**: the CBOR Web Token the protocol already uses, with a new detail type. Read, post and admit are actions in it. Admit may be delegated, which is how a channel's membership grows without the owner issuing every grant, and how a circle can vouch for its members in someone else's channel. Delegation depth, expiry at a log position and revocation in the log are the grant's own rules, unchanged.

The **relay** stores ciphertext, signed objects and sealed key deposits, in SQLite rather than one flat file. It can check grants and gate tokens, because they are public signed data, and it never holds a channel key. A relay operator compiles in exactly the behaviour they want to run: the base relay is what exists today, and channels, gating, and the issuer are Cargo features on top.

## Goals

- A person with a circle starts a channel, gets a link, and sends it. A person who opens the link lands in the browser app and is reading the channel within a few seconds. Once the native app exists, the same link opens it on an iPhone that has it installed, and the browser app offers the install to one that does not.
- A channel can be a one-way announcement stream or a group chat, and the owner can switch between them and anything in between by editing the policy, without a new channel or a new link.
- Members admitted by link can be given the right to post, either automatically because the policy says anyone with the link may post, or by the owner's approval, and the owner can take that right back.
- A channel owned by a circle of several members can require approvals: a quorum of seats to post an announcement, to change the policy, or to admit.
- The relay operator sees ciphertext, signed objects and traffic shape, and can prove the operator could never read, because the relay's code has no key path and its features are visible at compile time.
- The policy's evidence list is defined so that a passport proof, another circle's manifest, and an enclave-checked account are each one new kind of evidence and nothing else.
- The whole path is tested on the laptop with the in-memory relay and vectors, and end to end with two phones or a phone and the browser app against a real relay.

## Non-goals, for this version

- Any evidence kind but the link secret and an explicit key. The other kinds are specified as far as the policy and the issuer are concerned and not built.
- The issuer as a running service. The issuer role exists in the policy and in the relay's feature list; in this version the owner's phone performs it when the app is open.
- A pad guarantee for channel messages. A channel is conventional cryptography, labelled so.
- Media beyond what the circle's chat has, reactions, threads, read receipts, typing.
- Push notifications. The iOS design's position stands: poll in the foreground and background refresh.
- Moving the circle's own chat onto channels. The circle's chat stays on the chat crate's pad-seeded epochs.
- Publishing to several relays at once, and reading from any. It is the first thing the crisis-communication use case will need, and it is a client behaviour this design leaves room for and does not build.
- Anchoring the relay's log in an enclave (C5). Unchanged by this design.

## Anti-goals

- A relay that can read. No feature, flag or default puts a channel key, a link secret or a seal key on the relay. A deposit is opaque to it, a gate token is derived so the relay cannot recover the secret, and the issuer feature compiles only into the enclave build.
- A link that gives more than it says. A link admits a reader. Posting comes from a grant, and the policy says whether the link alone earns one.
- A member who cannot be removed. Every member holds a grant that a revocation ends, and every key they hold ends at the next epoch.
- Silent policy changes. A policy change is a signed object in the channel's log that every member's app shows.
- Sender anonymity among members presented as a feature. Every post is signed by its member's key. Readers can tell who posted; the relay can tell which grant posted. A channel that wants anonymous posting is a different design.
- A second message format in Swift or JavaScript. Channel messages are built and read by the Rust crates.

## What exists and is reused

- `crates/objects`: version 2 grants as CWTs (`iss`, `sub`, `cti`, `cnf`, `authorization_details` under -65539, `cahoots-log`, `cahoots-depth`, `cahoots-parent`, `cahoots-grantor`), chain verification, the evaluator shaped as an AuthZEN request, approvals (protocol section 8.4), the JSON projection.
- `crates/chat`: the per-sender chain schedule (section 2 of the chat spec), which the channel reuses with a different root and a wider sender id.
- `crates/hpke`: sealing a key to a public key, used for per-member deposits.
- `crates/core`: the manifest, the seat keys, canonical CBOR, the COSE profile.
- `crates/station`: axum, the hash chain, the rate limiter, the size cap, CORS, the feed page, the Docker image and the TVC deploy notes. `crates/relay`: `HttpRelay` and `MemRelay`, the clients.
- The iOS design (2026-10-30): `crates/phone` with UniFFI, the phone's own P-256 key in the Secure Enclave (its section 6), which is the member key here; the browser app's chat screen and relay sheet.
- The agent grants design (2026-10-29): the consent screen in the badge's words, the time rule, the evaluator's check order.
- [Attested readings](../../attested-data.md): the pattern that time is the log and revocation is a message in it.

## The design

### 1. Words

- **Channel.** A stream published under a circle, with its own id, key schedule, policy and log. A circle may own many.
- **Owner.** The circle that made the channel. Within it, the seats the policy names as admins, by default every seat.
- **Member.** A key that holds a grant for the channel. A member is not a seat and need not be in any circle. It is a P-256 signing key the app makes for that channel and keeps in software under the platform's data protection, on the phone and in the browser alike. It is not the phone's Secure Enclave key: every post is signed, and a user-presence key would ask for Face ID on each message. The owner's seat key, which does ask, signs only the rare acts: the channel object, the policy, rotations, revocations and root grants. Beside the signing key each member makes an X-Wing key pair for receiving deposits (section 4).
- **Policy.** The signed object that says who may read, post, admit and change the policy, and what each needs.
- **Evidence.** What a key shows to earn a grant. Kinds are numbered; the first version has two.
- **Issuer.** Whoever checks evidence and issues grants and deposits. The owner's phone in this version; an enclave later.
- **Link.** A URL carrying the relay, the channel id and a secret in the fragment.
- **Deposit.** A channel key sealed for someone and stored on the relay.
- **Epoch.** A period under one channel key. Rotation is a new epoch.

### 2. The channel object

Domain `cahoots/channel/v1`. A canonical CBOR map signed as COSE_Sign1 by an admin seat of the owner circle (kid `[seat]`), or carrying the approvals the policy demands for creation (section 6).

| Key | Value |
|---|---|
| 0 | version, 1 |
| 1 | circle_id, bstr 16, the owner circle |
| 2 | channel_id, bstr 16 |
| 3 | name, tstr, 1 to 64 bytes |
| 4 | created_log, uint, the position in the channel's own log at which this object takes effect, 0 for the first |
| 5 | policy_id, bstr 32, the SHA-256 of the current policy's signed bytes |
| 6 | nonce, bstr 16 |

`channel_id` is the first 16 bytes of `SHA-256("cahoots/channel-id/v1" || circle_id || nonce)`. The object is published to the relay with the owner's sealed manifest, so any reader can verify the signing seat. A channel object is replaced, never edited: a new one with the same `channel_id` and a later `created_log` supersedes it, under the same signing rule, and the relay keeps both.

### 3. The policy

Domain `cahoots/policy/v1`. Signed by an admin seat, or with approvals (section 6). The policy is a list of rules, one per action, plus the issuers.

| Key | Value |
|---|---|
| 0 | version, 1 |
| 1 | circle_id, bstr 16 |
| 2 | channel_id, bstr 16 |
| 3 | rules, array of rule maps, in action order |
| 4 | issuers, array of issuer maps |
| 5 | log, uint, the channel's log position at which the policy takes effect |
| 6 | parent, bstr 32 or nil, the previous policy's id |
| 7 | nonce, bstr 16 |

A **rule** is `{0: action, 1: evidence, 2: per_evidence, 3: quorum, 4: assurance}`.

- `action` is `0` read, `1` post, `2` admit, `3` govern (change the policy or the channel object, rotate, revoke).
- `evidence` is an array of evidence maps the rule accepts; a key that satisfies any one of them may be granted the action. An empty array means nobody may be granted it except the owner's seats.
- `per_evidence` is the number of member keys one piece of evidence may back, `0` for no limit. For a link secret it limits how many people may join from one link; for a passport nullifier it is normally `1`.
- `quorum` is how many approvals from the owner circle's seats the action needs each time it is used. `0` means the grant alone suffices. A post rule with a quorum of 2 is an announcement channel where two members of the owning circle must sign each post.
- `assurance` is the minimum assurance of those approvals, section 8.2 of the protocol. The default is `2`, present.

An **evidence** map is `{0: kind, 1: ref, 2: scope}`, where `scope` is a tstr and is empty when the kind does not use it.

| Kind | Name | `ref` | `scope` | Checked by |
|---|---|---|---|---|
| 0 | link | bstr 32, the link id (section 7), or empty: an empty `ref` means the owner's current link, which the relay's registered gate token names | unused | anyone: a gate token or a seal proves the secret |
| 1 | key | bstr 33, one explicit key, or empty: an empty `ref` means any key the owner admits by hand | unused | the grant itself |
| 2 | circle | bstr 32, a manifest id | unused | the issuer: that circle's sealed manifest and its latest checkpoint carry the key |
| 3 | document | bstr, the verifier's registry root | tstr, the nullifier scope | the issuer: a zero-knowledge document proof bound to the key, with a nullifier under the scope |
| 4 | account | bstr 32, an enclave measurement | tstr, the issuer and the predicate, such as `github:org:example` | the issuer: a fact signed by the enclave key naming the member key |

Kinds 0 and 1 are built in this version. Kinds 2 to 4 are defined here so the policy editor, the grant and the issuer interface do not change when they arrive; what they need is in section 11.

An **issuer** map is `{0: key, 1: endpoint, 2: measurement}`: a key that may issue grants and deposits for this channel, where to send a join request, and the enclave measurement when the issuer is one (nil otherwise). The owner's admin seats are always issuers and are not listed. In this version the list is empty and the owner's phone issues.

The policy editor on the phone shows presets and writes rules: **announcements** (read by link, post by owner), **announcements with a quorum** (post needs `k` approvals), **group chat** (read and post by link), **members post** (read by link, post by key after the owner admits), **closed** (read by key only). Every preset is a policy; the editor also shows the rules as they are so an owner can tune them.

### 4. Keys and epochs

A channel's key is not the owner circle's pad. The owner draws 32 random bytes for each epoch and names the epoch by a counter.

```
epoch_id   = channel_id (16) | epoch (u32)
root       = 32 random bytes from the owner's generator
epoch_key  = HKDF-Extract-and-Expand(salt = "cahoots/channel/epoch/v1" || epoch_id, ikm = root, info = "", len = 32)
chain[k][0] = HKDF-Expand(epoch_key, info = "cahoots/channel/chain/v1" || kid, 32)
msg_key[k][n], chain[k][n+1]   as in the chat spec, section 2, with the same zeroization and skip rules
```

`kid` is the 32-byte key id of the member key (the SHA-256 of the compressed key, as in a grant's `sub`). Chains are keyed per member key rather than per seat byte, so a channel can have more than 256 members and a member needs no seat.

An owner that is a pad circle MAY seed `root` from a 32-byte pad spend instead of its generator, so the circle's own members get the chat spec's pad-hedged claim for the channel. The spend is recorded as the circle's chat spends are. The channel object does not say which was used; the owner's app does.

**Rotation** is a new epoch. The owner rotates when it revokes a member or a link, when the policy changes who may read, and on the chat spec's count and time rules, and it rotates from the app in the foreground: a rotation is signed by a seat key, which on the phone asks for Face ID, and background refresh never rotates. Readers keep at most four epoch states. A rotation is a control message in the channel's log (section 5) carrying the new `epoch_id` and the ids of the deposits made for it, so a reader knows to fetch its deposit.

**Deposits** carry the epoch key to readers. Domain `cahoots/deposit/v1`, stored on the relay, opaque to it.

| Key | Value |
|---|---|
| 0 | version, 1 |
| 1 | channel_id, bstr 16 |
| 2 | epoch_id, bstr 20 |
| 3 | to, bstr: for a link, the link id (32); for a key, its key id (32) |
| 4 | kind, uint: `0` link, `1` key |
| 5 | sealed, bstr |

A link deposit is `ChaCha20-Poly1305(seal_key, nonce = epoch (u32) || 0x00 * 8, aad = "cahoots/deposit/v1" || channel_id || epoch_id || link_id, epoch_key)`, where `seal_key` is derived from the link secret (section 7). A key deposit is the HPKE sealing of `epoch_key` to the member's X-Wing public key in base mode (no pad PSK, since a reader holds no pad), with the same `aad` and info `cahoots/deposit/v1`, using a base-mode entry point added to `crates/hpke`. The signing key cannot receive: a Secure Enclave key does no key agreement and `crates/hpke` offers X25519 and X-Wing, so each member carries a second, encryption-only key. The join request names it, signed by the signing key, and the join control message repeats it so every member and the issuer know which key to seal to. `to` is the key id of the signing key either way. X-Wing makes deposits post-quantum, as the ceremony's reveals are. The issuer makes one link deposit per live link and one key deposit per member whose grant was earned by evidence other than a link, for each epoch. A member who joined by link reads the link deposit until the link is rotated away; if the owner wants that member to survive a link rotation, it issues them a key deposit.

### 5. Messages

A channel message is a version 4 message on the relay's log, so the relay tells it from pad messages (1), chat messages (2) and ceremony channel messages (3) by its first byte.

```
header   = version (u8 = 4) | channel_id (16) | epoch_id (20) | kid (32) | counter (u32) | len (u32) | type (u8)
nonce    = counter (u32) | 0x00 * 8
ct       = ChaCha20-Poly1305(msg_key[kid][counter], nonce, aad = header, plaintext)
body     = header || ct
message  = body || COSE_Sign1(payload = approvals, kid = kid, external AAD "cahoots/channel-msg/v1" || body)
```

The COSE_Sign1 embeds a canonical CBOR array of approvals (section 8.4 of the protocol, as signed bytes), empty for every message but an announcement, and its signature covers that array and the body through the external AAD. The approvals ride outside the ciphertext on purpose: a gating relay must count them, and they say only that seats approved a digest. The signature is by the member key. A reader verifies it against the key in the member's grant, which it fetches from the relay by `kid` and verifies up its chain to an admin seat in the owner's manifest. Admin seats post the same way: when a channel is made, each admin seat issues a root grant for all four actions to a member key it makes for itself, so there is one verification path and a seat key never appears as a message's `kid`. A message whose signer holds no valid post grant for this channel at this epoch is shown as unverified or dropped, as the reader's policy view says; a gating relay refuses it. A reader keeps the grants it has verified.

`type` is `0` text, `1` control, `2` announcement. Plaintext is bucket-padded as in the chat spec.

A **control** message's plaintext is a canonical CBOR map `{0: kind, ...}`: `0` rotation (`1: epoch_id, 2: deposit ids`), `1` policy change (`1: policy_id`), `2` revocation (`1: grant id, 2: reason tstr`), `3` join (`1: grant id`, posted by the issuer when it admits a member so others learn the key), `4` leave. Control messages are signed by an admin seat or an issuer key and are always readable by every member.

An **announcement** carries, in its COSE_Sign1 payload, the approvals (protocol section 8.4) whose `request_digest` is the SHA-256 of the body, whose `profile` is `channel-post`, whose `log` is the channel log position the proposer named, and whose `fields` are the channel name and the first line of the text. Readers, who can decrypt, check the fields against the text; the relay checks everything else. A reader counts approvals from distinct seats of the owner's manifest with the rule's assurance and shows the post as an announcement only when the count reaches the rule's quorum. A gating relay does the same check before storing it. A quorum of 1 still carries one approval, by the posting seat, so there is no special case.

The whole message is what the relay chains. The hash chain and the `seq` are per channel, as they are per circle today.

### 6. Grants, admission and governance

A channel grant is a version 2 grant (protocol section 8.3) with one detail:

```
{"type": "cahoots:channel", "channel": <channel_id hex>, "actions": ["read", "post", "admit", "govern"], "budget": uint}
```

`actions` is a non-empty subset in that order. `budget` is the number of posts the grant allows, `0` for no limit; it applies to `post` only. `iss` names the owner circle, `cnf` is the member key, `cahoots-log` is the expiry as a position in the channel's own log, and the chain rules are the protocol's: a root grant is signed by an admin seat, a delegated one by the parent's grantee, actions shrink, depth falls by one. A grant that carries `admit` may issue grants below it. A grant may not carry a channel the parent does not.

The grant's `cti` is its id for the relay and for revocation. A **revocation** is a control message naming it, signed by an admin seat or by any key above it in the chain. A revoked grant ends every grant below it. The relay keeps a revocation list per channel, and the evaluator's first check, revoked, reads it.

**Admission** is how a key gets a root grant.

1. The key's holder forms a join request: `{channel_id, key, actions wanted, evidence, name}` signed by the key, where `evidence` is what the policy's rule for those actions accepts: for a link, the gate token; for an explicit key, nothing; for the later kinds, the proof.
2. The request goes to an issuer. In this version it is posted to the relay's request queue for the channel, and the owner's phone fetches the queue when the app is open or on background refresh.
3. The issuer evaluates each requested action against the policy: the evidence is one the rule accepts, `per_evidence` is not exhausted, and the key is not revoked. Actions whose rule has no quorum and whose evidence checks are granted without asking. In this version the owner's phone shows the request and the person approves on screen with Face ID; a policy may mark an action as automatic and the phone then grants it without a prompt.
4. The issuer posts the grant and, if the member needs one, a key deposit, and a join control message.

A link holder reads before any of this: the link deposit is enough. The join request is for posting and for surviving link rotation.

**Governance** in a circle of several seats. The `govern` rule's quorum says how many seats must approve a new policy, a new channel object, a rotation or a revocation. A governed object carries its approvals as the announcement does, with `profile` `channel-govern` and the object's digest. A policy with a `govern` quorum of 2 cannot be loosened by one seat alone; the relay and every member check it. The default policy from the editor sets the `govern` quorum to 1 for a one-seat circle and to a majority for a larger one, and shows the number.

### 7. Links

A link is

```
https://cahoots.network/app/#/ch/<relay host>/<channel_id hex>/<secret base64url>
```

The fragment never leaves the device: browsers do not send it, and the app does not either. The path before it opens the browser app, which shows the channel and offers the native app; with the app installed and the universal link association in place, iOS opens the app directly. Both read the same fragment. A link without a secret opens a channel whose read rule has no link evidence, where the reader needs a grant already.

From the secret `s` (32 random bytes):

```
link_id    = SHA-256("cahoots/link/id/v1" || s)
seal_key   = HKDF-Expand(HKDF-Extract(salt = "cahoots/link/seal/v1", ikm = s), channel_id, 32)
gate_token = HKDF-Expand(HKDF-Extract(salt = "cahoots/link/gate/v1", ikm = s), channel_id, 32)
```

The policy names a link by `link_id`. The relay, when gating, is given `gate_token` by the owner when the link is made, and it learns nothing of `s` or `seal_key` from it. A reader presents `gate_token` in a header to fetch messages and deposits. A link is retired by rotating the epoch without a deposit for it and removing its id from the policy; the relay then refuses its token.

Honest limits of a link: anyone the holder forwards it to can read until it is retired, and a link alone cannot count or expire its readers, because nobody checks it at the door. A policy that wants both uses `per_evidence` and an issuer, which the first version has only when the owner's phone is open. The app says this on the share screen in one sentence.

### 8. The relay

The station becomes a workspace crate with Cargo features. Default features reproduce today's relay exactly, so an operator who changes nothing runs what they run now.

| Feature | Adds | Needs |
|---|---|---|
| (base) | circles, messages, hash chain, rate limits, size cap, `/v1/head`, `/healthz` | |
| `feed-page` | the live feed page at `/` | base. On by default |
| `objects` | manifests, channel objects, policies, grants, revocations, deposits, join requests: stored, served, listed by channel | base |
| `channels` | version 4 messages in the relay's one hash chain, per-channel heads, long-poll by channel | `objects` |
| `gate` | read and post checks: a gate token or a verified grant to read a channel; a verified post grant, signature, budget and quorum to post | `channels` |
| `issuer` | the relay checks evidence and issues grants and deposits under its own key. Declared; no code in this version | `gate`, and the TVC build only: the feature refuses to compile outside the enclave target profile, since a relay that issues holds keys |
| `anchor` | periodic head anchoring to the TVC log (C5). Declared; no code in this version | base |

`GET /v1/info` returns the features compiled in, the relay's version, and its limits, so the app knows whether this relay serves channels, gates, or issues, and says so to the person before they rely on it.

**Storage** is SQLite through `rusqlite` with the bundled engine, one file, WAL mode. Tables: `entries(seq, stream, received_at, message, hash)` where `stream` is a circle id or a channel id and the chain is per stream; `objects(id, kind, stream, body, created_seq)`; `revocations(stream, grant_id, seq)`; `gates(stream, token_hash)` storing the SHA-256 of the gate token so a database copy does not hand out live tokens; `requests(stream, id, body, seq)`. The hash chain is unchanged in form and is still verified on start. There is no migration: the project is pre-release, and the default relay's current log is discarded when the new station is deployed. `--log` is replaced by `--db`.

**API additions** under the features that bring them. All bodies are raw canonical bytes of the signed object; responses are JSON as today.

| Method and path | Feature | Does |
|---|---|---|
| `PUT /v1/circles/{circle_id}/manifest` | objects | stores a sealed manifest; refuses one whose id is not the circle's |
| `GET /v1/circles/{circle_id}/manifest` | objects | serves it |
| `POST /v1/channels` | objects | stores a channel object after verifying it against the stored manifest |
| `GET /v1/channels/{channel_id}` | objects | the channel object, its current policy, and the owner's manifest in one response |
| `POST /v1/channels/{channel_id}/policy` | objects | stores a policy; with `gate`, checks its `govern` approvals |
| `POST /v1/channels/{channel_id}/grants`, `GET …/grants?kid=` | objects | stores and serves grants; with `gate`, verifies the chain |
| `POST /v1/channels/{channel_id}/revocations` | objects | stores one; with `gate`, verifies the signer |
| `POST /v1/channels/{channel_id}/deposits`, `GET …/deposits?to=&epoch=` | objects | opaque blobs |
| `POST /v1/channels/{channel_id}/requests`, `GET …/requests` | objects | the join queue; `GET` needs an admin or issuer signature when gated |
| `POST /v1/channels/{channel_id}/messages`, `GET …/messages?after=&wait=` | channels | as for circles; with `gate`, the read needs `Cahoots-Gate: <token>` or a DPoP-style proof by a key with a read grant, and a post needs a valid post grant, signature, budget and quorum |
| `GET /v1/channels/{channel_id}/head` | channels | per-channel head |
| `GET /v1/info` | base | features and limits |

Gating a read by key uses the proof already in the protocol: a `cahoots/proof/v1` COSE_Sign1 over the request's hash and a relay nonce from `GET /v1/nonce`, by the member key. The iOS design's enclave relay key is not used here, for the Face ID reason in section 1.

Gate tokens reach the relay through `POST /v1/channels/{channel_id}/gates` (feature `gate`), a body signed by an admin seat carrying the token; the relay stores its hash. Retiring a link is a signed `DELETE` of the same.

**Rate limits** apply per client address as today, and with `channels` also per channel for posts, so one open group chat cannot starve the relay.

### 9. The apps

The browser app first, since the link lands there and it needs no key store work; the iPhone app adds the same screens on the same Rust surface after the iOS branch and this one merge. Screens:

- **Start a channel**, from a circle: name, a preset, and the sentence about what the relay can see. Makes the channel object, the policy, the first epoch and the first link, uploads them, and shows the link with a share sheet.
- **A channel**, as a chat screen: posts with the member's name and a mark for announcements, the policy in one line ("anyone with the link can read; only you can post"), and the owner's controls: share the link, retire it, rotate, edit the policy, members, join requests.
- **Join**, from a link: shows the channel's name and owner, what the policy lets a link holder do, and a button. Reading starts at once. If the policy lets link holders post, the app posts a join request and shows "waiting for the owner" until the grant arrives; if the owner's phone is open, that is seconds.
- **Requests**, for owners: each request with the name the joiner typed, what they asked for, and approve or refuse; a toggle to grant automatically what the policy already allows.
- **Members**, for owners: each grant, its actions and expiry, and revoke.

Copy follows the project's rules: plain words, what the thing does, no coined names. The channel screen's label is "end-to-end encrypted; the relay keeps ciphertext only; anyone with the link can read", with the last clause following the policy.

Universal links, when the native app exists: the site gains an `apple-app-site-association` file for `/app/`, and the app declares the domain. The browser app reads the fragment and, on iOS without the app, shows the install prompt beside the channel.

### 10. What the relay and others can see

- The relay sees channel ids, message sizes and timing, which grant posted each message, which gate tokens read, and the join queue's names and evidence. With `gate` off it sees less: only ciphertext and objects.
- A link holder sees everything readers see. A member with a post grant sees who posts.
- The relay cannot read a message, a deposit or a policy's effect on content. There is no key path in any feature but `issuer`, which exists only in the enclave build and is pinned by measurement in the policy.
- Forward secrecy inside an epoch comes from the sender chains; post-compromise security at the next epoch comes from rotation and the fresh root. A leaked link or a leaking member ends at rotation. None of this is a pad guarantee, and the label says so.
- Sender authenticity among members is new relative to the circle's chat: every post is signed. The circle's own chat does not get this from this design.

### 11. What the later evidence kinds need

- **Circle (kind 2).** C6 checkpoints, so an issuer can tell that a key in another circle's manifest has not been expelled. The issuer fetches that circle's manifest and latest checkpoint from a relay and verifies the member key is a seat. This is how one circle vouches for its members in another's channel, and it is how two organisations get a channel each way between them.
- **Document (kind 3).** A verifier for the document proof in the issuer, a request bound to the member key, the nullifier under `scope`, and `per_evidence` of 1. The scope is a privacy choice: per channel stops linking across channels; per owner circle lets a person be one member across all that circle's channels.
- **Account (kind 4).** The enclave issuer running the OIDC or TLS check and emitting a fact that names the member key; the policy pins its measurement.
- **The issuer service.** The `issuer` feature in the TVC build, with its key in the policy's issuers list. Join requests then answer in seconds with nobody's phone open, and link counts and expiries become enforceable.
- **The default relay** runs the `objects`, `channels` and `gate` features from the first deploy of this work, so retiring a link is enforced and posts without a grant are refused.
- **Several relays.** A channel object may later list relay endpoints, and clients post to all and read from any. Nothing here precludes it.

### 12. Testing

- Vectors in `objects.json`: a channel object, a policy with each preset, a channel grant chain of depth 2, a revocation, a link deposit and a key deposit, a text message, a control message, and an announcement with two approvals, each with inputs, canonical bytes and signed form.
- `crates/chat` gains the wider-kid chain under a feature or a sibling module with its own vectors.
- The station's tests run under each feature set: base, `objects`, `channels`, `gate`. A test for `gate` posts without a grant, with a revoked grant, with a spent budget and with a short quorum, and expects each refusal.
- `MemRelay` grows the same API, so `crates/phone` and the browser app test admission, rotation and revocation without a server.
- End to end: a phone starts a channel, a second phone opens the link in Safari, the app opens and reads; the owner grants posting from the request; the second phone posts; the owner revokes and rotates; the second phone stops reading.

## Left to the builder

- Whether the version 4 message schedule lives in `crates/chat` behind a feature or in a new `crates/channel`. The latter seems cleaner given the different sender id and the signature.
- The SQLite schema's indexes.
- The exact UniFFI surface for channels in `crates/phone`, following the iOS design's pattern.
- Nothing in this design needs the owner circle's own log or relay. Every position named here (`created_log`, a policy's `log`, a grant's `cahoots-log`, an approval's `log`) is a `seq` in the channel's stream on the channel's relay, which the relay, the owner and every reader share. The owner circle may chat elsewhere or never.
- The approvals screen on the phone for quorum posts: an announcement proposed by one seat waits as a draft until the others approve from their phones; the draft's transport can be the channel's own control messages.
- Default limits: posts per channel per minute, deposit size, object sizes.
- The wording of every policy preset and the share screen's sentence, following the copy rules.

## Calls made in this document

- The member key is a software P-256 key per channel, not the Secure Enclave key, so posting and reading never prompt for Face ID; seat-key acts (policy, rotation, revocation, root grants) do, and only those. Each member also has an X-Wing key for deposits, since a signing key cannot receive and the enclave does no key agreement.
- Every log position in a channel is the channel's own `seq`, never the owner circle's log, so expiry works for a circle that chats on another relay or not at all.
- Admin seats post under a self-issued root grant, so a message's `kid` is always a member key and readers have one verification path. A quorum of 1 carries one approval.
- The relay learns gate tokens through a signed endpoint, and a link is retired the same way.

- Channel keys are drawn by the owner, not from the pad, with pad seeding as an owner's option. A channel is conventional cryptography and is labelled so.
- A member is a key, not a seat, and chains are keyed by key id. This is what lets a channel exceed 256 members and admit keys that were never in a circle.
- The link secret rides in the URL fragment and derives a seal key and a gate token separately, so the relay can gate without being able to read.
- Posting rights are grants, never implied by a link. "Anyone with the link can post" is a policy rule the issuer applies, not a property of the link.
- The issuer in the first version is the owner's phone, so admission for posting waits for the owner's app to be open. The enclave issuer is defined and deferred.
- Relay behaviour is chosen with Cargo features, and the feature that holds keys compiles only for the enclave build.
- Governance quorums are approvals carried on the object they govern, reusing the protocol's approval and the badge's assurance levels.

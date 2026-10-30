# Grants the badge consumes: an agent signs commits on a budget, without a press each time

*Written 2026-10-29 from a conversation with Conner. The badge already issues grants (version 2 grants are CBOR Web Tokens with RFC 9396 authorization details) and the laptop verifies them, but nothing consumes one. This design makes the badge itself the first consumer: a person issues an agent on their laptop a grant to sign git commits as them, with a budget and an expiry, by pressing both buttons once; after that the badge signs each commit under the grant without asking, shows what it signed, counts it in the chip, and refuses when the grant is spent, expired or revoked. The same machinery is what a phone will later use to fetch chat keys (E1), and what a hosted seat will run with an organization's policy engine beside it. It is a design to plan from, not a plan.*

## The idea

A grant is a permission a seat gives a key. Today the badge can make one and a laptop can check one, which proves the format and nothing else. The first thing to do with a grant is to let the badge honour it: an agent presents the grant and proof that it holds the key the grant names, the badge checks the grant against the request the way an authorization server checks a policy decision request, and signs. The person decided once, on the badge's screen, what the agent may do and how much; the badge enforces that decision every time, in hardware, with a counter that cannot be turned back.

Nothing here is a new protocol. The token is a CWT (RFC 8392) signed as COSE_Sign1 (RFC 9052), its scope is an RFC 9396 `authorization_details` array, its holder is bound by an RFC 8747 `cnf` key, its lifetime is a standard `exp` and a budget, and the badge's check is shaped like an AuthZEN evaluation: subject, action, resource, context in; a decision out. What is Cahoots-specific is the detail type, the consent screen, and where the count lives.

## Goals

- A person issues a grant on the badge with both buttons, from a request the agent made, after reading on the screen who is asking, what for, as whom, how many, until when.
- The agent then gets commits signed by the seat key, under the grant, with no press, each one shown briefly on the badge and logged.
- A request outside the grant (another author, another action, a spent or expired or revoked grant, a wrong key) is refused or falls back to the ordinary prompt, and the badge says which.
- The count of uses lives in the chip as a monotonic counter, so a restored flash image cannot refund a grant.
- The grant survives a reboot and an unplug, and ends by its budget, its expiry, or a revocation on the badge.
- A grant is readable by anyone with an OAuth or CWT verifier: who issued it, to whom, for what, until when.
- The whole path is tested on the laptop with the simulated badge and the software chip, and runs on the bench.
- The agent that gets the first grant is the coding agent on Conner's laptop, with a budget of commits, as dog food.

## Non-goals, for this version

- A grant that lets anyone but the badge's own seat sign: no delegation chains in this version (the depth claim is 0), no hosted seat consuming a grant yet.
- Anything but git commit signatures under a grant. SSH logins and file signatures keep asking; the detail type is designed to name them later.
- An external policy decision service in the badge's path. The evaluator is written so a hosted seat can call one; the badge never does.
- A trusted clock. The expiry is enforced against the best time the badge has (below); Roughtime or the relay's checkpoints can feed it later without changing the token.
- Rotating or re-issuing a grant automatically when it runs out. The agent asks again and the person presses again.

## Anti-goals

- The badge signing anything the person could not have read on the screen when they issued the grant. The grant's details are shown in full at issue, and every later use is checked against those same details.
- A grant that works for whoever holds the cable. Every use carries a proof by the agent's key, and the grant names that key.
- A permanent permission issued by leaving a field blank. A grant with no expiry must be asked for as "until revoked" and shown in those words.
- Silent signing. Every use is drawn on the panel and written to the log, with the count.

## What exists and is reused

- `crates/objects`: version 2 grants as CWTs with `iss`, `sub`, `cti`, `cnf` and `authorization_details` under private claim key -65539 ([protocol](../../protocol.md) section 8.3). The signer and the verifier.
- `crates/badge/src/apps.rs`: `BadgeApps`, which draws a question for each chip request and signs with the seat key after Both; the git commit profile (`screens::git_signature`), which reads the author and the subject from the commit text the laptop sends and checks that text against the hash it signs.
- `crates/chip`: the `ChipOps` trait with `put_counter`, `counter_next` and `exists`, implemented by the software chip and by the SE051 driver, whose counters were confirmed on the bench on 2026-10-29 to go up by one and never down.
- `crates/custodian`: the `Chip` request family on the custodian wire protocol; `cahoots-host agent` and `cahoots-host-git-sign`, which already carry a commit's text to the badge through the `sign-with-content@cahoots` agent extension.
- The badge's chip record, which says which chip objects belong to which circle, and the many-circles records in flash.

## The design

### 1. The grant

A version 2 grant as today, with three additions.

| Key | Claim | Value |
|---|---|---|
| 3 | `aud` (RFC 8392) | tstr `cahoots:seat:<circle_id hex>:<seat>`: the badge's own seat. Only that badge honours it. |
| 4 | `exp` (RFC 8392) | uint, seconds since the epoch; absent only for a standing grant (section 5) |
| -65539 | `authorization_details` | one detail of type `cahoots:sign`, below |

The `cahoots:sign` detail:

```json
{"type": "cahoots:sign",
 "actions": ["git-commit"],
 "author": "Conner Swann <me@connerswann.me>",
 "locations": ["github.com/yourbuddyconner/cahoots"],
 "agent": "claude on connerswann-mbp",
 "budget": 50}
```

`type`, `actions` and `locations` are RFC 9396's own fields; `author`, `agent` and `budget` are type-specific, which the RFC allows. `actions` is one of `git-commit`, `ssh-login`, `file`, and this version accepts only `git-commit`. `author` is the exact `Name <email>` the commit's author line must carry. `locations` is optional; when present, the commit must come from a repository whose remote matches (section 3 says how the badge learns it). `agent` is a name the person typed, shown on the screen, and nothing checks it. `budget` is the number of signatures, 1 or more, and is required.

`cnf` is the agent's key, a P-256 key the agent made in the laptop's keychain. `sub` is that key's id, as today. `iss` names the circle and the signature is the seat key's, as today. The depth is 0: the agent may not delegate.

A grant is one of two kinds, told apart by its details: a **pad grant** has purposes and the trailing `cahoots:pad` budget detail and a `cahoots-log` claim, as today; a **sign grant** has exactly one `cahoots:sign` detail, no pad detail, and no `cahoots-log` claim (a badge in a room has no log position to bind it to; its lifetime is `exp` and the budget). The decoder in `crates/objects` is generalised so the log claim and the pad detail are required for a pad grant and absent for a sign grant, and a claims set that mixes the two, or has neither, is refused. The existing `GrantSign` request, whose screen shows purposes and a pad budget, refuses a sign-shaped grant: a sign grant is only ever issued through `GrantRequest` and its own screen, so nothing shows a person a pad screen for a permission to sign.

The private claim key for `authorization_details` stays -65539 until IANA registers one; the token's description in the protocol says so, and moving to a registered key changes a number and nothing else.

### 2. Issuing: the consent profile

The agent asks with a new chip request, `GrantRequest`, carrying the detail map it wants, its `cnf` key, the `exp` it wants (or `standing: true`), and the laptop's current time. The badge:

1. Refuses without asking if the detail has an unknown field, an action other than `git-commit`, no budget, a budget over the cap (section 6), or no `exp` and no `standing`.
2. Draws the grant profile, M4h's consent screen for a grant, with the fields it will enforce, in words a person reads:

   ```
   let an agent sign
   claude on connerswann-mbp (laptop)
   git commits as
   Conner Swann <me@…>
   50 of them
   until 29 Nov (laptop)
   refuse   A+B allow   refuse
   ```

   The agent's name and the expiry are marked "(laptop)": the badge cannot check either. The author is not marked, because the badge will check every commit against it. "until revoked" replaces the date for a standing grant.
3. On Both, makes the grant: signs the CWT with the seat key, creates the grant's counter in the chip at zero (section 4), writes the grant's record to flash (section 5), and answers with the signed grant and the record's id. On Left or Right, answers `Denied`.

The request goes through the ordinary `Chip` request path and the ordinary gesture rules, including the one that throws away a gesture seen while the prompt was being drawn.

Refusals that are not a person's "no": a ninth grant when the badge holds `GRANTS_MAX`, a budget over the cap, or an unknown field answer `Denied` with the reason in the log and in the laptop's message. `Busy` keeps its meaning of "ask again"; a full badge is not that.

### 3. Using: the evaluator

The agent's git signing helper, when it holds a grant for the key git is signing with, sends `SshSign` as today plus two fields: `grant`, the signed CWT, and `proof`, a COSE_Sign1 by the agent's key over the SHA-256 of the request's `data` and a nonce the badge issued (section 3.1). The badge evaluates before it draws anything:

| Check | Refusal |
|---|---|
| The grant's signature verifies under this badge's seat key for this circle, and `aud` is this seat | `Unparseable` |
| `cti` matches a grant record on this badge that is not revoked | `Denied` ("grant revoked") |
| `exp`, when present, is later than the badge's time (section 5) | `Denied` ("grant expired"), and the record and counter are deleted |
| The proof verifies under the `cnf` key over this request's `data` and the current nonce | `Unparseable` |
| The request is a git commit signature (namespace `git`) with the commit text, and the commit's author line is exactly the detail's `author` | falls back to the ordinary prompt, with the reason logged |
| `locations`, when present, matches the repository the helper named (below) | falls back to the ordinary prompt |
| The chip counter for the grant is below `budget` | `Denied` ("grant spent"), and the record and counter are deleted |

When every check passes, the badge increments the counter, signs, answers with the signature and `remaining`, and draws for one refresh:

```
signed for claude
Fix the parser for long names
37 left of 50
```

then returns to its page. The line goes to the log as `grant <cti short>: signed git commit "<subject>", 37 left`. The badge does not wait for a press.

The repository: git does not hand the signing program the remote. The helper reads `git config --get remote.origin.url` of the repository it runs in and sends it in the request as `location`, which the badge compares to the grant's `locations` after normalising scheme and `.git`. It is the laptop's word, like the host name of an SSH login, and the screen says nothing about it; a grant with no `locations` skips the check. This is the weakest check in the table and is stated as such.

The evaluator is one function, `evaluate(subject, action, resource, context) -> Decision`, in `crates/objects/src/authz.rs`, beside the grant types and with no dependency on any board, where the subject is the `cnf` key id, the action is the detail's action, the resource is the circle and the location, and the context is the author, the time and the count. Its request and response types follow the AuthZEN Authorization API's shapes, so the same function runs in a hosted seat and can be preceded there by a call to an external decision service. The badge calls it from `crates/badge/src/grants.rs`, which owns the records, the counters and the screens, and never makes that external call.

#### 3.1 The nonce

A proof must not be replayable. The badge keeps one 16-byte nonce per link, fresh at every boot and after every use under a grant, and answers `Identify` with it. The helper includes it in the proof; the badge refuses a proof over any other nonce. A laptop that reconnects asks `Identify` first anyway.

### 4. The counter

Each grant has a counter object in the chip, at a new key kind `Grant` with the grant's record slot as the id, created at zero when the grant is issued and incremented before each signature. The count is read from the chip, never from flash: `ChipOps` gains `counter_get`, a read that does not increment (the SE051 driver's object read, which the bench already used to read counters back), implemented by the software chip and the driver, and the bench script reads a grant's counter back with it. A badge without a chip (the simulator, the Nano) keeps the count in the grant's record, and its `Identify` answer says so in its assurance, the way the pad's tier does.

A counter is deleted when its grant is revoked, expires, or is spent. At boot the badge sweeps: a counter with no record, or a record with no counter, is deleted along with its partner, the way stray chip slots are handled today.

### 5. Lifetime: records, time, revocation

**Records.** A grant's record is `{cti, circle, seat, slot, cnf key id, detail, exp, revoked}` in a new flash region beside the chip record, at most `GRANTS_MAX` (section 6) entries. Records survive reboot and unplug: a standing grant that lapsed every morning would not be one. The protection against a lost badge is that the agent's key is on the laptop, so an attacker needs both, and that every grant can be revoked from the badge.

**Time.** The badge has a timer, not a clock. It keeps `known_time`: the latest laptop time it has ever been told, stored in flash, plus the uptime since it was told. The two requests this design adds or extends, `GrantRequest` and `SshSign`, carry an optional `time` field with the laptop's time (the rest of the custodian wire is unchanged, so no other vector moves); the badge raises `known_time` when a later time arrives and never lowers it. The stored value is written when a grant is issued or used and the stored time is more than an hour behind, so the flash is not worn by every commit. An agent can therefore make the badge believe it is later, which expires grants sooner, and never earlier. After a reboot the badge's time starts at the stored value, behind the truth until the next laptop message. The consent screen marks the expiry "(laptop)" for this reason. When a trusted time source exists (Roughtime answers verified against a pinned key, or the relay's signed checkpoints), it feeds `known_time` and the token does not change.

**Revocation.** A new menu page, `grants`, lists the live grants of the current circle: agent name, action, used of budget, expiry. Both on one asks "revoke this grant?" and Both again deletes its counter and marks its record revoked. The agent's next use is refused as revoked. A revoked record is kept (so its `cti` stays refused) until the sweep at boot finds it expired or the region is full, when the oldest revoked records go first.

**Standing grants.** A request with `standing: true` and no `exp` is shown as "until revoked" and recorded with no expiry. It is the only way to issue a grant with no end, and the screen says it.

### 6. Limits

- `GRANTS_MAX`: 8 live grants per badge in this version, which bounds the chip objects and the flash region. A ninth request is refused with `Denied` and the reason logged.
- `budget` at most 1000.
- The grant plus the proof plus the commit text must fit the 16 KiB request cap; a commit whose text is too long keeps failing as today.

### 7. The agent side

- `cahoots-host grant request --serial … --circle sf --agent "claude on connerswann-mbp" --author "Conner Swann <me@connerswann.me>" --budget 50 --until 2026-11-29 [--location github.com/yourbuddyconner/cahoots]` makes the agent's key in the keychain if there is none, sends `GrantRequest`, and stores the grant beside the key.
- `cahoots-host grant list` prints the agent's grants with what the badge last said was left; `cahoots-host grant show <file>` prints any grant as JSON, the way `card show` does.
- `cahoots-host-git-sign` and `cahoots-host agent` look for a grant for the circle's key when git signs, and send it with the proof. With no grant, or a refusal that falls back, the badge asks as it does today. The helper prints "signed under a grant, 37 left" or the badge's reason.
- `cahoots-host grant verify <file>` checks a grant as a plain CWT verifier would, against the manifest, and prints the details: the proof that nothing in the token needs Cahoots to read.

### 8. Tests

- `crates/badge/tests/grants.rs` with the fake board and the software chip: issue after Both; refuse on Left; a use that signs without a prompt and increments the chip counter; refusals for the wrong author, the wrong key, a replayed proof, a spent budget, a past expiry, a revoked grant, a standing grant that never expires; the sweep at boot; a restored flash image (the record copied back after a use) still refused because the chip's counter did not move; a badge without a chip counting in its record.
- `crates/host/tests/grant_agent.rs` with the simulated badge: `grant request`, then two `git commit`s through `cahoots-host-git-sign` with no press, the badge's screen showing the count, then a revoke and a third commit falling back to the prompt.
- The bench script gains the same sequence on the real badge, with the chip's counter read back.

## Constraints

- Copy rules as the repository: plain words on every screen, the fields a person can read, "(laptop)" on anything the badge cannot check.
- Nothing cryptographic outside the Rust crates; the keychain holds the agent's key, and the helper uses it through the agent so the key is never in a git process.
- The protocol document gains the `aud`, `exp` and `cahoots:sign` definitions in section 8.3 and the two new chip requests in section 6, and the vectors gain a grant of this kind.

## Left to the builder

- The flash layout of the grants region on each board, within the regions the firmware already reserves.
- The exact COSE structure of the proof (a COSE_Sign1 with detached payload is the obvious choice).
- The keychain API on macOS for the agent's key, and a file fallback on Linux.
- How `remaining` rides on the `Signature` response without breaking the existing decoders.
- The agent's own key id naming in `ssh-add -L` style listings, if it appears there at all.

## Decided, and why

- The badge consumes the grant it issued, with `aud` naming itself: the first consumer should be the one we test on the bench, and the budget lives where the enforcement is.
- The count is a chip counter: a count in flash is refunded by restoring an image; the chip's counters only go up, and the bench confirmed it.
- `exp` is enforced against a monotonic best-known time rather than skipped: a wrong time can only expire a grant early, and expired grants must leave the chip rather than fill it.
- No expiry only as "until revoked", in those words, on the screen: a permanent permission is never the default of a blank field.
- Every use is drawn and logged but not blocking: the press was the decision; the draw is so the person can see the agent at work.
- The evaluator has AuthZEN's shape and lives in a library: the hosted seat runs the same code with an organization's decision service beside it, and the badge never phones anyone.
- The repository check is the laptop's word and is documented as the weakest check: git gives the signer nothing better, and the author line, which the badge reads from the text it signs, is the check that holds.

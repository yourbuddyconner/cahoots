# Circle apps: proposal and implementation gaps

**Status: proposed.** This page restores and develops the circle-app direction from the earlier use-cases document. It describes what to build after the existing ceremony and messaging tools; it does not describe an implemented runtime.

For tools you can use now, see [Apps for your circle](apps.md).

## The product direction

A circle should become a place to do things together. Creating it establishes the group; returning to it should show useful apps and shared activity.

Start with the existing tools: chat, short messages, and call checks. Give desktop integrations a clear setup path. Add installable shared apps only when the host can enforce their permissions and account for their use of circle resources.

A future app catalog is a discovery surface. A per-circle list records which apps that particular group uses. Those are distinct from the list of circles and from the ceremony.

## What the repository already contains

| Capability | Evidence | What it establishes |
|---|---|---|
| Built-in circle tools | [Browser client](../web/app/app.js), particularly its saved-circle screen | Fixed routes for messages, chat, hello words, and history. No app registry or installation lifecycle. |
| Protocol WebAssembly | [WASM bindings](../crates/wasm/src/lib.rs) | Compiles the existing ceremony and messaging code for the browser. It does not host third-party modules. |
| External integrations | [Adapters](../crates/ADAPTERS.md) and [WireGuard CLI](../crates/wireguard/src/main.rs) | Useful tools built around a file-backed custodian/provider. No browser-to-tool handoff is exposed in the circle UI. |
| Badge operations | [BadgeApps](../crates/badge/src/apps.rs) | A dispatcher for defined chip requests with consent checks, not an arbitrary app loader. |
| Signing and card workflows | [Host tests](../crates/host/tests/) | Test coverage exists for agent, git-signing, attestation, card and grant workflows. This inspection did not rerun those tests or validate real hardware. |
| Ecosystem roadmap | [Apps milestones A1–A4](plan.md#apps) | Distribution and runtime are planned. Hello words is the implemented part of A1. |

The file-backed provider, browser IndexedDB store, and badge-backed custodian are different access paths. Reusing protocol concepts does not make them one connected user experience.

## The original shared-app model

The proposal is a deterministic module with a reducer:

```text
reduce(state, authenticated_event) -> next_state
```

Members process the same ordered events to reach the same application state. A separate rendering interface presents that state. The exact module ABI, UI interface, and permissions have not been specified or implemented.

An app would be identified by its content hash and version. The group would explicitly enable that version; a silent code update would require new agreement.

The circle host owns keys, spending, and the ledger. An app can request an allowed action; it never receives raw pad bytes. Apps do not run during the ceremony or replace the check-word and consent screens.

## Work required before third-party apps

1. **An app manifest and event format.** Define app IDs, versions, allowed capabilities, event namespaces, state migration, and how the group enables or disables an app. Existing network control messages are not a complete app lifecycle protocol.
2. **A host API.** Expose narrowly scoped storage, message reads, and sends. All sends must use the same custodian and ledger; copying the pad into each app is unsafe.
3. **An actual sandbox.** Restrict network access, storage, memory, execution time, and UI privileges. Keeping pad bytes private does not stop an app from leaking plaintext it can read. Decrypted data access needs permission too.
4. **Resource accounting.** Define byte budgets and send confirmation rules. Pure-pad messages and pad-seeded sessions have different costs; “one app action” is not a fixed pad cost.
5. **Agreement and replay.** Define the event history an app accepts, how members detect conflicting histories, and how offline members catch up. A single ordinary relay’s ordering is not independently verified consensus.
6. **Member identity where needed.** Group authentication alone does not establish which member sent an event. A one-person-one-vote poll needs an individually authenticated event scheme and a replay/membership policy.
7. **Distribution and compatibility.** Decide how modules are fetched and verified, which hosts can run them, and how updates and removal affect existing state. None of this exists as an installable SDK today.

The [chat specification](chat.md#4-security-properties) explicitly excludes sender authenticity among members. Apps must not infer an individual vote or approval from chat’s sender label.

## A practical sequence

### First: make existing tools feel connected

Turn the saved-circle screen into a home for that circle’s tools. Lead with Chat and Call checks, keep short messages available, and show what needs setup. Give Files, Private network, and Signing accurate integration status and actionable setup instructions instead of inactive launch buttons.

This is the smallest product step. It makes the existing value easier to see without requiring a plugin runtime.

### Next: connect one external tool end to end

WireGuard is a strong candidate because its CLI already has a two-host CI scenario. Establish a custodian/authorization path and a guided setup flow from one existing circle. Do not export the same live send state to multiple independent spenders.

The milestone is a person completing the flow, not another adapter crate.

### Then: build one shared app

A shared board is a candidate for the first experiment, with explicit group-edit semantics. Test app enablement, scoped encrypted events, deterministic replay, offline catch-up, and the host permission boundary.

The earlier proposal nominated a vote. That remains useful, but a meaningful member-counted vote needs sender authenticity and membership handling first. A demo tally must not be presented as a trustworthy approval mechanism.

Only after this path works should Cahoots promise an installable app ecosystem or stable third-party SDK.


## Recommended client roles

This recommendation reflects the product discussion after the initial ecosystem audit. It is a proposed direction, not implemented enrollment behavior.

- **Phone and browser:** independent, useful chat clients. A phone should not need a live badge connection for routine messaging. Keep browser-created software circles available for demos and ordinary software-backed use, with their actual protection stated.
- **Badge:** authorize sensitive desktop operations and, once membership changes exist, help approve enrollment. USB is the first desktop connection. Do not make phone-to-badge pairing a prerequisite for the chat experience.
- **Cahoots-hosted relay:** a default delivery service so a new circle can start chatting without server setup. Retain an override for other relays. A default address is configuration, not a source of circle keys or authority over membership.

### Enrollment is a separate feature

Adding another client for an existing person, adding a new person, and supplying entropy are different operations. The current protocol fixes seat count, pad slices, and a signed manifest during the ceremony. Copying an existing member's pad and ledger to a new client is not a safe enrollment mechanism.

Start with narrowly scoped client enrollment for an existing member. A badge approves a specific client public key and requested access; the client receives only the authorized application keys. The group must define who can authorize this, how credentials expire or are revoked, and how encryption keys rotate. New-person membership changes need the circle's membership policy and fresh keying for future messages. They are not an arbitrary single badge's decision unless that authority has been delegated.

Adding a badge later cannot revoke secret copies already held by software devices. Moving a software circle to badge-held custody needs a fresh cryptographic generation and a supported transition. The product may preserve its conversation identity while making the change explicit; it must not claim the old pad became hardware-protected.

### Optical transport

Animated QR already exists for ceremony exchanges and receiving extra randomness. It can be reused as a transport for a proposed enrollment protocol: the recipient displays a public-key request, an authorized device approves it, and returns an encrypted, recipient-bound response. The exact request, consent, expiry, replay prevention, and verification rules still need design.

Randomness is not a membership credential. Phone-generated bytes remain software randomness, even when sent optically; the receiving app should mix additional entropy with its own source. A camera recording must not reveal the only copy of a secret being provisioned. An authorized application-key transfer is therefore a different payload from an entropy contribution.

### Delivery first, enrollment next

The first milestone should be two independently created software clients completing a ceremony and chatting through a default hosted relay without entering a URL. The relay still needs a deployment target, HTTPS, persistent storage, health checks, request limits, and a defined retention policy. The current Dockerfile also pins Rust 1.88 while the repository pins 1.94; align and build it before deployment.

Next, test the USB badge path against a simulated badge, then on hardware: scoped client authorization, key delivery, unplugged chat, rotation, and refusal after revocation. Broader app distribution follows this connected user experience rather than preceding it.

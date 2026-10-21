# What Cahoots is concretely for

The ceremony produces one thing: a pile of random bytes that a small group shares and that nobody else has seen. This document lists what that pile is good for, in order from the obvious to the ambitious. The second half is a draft design for using a circle to bootstrap a network: WireGuard first, then physical mesh networks.

## 1. The two modes, restated

**Pure-pad mode** (`docs/protocol.md` section 4). One pad byte per message byte, plus 32 bytes for the one-time tag. Confidentiality and integrity do not depend on any math. Fit for short, high-value messages: a few hundred bytes at a time. A 256 KiB pad holds roughly 2,000 bucket-padded text messages across the circle.

**PSK mode** (handoff section 10). 32 pad bytes per session, mixed into an ordinary key exchange with HKDF. The session stays confidential if the math holds **or** the pad stays secret. A 256 KiB pad holds about 8,000 sessions of any size. This is the mode for files, tunnels, and anything bigger than a text message.

The two modes share a pad, a ledger, and a circle ID. A circle can use both.

## 2. Concrete uses

### 2.1 Group chat that cannot be broken later

The meetup demo. Members post ciphertext to the Numbers Station. Harvested ciphertext stays useless forever, including against quantum computers. This is the only use that is information-theoretically secure end to end.

### 2.2 Approvals that do not depend on public-key crypto

An `approve` message carries the SHA-256 of an action. A verifier that holds the pad checks the one-time tag. A Turnkey policy can require one of these alongside a passkey (handoff 10.4). In a world where signatures break, the approval still stands.

### 2.3 Encrypting files to a circle

`age -r cahoots:<circle>` wraps the file key with a 32-byte pad PSK. Two laptops that share a circle can exchange gigabytes with one ceremony. Each file spends a fresh offset, so a leaked file key reveals nothing about other files.

### 2.4 Hedging ordinary tunnels: WireGuard, TLS, MLS

Every one of these protocols already has a pre-shared-key slot that the designers added for exactly this reason: to hedge against the public-key layer failing. The slots are empty in almost every deployment because nobody has a good way to agree on the key. Cahoots fills the slot. The in-person ceremony is the key agreement.

WireGuard's `PresharedKey` is the clearest case. The WireGuard paper describes it as a hedge against quantum attacks on Curve25519, mixed into the Noise handshake as the `psk2` modifier. Operators mostly leave it unset. See section 3.

### 2.5 Recovery circles

A dealer device Shamir-splits a recovery secret at the ceremony and hands each member a share over the touch link (handoff 10.5). Recovery means k people meet again, or send their shares as pure-pad messages. Social recovery backed by in-person trust.

### 2.6 Bootstrapping networks

This is the use the rest of this document is about. The claim is simple: **a circle is a trust root for a network**, built in a room instead of downloaded from a certificate authority.

## 3. Bootstrapping a WireGuard network

### 3.1 The problem a circle solves

To set up a WireGuard tunnel between two machines, you need three things: each side's public key, each side's endpoint and allowed IPs, and, optionally, a pre-shared key. All three travel over some channel before the tunnel exists. In practice that channel is a chat message, a wiki page, or a coordination server. Each of those is a place where a key can be swapped or read.

A circle removes the channel. Members touch devices once. From then on they have an authenticated, confidential path to each other that does not depend on any network or any public key.

### 3.2 How it works

Three message types do the whole job. They are circle messages, so they can be posted anywhere, including the Numbers Station, and only circle members can read or forge them.

1. **Peer announcement** (`control` message, subtype `peer`). A member posts its WireGuard public key, its endpoint, and its allowed IPs. Every member now knows every peer. The announcement is authenticated by the pad, so a swapped public key is detected.
2. **PSK issue** (`control`, subtype `psk`). To set up or rotate the tunnel to peer B, member A spends 32 bytes from its slice and posts the handle `(circle_id, slice, offset)` and the pair it is for. B calls `psk_for(handle)` and spends the same bytes. Both sides now hold the same `PresharedKey`. The handle is a pointer, not a secret. It can travel in the clear.
3. **Rotation** is a new PSK issue on a schedule. The `cahoots-wg` helper renders the `wg set` command and the `[Peer]` config block.

The first tunnel needs no network at all: the announcements and the first PSK issues can be exchanged over the touch link at the ceremony itself. After that, rotation handles travel inside the existing tunnel.

### 3.3 Pad budget

| Circle | Peer pairs | Rotation | PSKs per year | Pad needed |
|---|---|---|---|---|
| 4 nodes | 6 | weekly | 312 | 10 KiB |
| 8 nodes | 28 | daily | 10,220 | 320 KiB |
| 8 nodes | 28 | weekly | 1,456 | 46 KiB |

A 256 KiB pad lasts years at weekly rotation. On an ESP32-S3 with 8 MB of flash the pad can be several MiB. The warning fires at 10% remaining, and the fix is a new ceremony.

### 3.4 What a member can and cannot do

Everyone in a circle holds the whole pad. That has two consequences for networks.

- **A member can read any PSK issued in the circle.** For WireGuard this does not break the tunnel today, because the handshake still needs the Curve25519 secrets. It does mean the pad hedge protects the fleet from outsiders, not members from each other.
- **A member can forge another member's announcement.** The circle tag proves that some member wrote a message, not which one.

Both are fine when one operator runs every node. Call this the **fleet profile**: a person's home lab, a company's edge boxes, a research group's sensors. Both are a problem when the nodes belong to different people who do not fully trust each other. That needs the **federation profile** in section 5.

## 4. Bootstrapping physical mesh networks

### 4.1 Why mesh networks are the right target

Mesh networks have the bootstrap problem in its purest form. There is no internet to fetch a certificate from. Nodes join and leave. Links are slow, lossy, and sometimes one-way. Deployments happen in the field, by people with a bag of hardware. And the communities that build them, off-grid radio, disaster response, neighbourhood networks, already meet in person.

The deployment story writes itself. The install crew runs the ceremony at the trailhead or in the van. Each node's badge, or the node itself, holds a slice. Nodes then come up on the roof with every key they need and no network trust at all.

### 4.2 Where the pad bytes go

Each mesh stack has a slot that a circle can fill. The slot decides the mode.

| Stack | Slot | Mode | Notes |
|---|---|---|---|
| WireGuard overlay (wg-mesh, hand-rolled, or as the transport under batman-adv) | `PresharedKey` per peer pair | PSK | Section 3 applies unchanged. |
| 802.11s Wi-Fi mesh with SAE | Mesh password | PSK, via HKDF | One password per mesh. Rotation requires a coordinated restart, so rotate rarely. |
| batman-adv over WPA3 links | WPA3 passphrase per link | PSK | Same as above, per adjacent pair. |
| Meshtastic / LoRa | Channel key (AES-128/256) | PSK | The channel key is the whole security of a Meshtastic channel. Today it is shared by QR code or URL. A pad PSK plus rotation is a strict improvement. |
| Reticulum | Link identity and pre-shared link key | PSK | Identities announced as peer messages; link PSKs issued per pair. |
| Yggdrasil, cjdns | Peer password / allowed public keys | PSK and announcements | The circle distributes the allowed-keys list. |
| LoRa point-to-point, APRS-style text | Message payload | **Pure pad** | Short payloads and no PKI: the pure-pad mode fits LoRa exactly. See 4.4. |

### 4.3 The mesh-specific gaps

Three things make mesh bootstrap harder than WireGuard between two servers.

1. **Nodes have no screen and no operator at join time.** A node mounted on a roof cannot read check words. The badge does the ceremony; the node receives its slice afterwards over USB or the touch link. That transfer is a pad-to-node provisioning step that the protocol needs to name. Draft: the node's slice is written to the node's pad partition and the badge erases its copy, the same move as a recovery share (handoff 10.5).
2. **Links are lossy.** Circle messages over LoRa need the fountain-coded framing the phone client uses for QR. The `raptorq` packets are already sized for that.
3. **Nodes join after the ceremony.** Adding a member without a new ceremony is out of scope for v1 (handoff section 5). The practical answer today is to run the ceremony with a spare slice held by the operator's badge and hand that slice to the new node. The spare is a planned member, not an extension of the pad.

### 4.4 The Numbers Station over LoRa

A Cahoots badge with a LoRa radio is a numbers station in the literal sense. It broadcasts pad-encrypted text that anyone can hear and only the circle can read. Pure-pad messages are small, carry their own integrity, and need no handshake. This is the most on-brand demo the project can do. It is listed as a later milestone because the touch link and the device policy are no-radio during the ceremony, and a LoRa radio on the badge must be verifiably off while contributions are generated.

## 5. Spec additions (draft, not yet normative)

These extend `docs/protocol.md`. They are marked draft until the first `cahoots-wg` bootstrap works end to end.

### 5.1 Control message subtypes

A `control` message's plaintext begins with one subtype byte.

| Subtype | Name | Payload |
|---|---|---|
| `0` | `peer` | `proto (u8) | pubkey_len (u8) | pubkey | endpoint_len (u8) | endpoint (UTF-8) | allowed_len (u8) | allowed_ips (UTF-8)`. `proto`: `0` WireGuard, `1` Reticulum, `2` Yggdrasil, `3` Meshtastic. |
| `1` | `psk` | `purpose (u8) | peer_a (u8) | peer_b (u8) | offset (u32)`. The issuing member's slice is the message's own slice. `purpose`: `0` WireGuard, `1` TLS, `2` MLS, `3` Wi-Fi/802.11s, `4` Meshtastic, `5` other. |
| `2` | `rotate` | `purpose (u8) | peer_a (u8) | peer_b (u8) | not_after (u64 unix seconds)` |
| `3` | `slice-transfer` | `member (u8)`. Announces that the member's slice now lives on a node, not a badge. Informational. |

The `context` bound into the PSK HKDF salt (handoff 10.3) is the `purpose` byte followed by the two peer indices, so a PSK issued for WireGuard between A and B cannot be reused for TLS or for another pair.

### 5.2 The fleet profile and the federation profile

- **Fleet profile** (v1): one group pad, as specified. One operator, or a group that trusts every member with every tunnel. All of sections 3 and 4 work in this profile.
- **Federation profile** (later): at the ceremony, every pair of devices also touches directly and runs a two-member sub-ceremony. Each pair holds a **pairwise pad** that nobody else has. Pairwise pads give sender authenticity (handoff 3.8 lists this as out of scope for v1) and keep each tunnel's PSK private to its two ends. The cost is `n(n-1)/2` touches, which is 28 for eight members. The group pad stays for circle-wide messages.

The federation profile is the version that scales from "my boxes" to "our neighbourhood network".

### 5.3 Provisioning a slice to a headless node

1. The badge completes the ceremony and holds the full pad.
2. Over USB or the touch link, the badge writes the pad and its ledger to the node's pad partition and reads it back.
3. The badge erases its copy of the pad and verifies the erase.
4. The badge posts a `slice-transfer` control message.

A node provisioned this way holds the whole pad, as any member does. The federation profile changes this to the node's pairwise pads only.

## 6. The thesis, restated for networks

Every network today bottoms out in a trust root someone downloaded: a CA bundle, a coordination server's key, a vendor's signing key. Cahoots offers a trust root you built with people you could see. In the Pessiland story, pad couriers are the new certificate authorities. For a mesh network on a hillside, that is not a thought experiment. It is the only option that works with no uplink.

Turnkey's piece is unchanged: a node that lives in the cloud, a gateway or exit node, holds its slice in TVC under a pinned policy, and every PSK it issues or accepts is in the anchored log.

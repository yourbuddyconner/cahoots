# What you can do with a circle

See [Apps for your circle](apps.md) for the tools available after setup and their implementation status.

A circle shares a secret created during an in-person meeting. Different applications use that secret in different ways.

## Exchange short messages

Use **Send a message** in the browser app to encrypt short text directly with unused pad bytes. Pass the encrypted code or text to another member, who reads it once.

This needs no relay. It consumes pad space for each message and is distinct from the app’s ongoing chat mode.

Start with [Create a circle](ceremony-guide.md), then [Use the browser app](browser.md).

## Keep a group chat

Open **Chat** to send messages through a relay. The pad seeds chat keys, and conventional encryption protects the conversation.

The relay needs to be reachable by every member. It holds ciphertext, not the circle’s keys. See [Run a relay](relay.md).

## Check someone on a call

**Hello words** gives each pair of members words to compare during a phone or video call. Move to a fresh hello number together for the next check.

This verifies access to the pair’s key. It cannot distinguish the owner from someone using their unlocked device. See [the browser guide](browser.md#check-someone-on-a-call).

## Connect machines or encrypt files

Developer integrations use the secret as an input to existing cryptographic tools:

- **WireGuard:** exchange preshared keys and peer announcements for a tunnel.
- **age:** encrypt files to a circle using the Cahoots plugin.
- **HPKE:** use the circle secret in HPKE’s preshared-key mode.

These are conventional encryption integrations, not one-time-pad encryption. Read the [adapter reference](../crates/ADAPTERS.md) for commands, status, and limitations. TLS and MLS adapters are not implemented.

## What is still a design

Hardware-backed approvals, hosted agents, attested sensors, and programmable circle apps are development directions. They are not interchangeable with the working browser flows above.

See [Project status](status.md) for a short account of what is available, or the [roadmap](plan.md) for the development detail.

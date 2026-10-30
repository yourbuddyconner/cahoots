# Use the browser app

The [Cahoots app](https://cahoots.network/app/) runs on phones and laptops. It stores circles and message history in that browser on that device.

First, [create a circle](ceremony-guide.md) with someone in person.

## Find your way around

The home screen lists the circles saved in this browser. Open one to enter its conversation directly.

On a desktop, the circle switcher and tool navigation stay on the left, with people beside the conversation. On a phone, use the **Conversation**, **People**, and **Network** navigation row. **Circle tools** opens short messages, history, and hello words without a separate app directory.

The **Network** page describes the WireGuard desktop integration and its setup. The browser does not yet load machine lists or manage tunnels.

## Put names to seats

Open **People** in a circle and select **Name this person**. Save a name you recognize; it will also appear in chat and hello words on this device.

Names are local labels. They are not shared with other members and do not verify anyone’s identity. You can change one with **Edit name**, or clear it to return to the seat number. Changing the circle’s relay keeps these names.

## Send a short message

1. Open a saved circle.
2. Select **Send a message** from the sidebar, or from **Circle tools** on a phone, and enter your text.
3. Pass the encrypted QR code or copyable text to another member.
4. The recipient opens the same circle, selects **Read a message**, and scans or pastes it.

This uses **pure-pad mode**: fresh secret bytes encrypt each message and are then consumed. It works without a relay. Passing a message over another service does not give that service the circle’s secret.

A message can be read once by each receiving device. Reading it again is refused as reuse. The app keeps a local history of messages it has already read.

## Use group chat

Chat is a separate mode. It uses conventional encryption with keys seeded from the circle’s secret, so it can carry many messages without consuming one secret byte per text byte.

1. Open a saved circle. Its **Conversation** view is the group chat.
2. Type a message. It travels through the default relay, `relay.cahoots.network`.
3. To use a relay a member [runs](relay.md), set **Relay address** from the menu, and have every member of the circle set the same one.

If the status line says the relay cannot be reached, creating the circle still succeeded; delivery is the missing step. Check the address in the menu or try again later.

The relay receives encrypted messages. It does not receive the secret. It can still observe traffic and stop delivering messages.

### Rotate chat keys

An **epoch** is a period using one set of chat keys. Every member derives them from the circle root, so starting an epoch spends no secret bytes. Chat starts the first one when needed and rotates after 1,000 of your own messages in an epoch. A circle made before root epochs cannot chat. Run a new ceremony to chat in it.

Use **Start a new epoch** in the chat menu to rotate sooner. Automatic rotation after a time interval is not implemented.

## Check someone on a call

Open **Hello words** in the circle and choose the person you are calling.

The screen shows two words for you to say, two for them to say, and a hello number. Both devices must use the same number. If needed, set it from the menu.

Compare the responses. On a later call, or if something feels wrong, both people select **Next hello** and compare the new words.

Hello words verify access to the pair’s key, rather than the sound of someone’s voice. They do not protect against someone who has the other person’s unlocked device. They use a different kind of key from pure-pad messages; see [Security and limits](security.md).

## Understand local storage

A saved circle contains secret material. Chat history also contains plaintext on the device.

- Use the same browser and browser profile to return to your circles.
- Clearing site data can remove circles and history.
- The relay is not a backup of your keys and cannot restore a lost circle.
- Browser storage does not provide reliable physical erasure. A compromised browser or operating system can expose secrets and history.

Do not restore or copy a circle’s send state onto a second device and use both copies. Reusing pad ranges breaks the protocol’s security guarantees. Create a new circle when you need a fresh start.

For local builds, WebAssembly details, and exchange formats, see the [browser client README](../web/README.md).

# Use the browser app

The [Cahoots app](https://cahoots.network/app/) runs on phones and laptops. It stores circles and message history in that browser on that device.

First, [create a circle](ceremony-guide.md) with someone in person.

## Create a circle

Select **Create a circle**, give it a name, and count the people in the room. Then pick how to hold the ceremony.

- **Through the relay** is the default. Your screen shows one code. Each person joining selects **Join a circle**, scans it, and picks a seat. Their screen then shows a code of their own, and you scan each one with **Scan a joiner's code**. When everyone is scanned, select **Continue**. The rest of the ceremony goes through the relay, sealed with a key that never leaves your first code, so the relay cannot read it.
- **Codes only** works with no network. Everyone shows and scans animated codes for each step until every device has every code.

Either way, everyone reads the six words aloud at the end. If every word matches on every screen, select **They match**. If one differs, everyone starts again.

## Find your way around

The home screen lists the circles saved in this browser. Open one to enter its conversation directly.

On a desktop, the circle switcher and tool navigation stay on the left, with people beside the conversation. On a phone, use the **Conversation**, **People**, and **Network** navigation row. **Circle tools** opens short messages, history, and hello words without a separate app directory.

The **Network** page describes the WireGuard desktop integration and its setup. The browser does not yet load machine lists or manage tunnels.

## Put names to seats

Open **People** in a circle and select **Name this person**. Save a name you recognize. It will also appear in chat and hello words on this device.

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

If the status line says the relay cannot be reached, creating the circle still succeeded. Delivery is the missing step. Check the address in the menu or try again later.

The relay receives encrypted messages. It does not receive the secret. It can still observe traffic and stop delivering messages.

### Rotate chat keys

An **epoch** is a period using one set of chat keys. Every member derives them from the circle root, so starting an epoch spends no secret bytes. Chat starts the first one when needed and rotates after 1,000 of your own messages in an epoch. A circle made before root epochs cannot chat. Run a new ceremony to chat in it.

Use **Start a new epoch** in the chat menu to rotate sooner. Automatic rotation after a time interval is not implemented.

## Channels

A circle is closed: only the people at the ceremony are in it. A **channel** is how a circle reaches people who were not there. You start one from a circle and send its link. Anyone who opens the link reads the channel in their browser, without joining the circle, and can post if the channel allows it.

A channel uses conventional encryption with keys your device draws. It does not use the circle's pad. Every screen of a channel says "End-to-end encrypted; the relay keeps ciphertext only", followed by who can read and post.

A channel uses its circle's relay, and that relay must be [built for channels](relay.md#choose-what-the-relay-runs). The default relay is. If your circle's relay is not, **Start** says so.

### Start a channel

1. Open a saved circle and select **Start a channel** from the circle tools.
2. Give it a name and pick who can read and post:
   - **Announcements:** anyone with the link reads. Only you post.
   - **Announcements with approvals:** anyone with the link reads. A post goes out when enough seats of your circle approve it.
   - **Group chat:** anyone with the link reads and can ask to post.
   - **Members post:** anyone with the link reads. People you let in post.
   - **Closed:** only people you let in read and post.
3. Select **Start**.

The next screen shows the link, with **Share the link** (where the device can share) and **Copy the link**. It also says: "Anyone you forward this link to can read until you retire it." Nobody checks a link at the door, so it cannot count its readers or expire on its own.

### Run your channel

The channel's menu has the owner's controls:

- **Share the link** shows the current link. If you retired it, this makes a new one.
- **Retire the link** stops the link from working. Everyone reading by the link stops at the next keys. People you let in keep reading.
- **Start new keys** starts new channel keys now. The app also does this when you retire a link or revoke someone.
- **Policy** changes who can read and post. In a circle of more than one seat, the app refuses: changing the policy "needs their approval, which this version cannot collect yet".
- **Members** lists the people you let in. **Revoke** removes one. When the channel has a live link, revoking also retires it and shows a new one, because the person you removed still has the old link. The new screen says: "Everyone who should keep reading needs this new link." A channel with no live link just starts new keys.
- **Requests** lists people asking to post, each with the name they typed and what they asked for. **Allow to post** lets them read and post, **Allow to read** lets them read only, and **Refuse** drops the request. Turn on **Allow automatically what the policy already permits** to let in, while the channel is open on this device, everyone who came with the live link, for what the link proves: in a group chat, reading and posting. In a channel where only people you let in post, requests wait for you.

Requests wait for you. The app answers them only while the channel is open on your device.

### Post with approvals

In a channel made with **Announcements with approvals**, a post needs approvals from seats of your circle.

1. Write the post and select **Propose**. The app adds your own approval and shows a code.
2. Send the code to the other seats. Each one opens the channel on a device that holds the circle, selects **Approve a post** from the menu, pastes the code and selects **Read the code**. They check the first line it shows, select **Approve**, and send you the approval it gives them.
3. Paste the approvals and select **Post**.

Readers mark the post as an announcement only when it carries enough approvals. When a single approval is enough, **Propose** posts at once.

### Open a channel from a link

Open the link. The app shows the channel's name, the circle that owns it, and who can read and post. If people with the link may post, it asks for **Your name**. Select **Open**.

You read at once. The app removes the secret from the address bar and the browser history. If the channel lets you post, the app asks the owner and shows "Waiting for the owner to let you post." until the owner allows it. Then the message box works.

If the owner retires the link or revokes you, the channel says "This link no longer works. Ask for a new one." A new link from the owner opens the same channel again in this browser.

A link without a secret shows "Only people the owner lets in can read this channel." This version has no way to ask to be let in from such a link.

### What is stored in the browser

Each channel is saved in this browser's IndexedDB, in the `channels` store: your key for the channel, its current keys, the link's secret, your grant and the policy. Posts you have read are in the `chmsgs` store, as plaintext. Clearing site data removes them. The relay keeps only ciphertext and cannot give you back a lost channel.

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

# Use Cahoots in your browser

The [Cahoots app](https://cahoots.network/app/) works on phones and laptops. Your circles, keys, and message history stay in the browser you use. You don’t need an account.

New to Cahoots? Start with [Getting started](getting-started.md). The app also shows a short guide on first use. You can reopen it from the home screen’s **More options** menu by selecting **How Cahoots works**.

## Create or join a circle

Gather the people who want to join. One person selects **Create a circle**, enters a name, and counts everyone joining on a separate device, including themselves. This person is the **host** and uses seat 1. Each other device needs a different seat number.

Choose how to connect:

- **Through the relay** uses an internet connection. Everyone scans the host’s code and chooses a seat. The host then scans each person’s code with **Scan a joiner’s code**. When everyone is scanned, the host selects **Continue**. The relay carries the remaining encrypted setup messages.
- **Codes only** works without a network connection during setup. Everyone shows and scans animated codes at each step. Keep each animation in view until the progress bar fills. Before moving on, make sure the others have scanned your code too.

This setup is called a **ceremony**. At the end, one person reads all six words aloud while everyone checks their own screen. Select **They match** only when every word matches on every device, in the same order. If any differ, start a new ceremony together.

Keep the app open until setup finishes. A setup session that is lost after closing or reloading cannot be resumed. The group may need to start again.

## Find your way around

The home screen lists circles and channels saved in this browser. Open a circle to enter its conversation.

On a desktop, the circle switcher and tools stay on the left. On a phone, use the **Conversation**, **People**, and **Network** tabs. **Circle tools** opens hello words, short messages, history, and channel creation.

In **People**, select **Name this person** to give a seat a familiar name. Names appear in chat and hello words on your device only. Use **Edit name** to change a name, or clear it to restore the seat number. A name or decorative avatar does not verify someone’s identity.

## Chat with your circle

Open the circle’s **Conversation**, type a message, and select **Send**. Messages are encrypted on your device before the relay delivers them. The relay cannot read them, but it can see traffic and can stop delivery.

Cahoots uses `relay.cahoots.network` by default. To use another relay, open the conversation menu and select **Relay address**. Every member must use the same address. See [Running a relay](relay.md) if you want to host one.

If the relay cannot be reached, your circle still exists. Delivery needs a working connection. Check the relay address and try again later.

**Renew encryption keys** changes the keys used for future messages. It does not change the protection of messages already sent. A set of keys is called an **epoch**. Chat renews them automatically after 1,000 of your own messages in an epoch; it does not currently renew them on a timer.

A circle made with an older version may not support chat. Create a new circle together to use the current chat format.

## Check someone on a call

From **Circle tools**, open **Hello words** and select the person you’re talking to. Ask them to open your seat in the same circle.

1. Match the hello number on both screens. If one number is lower, advance that device to catch up. Don’t reuse a number from an earlier conversation.
2. Read the pair under **You say aloud**. The other person checks it against the reply shown on their screen.
3. Ask them to read their own pair. Check that you hear the pair shown under **Listen for their reply**. Don’t read this answer to them.
4. When both pairs match, each of you selects **Next hello** and confirms. This prepares fresh words for next time. It changes only the device on which it was selected.

If the words differ, check the circle, seats, and number. If they still differ, stop the check. The words check access to a key shared by your two devices. They do not protect against someone who can use the other person’s device or keys.

The hello-word menu lets you save a local name and adjust the hello number if needed. See [Security and limits](security.md) for more about what these checks can and cannot establish.

## Exchange a message without a relay

**Send a message** in Circle tools encrypts a short note that you can pass along as a code or text. This is separate from chat and works without a relay.

1. Enter the note and select **Encrypt**.
2. Let another circle member scan the code, or send them the encrypted text using another service.
3. They open the same circle, select **Read a message**, and scan or paste it.

This mode uses a **one-time pad**: it consumes a fresh part of the circle’s shared secret for each message. When your share is used up, create a new circle together to send more messages this way. Chat uses a different encryption method and does not consume this pad for each message.

Each receiving device can open a one-time-pad message once. Later attempts are refused to prevent reuse. The app saves the readable message in local **History** so you can return to it.

The optional length-hiding setting rounds message sizes up to fixed buckets. It hides the exact length, but not the approximate size.

## Start a channel

A circle contains the people who took part in its ceremony. A **channel** lets that circle share messages with people who weren’t there.

From Circle tools, select **Start a channel**, enter a name, and choose who can read and post:

- **Announcements:** anyone with the link can read; only the owner posts.
- **Announcements with approvals:** posts need approval from the required number of circle members.
- **Group chat:** anyone with the link can read and request permission to post.
- **Members post:** anyone with the link can read; the owner chooses who can post.
- **Closed:** only approved people can read or post. There is no shared link.

Select **Start**, then use **Share the link** or **Copy the link**. Anyone who receives a shared link can forward it. A link grants reading access, not automatic posting permission.

The channel uses its circle’s relay, which must support channels. If creation fails because the relay does not support them, change the circle’s **Relay address** in conversation settings. The default relay supports channels.

## Manage channel access

Use the channel’s menu:

- **Requests** shows access requests and the names people entered. **Allow posting** permits reading and posting; **Allow reading** permits reading only; **Decline** dismisses a request. A request given reading access only stays listed so you can allow posting later.
- **Auto-approve requests covered by the link** approves requests only when the current link already covers every permission requested. Requests for extra permissions still need your decision. Automatic approval runs only while the channel is open on your device.
- **Members** lists people you have approved. **Remove member** ends their access to future messages and renews the encryption keys. If there is a shared link, it is replaced too. Share the new link with people who still need to read through a link. Other approved members keep their access.
- **Disable the link** stops future access through the current link. Approved members keep their access. Use **Share the link** later if you want to create a new one.
- **Renew encryption keys** changes the keys for future messages immediately.
- **Access rules** changes who can read and post. Some changes require approval from other circle members. This version cannot collect those approvals yet and will explain when a change is unavailable.

Requests are processed only while the channel is open on the owner’s device. Removing access cannot take back messages that someone already received or saved.

## Open a channel link

The app shows the channel’s name, owner, and access rules before you open it. If you can request posting permission, enter a name the owner will recognize. Select **Open** to read. Your name is shown to the owner with your request and beside your posts when available.

The app saves the channel locally and replaces the shared link in the address bar with a channel address that does not contain the link’s secret. Reading does not require joining the owning circle.

When posting permission is required, the app sends a request and shows a waiting message. Once the owner allows posting, the message box becomes available. Reading-only permission does not enable the message box.

If your access ends, ask the owner for a current link or permission to rejoin. A new link can reopen the same channel in this browser. A link without a secret cannot request access to a Closed channel in this version.

## Approve an announcement

For **Announcements with approvals**:

1. Write a post and select **Propose**. The app adds your approval and shows a code.
2. Send the code to other members of the owning circle. Each opens the channel on a device holding that circle, selects **Approve a post**, pastes the code, and selects **Read the code**.
3. The code shows only the first line. Verify the full post with the sender before approving it. Select **Approve**, then send the resulting approval back to the person posting.
4. Paste the approvals and select **Post** when there are enough.

Readers recognize an approved announcement only when it carries enough valid approvals. If just one approval is required, **Propose** can publish it immediately.

## Network connections

The **Network** page explains the WireGuard desktop integration. The browser cannot yet show machines or control their connections. Use the desktop command-line tool and follow the [integration guide](../crates/ADAPTERS.md). Saving a circle in the browser does not automatically configure that tool.

## Keep your browser data

Your circles and channels contain keys. Message history is stored here as readable text.

Use the same browser and browser profile to return to them. Clearing site data can remove your access and history. The relay does not have your keys and cannot restore them. A channel reader can reopen the current channel link, but that does not restore local history or earlier posting permission.

Removing a circle or channel removes it from the app on this device. Other people keep their copies. Browser storage may retain message history or traces after deletion; it does not guarantee physical erasure.

Do not copy a circle’s saved state onto a second device and use both copies. This can reuse parts of the shared secret. Create a new circle together when you need another device.

For local builds and file formats, see the [browser client README](../web/README.md).

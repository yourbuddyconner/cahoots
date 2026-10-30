# Start here

Cahoots creates a private network between people who meet in person. You exchange information between your devices, compare six words, and save a shared secret. After the meeting, that secret protects your group’s messages.

The group is called a **circle**. Each device in the circle has a numbered **seat**. A badge wears a **cahoot**, a pixel face of its own ([the faces](cahoots.md)).

Cahoots is experimental software and hardware. Start with a test circle and messages you can afford to lose.

## What happens after the meeting

Open your circle to chat, exchange a short message, or check someone on a call. Developer tools can also use circle secrets for file encryption, private networks, and signing.

Start with [Apps for your circle](apps.md): what is available now, which tools need separate setup, and where the app ecosystem is headed. Creating the circle establishes the group; the tools give you reasons to return to it.

## Try it with someone

You need two phones or laptops. No badge is required.

1. Open the [browser app](https://cahoots.network/app/) on both devices.
2. One person selects **Create a circle**. The other selects **Join a circle**.
3. Follow [Create a circle](ceremony-guide.md). The host shows one code and the other person scans it. Then the host scans the code on the other person's screen, and the rest goes through the relay. Then you both check the words. To hold it with no network, pick **Codes only**.
4. Open **Chat** and talk. Messages travel through the default relay, which only ever sees encrypted bytes. To send something without any server, use **Send a message** by QR code or paste.

The meeting that creates the circle is called a **ceremony**. It exchanges keys; it does not set up a chat server. Chat uses a relay that every member can reach. The app ships with one; a member can [run their own](relay.md) instead.

## Two ways to send messages

| In the app | How it works | What you need |
|---|---|---|
| **Send a message** | Encrypts a short message directly with unused bytes of the shared secret. Each message consumes those bytes. | A way to pass the encrypted code or text to another member. |
| **Chat** | Uses a small amount of the secret to seed encryption keys for many messages. | A relay every member can reach. The app ships with one. |

These have different security properties. Chat uses conventional encryption; it is not a one-time pad. See [Security and limits](security.md) before relying on either mode.

## What to read next

- [Create a circle](ceremony-guide.md): the meeting, screen by screen.
- [Use the browser app](browser.md): messages, chat, hello words, and local storage.
- [Run a relay](relay.md): connect your circle after the meeting.
- [Hardware](hardware.md): what is built and where to find the wiring guides.
- [Project status](status.md): tested paths, unfinished work, and the development roadmap.

For an implementation, start with the [protocol specification](protocol.md) and [chat specification](chat.md). The [repository README](../README.md) covers builds, tests, and crate layout.

# Apps for your circle

Creating a circle is the starting point. It gives your group shared secrets and a membership record that other tools can build on.

Today, Cahoots offers a few built-in tools plus separate developer integrations. An installable app ecosystem is a development direction, not a feature you can use yet.

## Open a built-in tool

These are available on a saved circle’s screen in the [browser app](https://cahoots.network/app/).

| Tool | Use it to | What to expect |
|---|---|---|
| **Chat** | Keep talking after the meeting. | Encrypted group chat through a relay. The app ships with `relay.cahoots.network`; a member can run their own instead. |
| **Send a message** / **Read a message** | Exchange short text by QR code or paste. | Uses the pad directly; consumes secret bytes for each message. No relay needed. |
| **Hello words** | Check someone during a call. | Compare fresh words derived from the pair’s key. Both devices use the same hello number. |
| **History** | Revisit messages this device has read or sent. | Local history, not a backup service. |

For most groups, the next step is to open Chat. For a quick test without any server, exchange a short message. To use your own relay, see [run a relay](relay.md). The [browser guide](browser.md) walks through each tool.

## Connect other tools

These integrations have code in the repository, but they are not buttons in the browser app. You need the command-line tools and their own provider or device setup.

| Integration | What it adds | Current state |
|---|---|---|
| **WireGuard** | Private network connections between members’ machines. | CLI and peer/key exchange exist; a two-host tunnel is exercised in CI. |
| **age** | File encryption addressed to a circle. | Plugin implementation and protocol-level tests exist. The full real-CLI acceptance run remains pending in the roadmap. |
| **HPKE** | Circle-backed encryption for developers building another application. | Library adapter with X25519 and X-Wing suites. |

See [Integrations](../crates/ADAPTERS.md) for the current commands and limitations. These tools use conventional encryption with circle-derived keys; they do not encrypt file or tunnel payloads with a one-time pad.

The browser stores circles in IndexedDB; these CLIs use a file-backed provider. A browser circle does not currently come with a guided handoff into that provider. Copying live pad state between independent tools also risks duplicate spends. A shared custodian and spend ledger are prerequisites for a safe, connected experience.

## Use the badge with desktop tools

The host and badge code also support SSH signing, git commit signing, signed contact cards, and grants. These are built and tested against a simulated badge/software chip. Real secure-element workflows still need bench verification.

These are developer workflows, not finished browser apps. See the [host README](../crates/host/README.md), [hardware guide](hardware.md), and [project status](status.md).

## Where the ecosystem is headed

The proposal is for a circle to run small shared apps: a board, a poll, or another group tool whose state comes from the circle’s messages. Everyone would agree on the app version, and the host would control its access to messages, storage, and sends.

There is no installable app catalog, third-party app runtime, or shared app installation flow today. Existing compiled WebAssembly runs Cahoots’ own protocol code; it is not a plugin system.

The [circle-app proposal](circle-apps.md) describes this direction and the missing pieces. It is a proposal for development, not an SDK contract.

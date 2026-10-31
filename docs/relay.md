# Run a relay

Chat needs a relay that every circle member can reach. Cahoots calls its relay the **Numbers Station**. The app ships pointing at the default relay, `relay.cahoots.network`, so a new circle can chat without setting anything up. This page is for running your own, which any member can do.

The relay stores and delivers encrypted messages. It has no circle keys and cannot decrypt them. Anyone can read the public feed or submit messages that fit its format; members’ devices check whether a message is authentic.

## Start a local relay

From a checkout of the Cahoots repository, with its Rust toolchain installed:

```sh
cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --db ./station.db
```

Open `http://127.0.0.1:8080/` to see the relay’s feed page. The log is stored in the SQLite database `station.db` (with `station.db-wal` beside it while the relay runs); keep those files if you want the relay to retain its history after a restart.

This runs the plain relay, which carries circles and chat. To carry [channels](browser.md#channels) too, build it with three features:

```sh
cargo run -p cahoots-station --features objects,channels,gate -- --listen 127.0.0.1:8080 --db ./station.db
```

This command accepts connections only from the same computer. On a phone, `127.0.0.1` means the phone itself, not your laptop.

## Make it reachable by the group

For chat between devices, serve the relay at an address all members can reach. Use HTTPS when connecting from the HTTPS-hosted browser app. A plain HTTP relay can be blocked by the browser as mixed content.

A typical deployment puts a reverse proxy with HTTPS in front of the relay. The proxy forwards requests to the local listening address above. Keep the log on persistent storage and add request limits at the proxy: the station does not provide rate limiting itself.

The [station README](../crates/station/README.md) has Docker commands and the full API. [deploy/relay](../deploy/relay/README.md) is the recipe the default relay uses, with the station’s own rate limit and size cap; it works for a relay of your own too.

## Choose what the relay runs

The relay is built with Cargo features, so an operator compiles in exactly what they want to run. With none chosen, it is the plain relay: circles, chat, the hash chain, the limits and the feed page.

| Feature | What it adds |
|---|---|
| `feed-page` | The live feed page at `/`. On by default. |
| `objects` | Stores the signed parts of a channel: the owner's manifest, the channel object, its policy, grants, revocations, key deposits, join requests and link tokens. |
| `channels` | Stores and serves channel messages, and limits each channel to `--channel-posts-per-minute` posts (default 300). |
| `gate` | Checks who may read a channel (a link's token or a member's grant) and who may post (a grant, its budget, and any approvals the policy asks for). |
| `issuer` | Declared, not built. It will let a relay in an enclave admit members while the owner's phone is closed. |
| `anchor` | Declared, not built. It will anchor the log's head in an enclave's log. |

`GET /v1/info` answers with the features a relay was built with and its limits. The app reads it before it starts a channel, and refuses a relay without `channels` and `gate`.

None of these features gives the relay a channel key, a link's secret or anything that decrypts. With `gate`, it checks grants and link tokens, which are signed public data or derived so that the secret cannot be recovered from them.

## Connect the app

1. Open a saved circle and select **Chat**.
2. From the menu, open **Relay address** and enter the relay’s base URL, such as `https://relay.example.org`.
3. Select **Save and connect**.
4. Give every other member the same URL. Members on different relays do not see each other’s messages.

Relay settings are saved separately for each circle. **Use the default relay** in the same sheet switches back.

## The default relay

`relay.cahoots.network` runs the same station binary from the recipe in [deploy/relay](../deploy/relay/README.md), built with `objects`, `channels` and `gate`. It keeps everything it accepts, limits each client to 120 posts and 600 reads a minute, and refuses posts once its log reaches its size cap. It is an ordinary host, not the enclave deployment in the roadmap: the operator can read the ciphertext and the traffic shape, like any relay, and nothing more.

## What the relay can see and do

The relay sees public circle IDs, encrypted message sizes, timing, and network connections. It can delay or refuse delivery. For a channel it also sees which member key posted each message, which link tokens read, and the names and keys in the join queue; see [Security and limits](security.md#a-channel-is-ordinary-encryption).

Its log is hash-chained. A reader with a previously saved head hash can detect some changes to history, but a hash chain alone does not stop an operator from giving different readers different histories. The planned enclave deployment and external anchoring are not deployed.

Running the relay does not make you a circle member. Conversely, losing the relay does not erase secrets stored on members’ devices.

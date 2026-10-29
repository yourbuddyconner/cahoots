# Run a relay

Chat needs a relay that every circle member can reach. Cahoots calls its relay the **Numbers Station**. There is no default public relay in the app.

The relay stores and delivers encrypted messages. It has no circle keys and cannot decrypt them. Anyone can read the public feed or submit messages that fit its format; members’ devices check whether a message is authentic.

## Start a local relay

From a checkout of the Cahoots repository, with its Rust toolchain installed:

```sh
cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --log ./station.jsonl
```

Open `http://127.0.0.1:8080/` to see the relay’s feed page. The log is stored in `station.jsonl`; keep that file if you want the relay to retain its history after a restart.

This command accepts connections only from the same computer. On a phone, `127.0.0.1` means the phone itself, not your laptop.

## Make it reachable by the group

For chat between devices, serve the relay at an address all members can reach. Use HTTPS when connecting from the HTTPS-hosted browser app. A plain HTTP relay can be blocked by the browser as mixed content.

A typical deployment puts a reverse proxy with HTTPS in front of the relay. The proxy forwards requests to the local listening address above. Keep the log on persistent storage and add request limits at the proxy: the station does not provide rate limiting itself.

The [station README](../crates/station/README.md) has Docker commands and the full API. The repository does not provide a complete production hosting guide.

## Connect the app

1. Open a saved circle and select **Chat**.
2. Open **Relay address** and enter the relay’s base URL, such as `https://relay.example.org`.
3. Select **Save and connect**.
4. Give every other member the same URL.

Relay settings are saved separately for each circle.

## What the relay can see and do

The relay sees public circle IDs, encrypted message sizes, timing, and network connections. It can delay or refuse delivery.

Its log is hash-chained. A reader with a previously saved head hash can detect some changes to history, but a hash chain alone does not stop an operator from giving different readers different histories. The planned enclave deployment and external anchoring are not deployed.

Running the relay does not make you a circle member. Conversely, losing the relay does not erase secrets stored on members’ devices.

# cahoots-chat

Pad-seeded end-to-end encrypted group chat, and the demo client that runs it over the Numbers Station relay. The design is in `docs/chat.md`.

## The security claim

One 32-byte pad spend keys an **epoch**. Inside an epoch every sender has its own symmetric ratchet, so each message gets a fresh key and used keys are deleted. Messages are ChaCha20-Poly1305 with the header as associated data.

- Confidentiality and integrity are **computational, hedged by the pad**. There is no public-key step an attacker could break instead. The UI says "end-to-end encrypted, pad-hedged", never "unbreakable". Pure-pad text (`docs/protocol.md`) remains the mode for lines that deserve the strongest claim.
- Deleting chain keys gives forward secrecy inside an epoch. The next epoch spend gives post-compromise security, because the attacker never had the pad.
- One epoch costs the circle 32 pad bytes, however many messages it carries.

## Limits

- **`sender` is a chain label, not a person.** Every member can derive every sender's chain, exactly as every member holds every slice. The demo puts the typed name inside the plaintext as a courtesy, not as proof.
- **The relay sees traffic shape.** It sees which circle posted, which epoch pointer and counter, when, and the bucketed size. It cannot read or forge an entry and cannot refuse chat messages for reuse, because an epoch pointer legitimately appears in many messages. Epoch reuse is refused by each member's own ledger.
- A tampered message that is the **first** sighting of a new epoch still burns that epoch's 32-byte range on the reader, before the tag check. The untampered copy then decrypts.
- The local ceremony is a development shortcut with OS randomness, so its circles carry the `CSPRNG` label.

## Demo: three terminals and a relay

Build once:

```sh
cargo build -p cahoots-station -p cahoots-chat --features cahoots-chat/cli
```

Terminal 0, the relay:

```sh
./target/debug/cahoots-station --listen 127.0.0.1:8080 --log ./station.jsonl
```

Terminal 1, the ceremony and the first member:

```sh
./target/debug/cahoots-chat local-ceremony --members 3 --out ./circles --name demo
./target/debug/cahoots-chat run --circle ./circles/member-0 --name alice
```

Terminals 2 and 3:

```sh
./target/debug/cahoots-chat run --circle ./circles/member-1 --name bob
./target/debug/cahoots-chat run --circle ./circles/member-2 --name carol
```

Type a line in any terminal and it appears in the others as `[#<sender>] name: text`. Press ctrl-d to quit; the session state and last seen sequence number are saved after every send and receive, so restarting the same member resumes without replaying.

Show a rotation by starting a member with `--rotate-after 2`: its third message opens a new epoch, and the others follow it on receipt. Show a tag failure from a fourth terminal:

```sh
./target/debug/cahoots-chat tamper-test --circle ./circles/member-2
```

Every reader prints `[rejected seq N: Tag]`, and the next real message still decrypts. Open `http://127.0.0.1:8080/` to see the relay's view: ciphertext, circle ids, counts, and nothing else.

## Files the client keeps

The member directory is the `cahoots-provider` `FileProvider` format (`key`, `<circle_id>/{params, slice, pad.enc, ledger}`), written by `local-ceremony` through the provider crate. The chat client reads it through its own adapters in `src/cli/store.rs`, because `FileProvider` does not hand out its `Pad`. It adds `label`, `chat.state` (exported session state, encrypted under the same key), and `chat.seq`.

## Library

Without the `cli` feature the crate is `no_std` + `alloc` and builds for wasm32 and thumbv7em. The API is `ChatSession` over any `cahoots_core::pad::Pad`, plus the primitives (`epoch_key`, `chain_init`, `step`, `seal`, `open`, bucket padding). Vectors are in `crates/vectors/chat.json`.

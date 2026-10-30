# cahoots-chat

This crate is end-to-end encrypted group chat keyed by the circle root, plus the demo client that runs it over the Numbers Station relay (the public feed). The design is in `docs/chat.md`.

## The security claim

Chat runs in numbered **epochs**. The key of epoch `n` is `HMAC-SHA256(root, "chat" || circle_id || n)`, the root's chat key, so an epoch costs no pad. Inside an epoch every sender has its own symmetric ratchet, so each message gets a fresh key and used keys are deleted. Messages are ChaCha20-Poly1305 with the header as associated data.

- Confidentiality and integrity are **computational**. Chat is as secret as the root. The UI never says "unbreakable". Pure-pad text (`docs/protocol.md`) remains the mode for lines that deserve the strongest claim.
- Every member's device holds the root. Whoever compromises one member's device can read every message the relay still holds, past and future. Deleting chain keys bounds what one message key reveals and nothing more. A new epoch does not help.
- A device writes its chat state before it posts, and reads the relay to the end, its own messages included, before its first send after a restart. Otherwise it could reuse a key and nonce. The session refuses to send with `NotSynced` until then.
- Pad epochs, where 32 pad bytes keyed each epoch, are retired as of 2026-10-30. A message with a pad epoch id is refused.

## Limits

- **`sender` is a chain label, not a person.** Every member can derive every sender's chain, just as every member holds every slice. The demo puts the typed name inside the plaintext as a courtesy; it proves nothing.
- **The relay sees traffic shape.** It sees which circle posted, which epoch number and counter, when, and the bucketed size. It cannot read or forge an entry. Each member's chains refuse replays.
- A receiver keeps four epochs. An epoch older than one it dropped is refused.
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

Every reader prints `[rejected seq N: Tag]`, and the next real message still decrypts. Open `http://127.0.0.1:8080/` to see the relay's view, which is only ciphertext, circle ids, and counts.

## Files the client keeps

The member directory is the `cahoots-provider` `FileProvider` format (`key`, `<circle_id>/{params, slice, pad.enc, ledger}`), written by `local-ceremony` through the provider crate. The chat client reads it through its own adapters in `src/cli/store.rs`, because `FileProvider` does not hand out its `Pad`. It adds `label`, `root` (the circle root from the ceremony, encrypted under the same key, mode 0600), `chat.state` (exported session state, encrypted under the same key), and `chat.seq`. A circle directory without `root` was made before root epochs, and `run` refuses it.

## Library

Without the `cli` feature the crate is `no_std` + `alloc` and builds for wasm32 and thumbv7em. The API is `ChatSession::new(root, circle_id, members, seat)`, plus the primitives (`epoch_key`, `chain_init`, `step`, `seal`, `open`, bucket padding). Vectors are in `crates/vectors/chat.json`.

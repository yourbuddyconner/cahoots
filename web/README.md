# Cahoots phone and laptop client (M2, C4)

A PWA. `cahoots-core` and `cahoots-chat` run as WebAssembly. There are no CDN scripts and no bundler.

A ceremony runs one of two ways. **Through the relay** is the default: the host shows one code, each person joining scans it and picks a seat, the host scans each joiner's code, and the rounds go through a Numbers Station relay sealed with a key that never leaves that first code. **Codes only** is the fallback with no network: members exchange every round by pointing phones at each other's screens, or by paste and file when a camera is not available. Either way the six words are read aloud and compared at the end. Chat goes through the relay too. Pure-pad messages need no network. The relay only ever sends and receives ciphertext.

## Screens

One job per screen: a top bar (back, title, a menu for secondary options), one stage element, and one primary action pinned at the bottom. On phones the ceremony's commit and reveal steps have a Show / Scan toggle; on screens wider than 960 px both panels sit side by side. Secondary paths (paste, copy, download, load a file, leave) live in the menu sheet so the main screen never needs scrolling.

In a codes-only ceremony a commitment code carries the circle parameters, so a joiner scans any member who has already committed. A member that has not yet revealed accepts a late reveal as both commitment and reveal (`Ceremony.add_reveal`), which lets a member who joined late catch up without the others going back a step. The check-words screen keeps your own reveal available from its menu for anyone still collecting.

## Layout

| Path | What |
|---|---|
| `crates/wasm/` | `cahoots-wasm`: wasm-bindgen API over the core. Ceremony, `Circle` (pad + ledger + label + circle root + chat session + relay cursor) with pure-pad send/receive, chat send/receive, epoch rotation, and export/import for IndexedDB; RaptorQ fountain coding for animated QR; QR rendering to SVG; bucket padding; base64url. Also compiles natively, so `cargo test -p cahoots-wasm` exercises the whole API. |
| `app/` | The static PWA: `index.html`, `app.js` (ES module), `styles.css`, `sw.js` (network-first service worker with a cache fallback; bump `VERSION` on every release), `manifest.json`, icons. |
| `e2e.mjs` | End-to-end check: isolated browser contexts run a codes-only ceremony through the paste fallbacks and a ceremony through the relay, chat through a Numbers Station the script starts, and exchange a pure-pad message. `cargo build -p cahoots-station && node web/e2e.mjs`. |
| `app/pkg/` | Build output of `wasm-pack`. Not committed. |

## Build and run

```sh
cargo install wasm-pack --locked   # once

cd crates/wasm
wasm-pack build --target web --release --out-dir ../../web/app/pkg --no-typescript

cd ../../web/app
python3 -m http.server 8765 --bind 127.0.0.1
# open http://127.0.0.1:8765/
```

The service worker fetches from the network first and falls back to its cache, so a load with a connection always gets the current app and the app still opens with no connection. For installs on a phone, serve over HTTPS (any static host) and use "Add to Home Screen". The deployed app lives at https://cahoots.network/app/. The wasm binary is about 770 KB; the QR decoder and fountain coder are built at `opt-level = 3` for speed.

## How a ceremony flows

1. The host opens **Create a circle** and picks a name, the number of people, how to hold the ceremony (**Through the relay**, the default, or **Codes only**), and a secret size (Small 16 KiB, Medium 64 KiB, Large 256 KiB). The hosted software member option is under **Advanced**. When it is on, every member's seat screen says so.
2. Randomness is this device's `crypto.getRandomValues`. **Add hardware randomness** mixes in extra random bytes loaded from a file or an animated QR. The label is `TRNG` only when a full-length hardware contribution was mixed in.

### Through the relay (the default)

1. The host's screen shows one code: "Show this code to everyone joining". It carries the circle's parameters, the relay's address and a channel key. The key never leaves the code, and the relay never sees it.
2. Everyone else opens **Join a circle**, scans the host's code, and picks a free seat ("Which seat are you?"). Seat 1 is the host's.
3. Each joiner's screen then shows a code of its own: "Show this to the host". The host scans each one with **Scan a joiner's code**. Only a commitment the host has scanned goes into the host's ceremony, so someone who saw the host's code from across the room cannot slip in.
4. When every seat is scanned the host sees "Everyone is scanned" and chooses **Continue**. The commitments, reveals and signatures go through the relay, sealed under the channel key, with no more scanning.
5. **Step 4, Compare words.** Read all six words aloud and compare every device. **They match** saves the circle, and the circle chats through the relay the ceremony used. A mismatch means everyone discards the ceremony and starts again.

A joiner whose seat was taken on the relay by another device first is told so and posts nothing more. A joiner that sees a seat-1 setup on the relay that is not the host it scanned says so and ignores it.

### Codes only (no network)

1. The host keeps seat 1 and taps **Make my contribution**. Everyone else opens **Join a circle**, scans any member's code (it carries the parameters), and picks a free seat. Seats already seen are crossed out.
2. **Step 1, Meet the group.** Exchange commitment codes. The screen names the missing seats and counts received codes without restarting the scanner between scans.
3. **Step 2, Exchange secrets.** Exchange encrypted reveals. The local cryptographic state can advance before peers have scanned this device’s reveal, so the UI deliberately keeps showing it. Once every reveal is received, the device switches to Show mode. The user must confirm **They have scanned my reveal** or **Everyone has scanned my reveal** before the UI advances. This confirmation is a human handoff check, not a cryptographic receipt.
4. **Step 3, Confirm members.** Exchange signatures over the manifest. Signatures received early are buffered until the reveals allow verification. Once all signatures are in, keep showing this device’s code until the others confirm receipt.
5. **Step 4, Compare words.** As above.

A device that receives another member’s reveal while still collecting commitments can use that reveal as both. This must not make its own outgoing reveal disappear. The end-to-end regression covers that two-person case, an early signature arriving before its reveal, and a three-person ceremony with one reveal still missing.

Messages are base64url text shown as a QR and as copyable text. Reads take a scan or a paste. Reuse and tag failures show distinct refusals.

## Hello words

**Hello words** on a circle's screen is for a phone or video call. Pick the person (the only other member is picked for you in a two-person cahoot). The screen shows two words you say and two words they say back, with a hello number. Both devices show the same four words for the same number. Tap **Next hello** on both devices to move on, which you do whenever a call feels wrong: the person who cannot say the new words is not the person from the ceremony. The menu lets you name the person on this device, which the chat then uses instead of a seat number, and set the hello number by hand if the two of you drift apart.

The words come from the pair keys made during the sealed reveal step, so a circle made before that step has none and the screen says so. The hello number is saved with the cahoot. Nothing about hello words touches the pad or the network.

## Chat (root epochs, over a relay)

**Chat** on a circle's screen opens the chat for that circle. The header reads "End-to-end encrypted": the circle root from the ceremony seeds the keys and ChaCha20-Poly1305 encrypts, per `docs/chat.md`. The relay sees ciphertext and traffic shape only. A circle saved before root epochs has no root. Its chat screen says "This circle was made before root epochs. Run a new ceremony to chat." in place of the message box, and its pure-pad messages still work.

- **Relay.** The app ships pointing at `https://relay.cahoots.network` (`DEFAULT_RELAY` in `app.js`). **Relay address** in the menu sets another one per circle, stored in IndexedDB, and **Use the default relay** switches back; a circle saved before the default existed gets it on load. For a local relay: `cargo run -p cahoots-station -- --listen 127.0.0.1:8080 --log station.jsonl`. The screen long-polls `GET /v1/circles/{id}/messages?after=<seq>&wait=25000` while open and posts sealed messages as raw bytes.
- **Epochs.** Epochs are numbered from 1, and each one's keys come from the circle root. The first send starts epoch 1. **Start a new epoch** in the menu moves to the next number. Neither spends pad. Peers switch to the newest epoch they see, and the menu shows the current one. Rotation also happens on its own after 1000 of your own messages in an epoch.
- **Padding.** Plaintext is padded to 64, 256, 1024, or 4096 bytes before encryption.
- **Rejections** show inline and muted as `rejected · <reason> · seq N`, where the reason is the friendly text for the error (see `friendly()` in `app.js`). Your own messages coming back from the relay go through `chat_receive` too. It does not show them again: it moves this device's sending position past them, so a device restored from older state never reuses a key.
- **Persistence.** Chat keys, the epoch state, and the last relay sequence number are exported with the circle, so a reload resumes where it left off without re-reading the feed. Plaintext history is stored on the device.
- **CORS.** The station allows any origin, so the app on cahoots.network can talk to a relay on any host.
- The service worker never caches `/v1/` responses.

## Exchange formats (also used by the paste and file fallbacks)

| Data | Form |
|---|---|
| Parameters | `cahoots-params:<base64url params bytes>` |
| Commitment | `cahoots-commit:<index>:<base64url 32-byte hash + 1216-byte X-Wing public key>:<base64url params bytes>`. The parameters ride along so a joiner can scan any member. |
| Reveal (text/file) | `cahoots-reveal:<index>:<base64url sealed reveal>`, the blob from `docs/protocol.md` section 3.2, encrypted to every other member |
| Commitment (animated QR) | Fountain packets of `0x01 ‖ index ‖ params_len ‖ params ‖ hash ‖ public key`. Each QR frame is base64url of `"CHQ1" ‖ RaptorQ OTI (12 bytes) ‖ encoding packet`. |
| Reveal (animated QR) | Fountain packets of `0x02 ‖ sealed reveal` |
| Message | base64url of the wire message from the protocol spec |

## What was verified

2026-10-29, `web/e2e.mjs` in headless Chrome, a phone-sized context and a desktop one: a two-member ceremony through the paste fallbacks, including the late-joiner path where the host takes the joiner's reveal as commitment and reveal; identical check words; hello words; a chat through a station the script starts, with the relay address set from the menu and a message each way; a one-time-pad message decrypted on the other member; the same message refused as reuse on a second read; no console errors. The script runs in about ten seconds and is the check to run before a release.

The suite now also holds a ceremony through the relay: the host's code is pasted into the joiner, the joiner picks a seat, the host takes the joiner's code, and the rounds go through the station the script starts, with the same words on both sides.

2026-10-20, by hand

Driven in Chrome with two tabs on two origins (`127.0.0.1` and `localhost`, so each tab had its own IndexedDB like two phones would), using the paste fallback:

- Two members completed a ceremony with 16 KiB pads. Both screens showed the same six check words.
- A message from member 0 decrypted on member 1 and a reply decrypted on member 0.
- A message with one flipped byte was refused with "Integrity check failed", and the untouched original then decrypted, showing nothing had been spent.
- Replaying a message was refused with "Possible pad reuse".
- The circle survived a page reload: the ledger was restored and the next send used offset 64.
- The fountain path ran in the browser: a 16 KiB transfer decoded with one in four packets dropped, in about 50 ms.

Chat, verified the same day with two tabs on two origins behind a proxy that forwarded `/v1/` to one `cahoots-station` process:

- After a 2-member ceremony, member 0 opened chat, sent a message, and it appeared on member 1 under "member 0 · epoch 0:0". Member 1 replied in the same epoch, and member 0 received it.
- Member 0 pressed **Rotate epoch** (the button is now named **Start a new epoch**; spent 32 bytes, chip changed to `0:32`) and sent again. Member 1 decrypted it and its own chip switched to `0:32`.
- A copy of that message with one ciphertext byte flipped was posted straight to the relay as a new entry. Member 1 showed `rejected: replay`, because the copy reused an already-consumed counter, which is checked before the tag. The tag-failure path is covered by the Rust tests.
- Member 1 reloaded the page. It came back on epoch `0:32` with the full history, "Last seq 3", no re-decrypted or duplicated entries, and received the next message from member 0.
- No console errors in either tab.

Camera scanning opens the camera with `getUserMedia`, draws each frame to a canvas at up to 720 px, and decodes it in our own wasm with the `rqrr` crate (`qr_decode_rgba`). No browser barcode API is used, so it works on iOS Safari and Chrome alike. The decoder is covered by Rust tests on rendered frames, including a 600-byte fountain frame. A real camera was not available in the automation session, so the end-to-end scan still needs a check on a phone.

## Limits

- Erasure on phone storage is best effort. IndexedDB may keep old pages on disk.
- Plaintext history is stored on the device so messages can be reread. Deleting the circle removes it.
- A message is at most 255 characters. With length hiding off (in the menu) longer messages are possible, but the compose box keeps the limit.
- `bucket_pad` plaintext length is hidden only within the chosen bucket.


### Ceremony UI regression checks

Run `node web/e2e.mjs` for the full browser suite, or `node web/e2e.mjs --ceremony-only` for the three-member ceremony and compact-layout checks. Each run uses a fresh relay log. The suite checks both directions of the reveal handoff, buffered early signatures, matching check words, and complete QR/camera visibility on a 375 × 667 viewport. It also rasterizes a displayed reveal frame and decodes it with the real WASM QR decoder. These checks do not replace a physical phone-camera test.

# Cahoots phone and laptop client (M2)

An offline PWA. `cahoots-core` runs as WebAssembly. There are no network calls, no CDN scripts, and no bundler. Members exchange ceremony data by pointing phones at each other's screens, or by paste and file when a camera is not available.

## Layout

| Path | What |
|---|---|
| `wasm/` | `cahoots-wasm`: wasm-bindgen API over the core. Ceremony, `Circle` (pad + ledger + label) with send/receive and export/import for IndexedDB, RaptorQ fountain coding for animated QR, QR rendering to SVG, bucket padding, base64url. Also compiles natively, so `cargo test -p cahoots-wasm` exercises the whole API. |
| `app/` | The static PWA: `index.html`, `app.js` (ES module), `styles.css`, `sw.js` (cache-first service worker), `manifest.json`. |
| `app/pkg/` | Build output of `wasm-pack`. Not committed. |

## Build and run

```sh
# once: wasm-pack. The repo pins Rust 1.88, which is too old to build wasm-pack itself,
# so install it with a newer toolchain. Do this outside the repo directory.
cargo +stable install wasm-pack --locked

cd crates/wasm
wasm-pack build --target web --release --out-dir ../../web/app/pkg --no-typescript

cd ../../web/app
python3 -m http.server 8765 --bind 127.0.0.1
# open http://127.0.0.1:8765/
```

The service worker caches every asset on first load. After that the app works with the network off. For installs on a phone, serve over HTTPS (any static host) and use "Add to Home Screen". `wasm-opt` is disabled in `crates/wasm/Cargo.toml` so the build needs no binaryen download; enable it later for a smaller binary (currently about 435 KB).

## How a ceremony flows

1. The leader opens **Start a ceremony**, picks name, member count, pad length, and the agent seat. If the agent seat is on, the screen says the circle trusts the enclave with its pad.
2. Every member confirms the parameters, picks their member number, and picks a randomness source: this device's `crypto.getRandomValues`, or that XORed with a `cahoots-yk` contribution (file import or animated-QR import from a laptop). The label is `TRNG` only when a full-length hardware contribution was imported.
3. **Commit.** Each member shows a single QR of their commitment. Others scan it. Nobody can reveal until all commitments are in; the app withholds the reveal until then.
4. **Reveal.** Each member shows an animated QR (RaptorQ fountain packets at about 10 frames per second). Receivers scan until the progress bar fills. Every reveal is verified against its commitment; a mismatch aborts with nothing kept.
5. **Check words.** Six BIP-39 words. If every screen agrees, members tap **They match** and the circle is saved.

Messages are base64url text shown as a QR and as copyable text. Reads take a scan or a paste. Reuse and tag failures show distinct refusals.

## Exchange formats (also used by the paste and file fallbacks)

| Data | Form |
|---|---|
| Parameters | `cahoots-params:<base64url params bytes>` |
| Commitment | `cahoots-commit:<index>:<base64url 32 bytes>` |
| Reveal (text/file) | `cahoots-reveal:<index>:<base64url contribution>` |
| Reveal (animated QR) | Fountain packets of `[index] ‖ contribution`. Each QR frame is base64url of `"CHQ1" ‖ RaptorQ OTI (12 bytes) ‖ encoding packet`. |
| Message | base64url of the wire message from the protocol spec |

## What was verified (2026-10-20)

Driven in Chrome with two tabs on two origins (`127.0.0.1` and `localhost`, so each tab had its own IndexedDB like two phones would), using the paste fallback:

- Two members completed a ceremony with 16 KiB pads. Both screens showed the same six check words.
- A message from member 0 decrypted on member 1 and a reply decrypted on member 0.
- A message with one flipped byte was refused with "Integrity check failed", and the untouched original then decrypted, showing nothing had been spent.
- Replaying a message was refused with "Possible pad reuse".
- The circle survived a page reload: the ledger was restored and the next send used offset 64.
- The fountain path ran in the browser: a 16 KiB transfer decoded with one in four packets dropped, in about 50 ms.

Camera scanning uses the `BarcodeDetector` API and was not tested here; there is no camera in the automation session. Chrome on Android and desktop Chrome support it. Safari does not, so iPhone users use paste or file import until a vendored JS decoder is added.

## Limits

- Erasure on phone storage is best effort. IndexedDB may keep old pages on disk.
- Plaintext history is stored on the device so messages can be reread. Deleting the circle removes it.
- Message compose is limited to 255 bytes with padding on, and 65535 bytes with it off.
- No animated-QR *sending* of a `cahoots-yk` contribution exists yet on the laptop side; the import screen is ready for it.
- `bucket_pad` plaintext length is hidden only within the chosen bucket.

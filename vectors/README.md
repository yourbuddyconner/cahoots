# Test vectors

Generated from `cahoots-core`. Regenerate with `cargo run -p cahoots-vectors`. CI runs `cargo run -p cahoots-vectors -- --check` and fails if these files are stale.

| File | Covers |
|---|---|
| `ceremony.json` | Params encoding, commitments, combined pad, check words, circle ID, a reveal that must abort |
| `checkwords.json` | Check words and circle IDs for several pads |
| `messages.json` | Seal and open for every message type, plus a tag failure |
| `spend.json` | Two sends, sender erasure, receiver replay rejection, ledger contents |
| `frames.json` | Touch-link framing with CRC-32 |

All byte strings are lowercase hex.

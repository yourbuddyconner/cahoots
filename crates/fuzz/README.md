# Fuzz targets

`cargo-fuzz` targets for the parsers in `cahoots-core`. The project's working rules require the message decoder to be fuzzed; property tests in `crates/core/tests/proptests.rs` cover the same surface in CI, and these targets run longer campaigns locally.

```sh
cargo install cargo-fuzz
cd crates/fuzz
cargo +nightly fuzz run message_parse -- -max_total_time=60
cargo +nightly fuzz run frame_decode -- -max_total_time=60
cargo +nightly fuzz run params_decode -- -max_total_time=60
cargo +nightly fuzz run ceremony_feed -- -max_total_time=60
```

| Target | What it checks |
|---|---|
| `message_parse` | `parse` then `open` on arbitrary wire and pad bytes never panic |
| `frame_decode` | `decode` never panics; a decoded frame re-encodes to the same bytes |
| `params_decode` | `CircleParams::decode` never panics; decoded params re-encode identically |
| `ceremony_feed` | Any sequence of `add_commitment` / `add_reveal` / `finish` calls never panics |

This directory is excluded from the workspace because it needs nightly and `libfuzzer-sys`. Corpora and artifacts are git-ignored.

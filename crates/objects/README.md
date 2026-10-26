# cahoots-objects

The four objects a seat signs: a grant, an approval, a fact, and a receipt. Each is a canonical CBOR map in a COSE_Sign1 with ES256, signed by a seat key made at the ceremony. The formats are in [protocol](../../docs/protocol.md#8-signed-objects). `no_std` with `alloc`. The `std` feature adds a JSON projection, which is informative and never verified.

| Object | Domain string | What it says |
|---|---|---|
| `grant::Grant` | `cahoots/grant/v1` | A seat lets a key spend on some purposes, and the key may delegate a narrower grant. |
| `approval::Approval` | `cahoots/approval/v1` | A seat's verdict on a request, with the lines the person saw and an assurance level. |
| `fact::Fact` | `cahoots/fact/v1` | A reading a seat took from a module, tied to a log position by its nonce. |
| `receipt::Receipt` | `cahoots/receipt/v1` | A record that a seat spent a range of the pad. |

Each type has `encode`, `decode` (which refuses anything non-canonical), `sign(key, kid)`, and `verify`.

## Verify one against a manifest

The circle's signed manifest is the root. Verify it first, then the object:

```rust
let manifest = sealed_manifest.verify()?;           // cahoots_core::manifest::SealedManifest
let approval = Approval::verify(&signed, &manifest)?; // checks the seat's key and the circle
```

For a grant that was delegated, `grant::verify_chain(&[root, link], &manifest)` checks the chain root first and returns the leaf.

## Vectors

`crates/vectors/objects.json`, with the fields described in [`crates/vectors/README.md`](../vectors/README.md). `tests/vectors.rs` rebuilds every object from the inputs. Regenerate with `cargo run -p cahoots-vectors`.

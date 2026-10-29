# Attestation roots

`nxp-se05x-attestation-root.pem` is a placeholder. It is the software chip's test certificate (`cahoots_chip::testca::ROOT_PEM`), whose private key is public in the source. `NXP_ROOTS` in `src/attestation.rs` includes this file, so until it is replaced, `NXP_ROOTS` verifies only the software chip. It does not verify a real SE051.

Placeholder fingerprints:

- SHA-256 of the PEM file: `ae9f6ab663f743d2708180ecd61888837af8a4f98565d5d0d085a7a0d4772bfd`
- SHA-256 of the certificate (DER): `57077975609eda4c4d9ef223e52a54bf367ccfffca6f1651c8e1fc5dd60f6aea`

## Replacing it with NXP's root

1. Read the attestation certificate chain from the first real SE051 bundle (bench, Task 10 of the chip plan).
2. Take the top certificate of the chain, the one that issued the rest. Check its fingerprint against NXP's published value for the SE05x attestation root before trusting it.
3. Save it as PEM over `nxp-se05x-attestation-root.pem`, and write its SHA-256 fingerprints here in place of the placeholder's.
4. Confirm the layout of the bytes the chip signs and change `attested_payload` in `src/attestation.rs` to match. The current layout is the software chip's.
5. Run `cargo test -p cahoots-objects`. The software-chip tests pass their own root explicitly, so they keep working.

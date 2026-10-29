# Attestation roots

`NXP_ROOTS` in `src/attestation.rs` is empty. NXP's SE05x attestation root is not in the repository yet, so nothing verifies against `NXP_ROOTS`, and `cahoots-host attest` on a real SE051 says that its attestation cannot pass until the root is added.

`software-chip-test-root.pem` is the software chip's test certificate (`cahoots_chip::testca::ROOT_PEM`). Its private key is in the source, so any program can make a bundle that passes against it. It is `TEST_ROOTS`, never part of `NXP_ROOTS`. Tests pass it explicitly, and `cahoots-host attest` accepts it only with `--sim` or `--test-root`, and then reports a software chip, not hardware.

Test root fingerprints:

- SHA-256 of the PEM file: `ae9f6ab663f743d2708180ecd61888837af8a4f98565d5d0d085a7a0d4772bfd`
- SHA-256 of the certificate (DER): `57077975609eda4c4d9ef223e52a54bf367ccfffca6f1651c8e1fc5dd60f6aea`

## What the check does not do

- It does not check validity dates (`notBefore`, `notAfter`).
- It does not check `basicConstraints` or `keyUsage` on the root or on the attestation certificate.
- The chain is one level: the attestation certificate must be a root in the list or be signed by one. NXP's chain probably has an intermediate certificate between the root and the chip's certificate. What the real chain looks like is bench work, and the check may need to walk more than one level.

## Adding NXP's root

1. Read the attestation certificate chain from the first real SE051 bundle (bench, Task 10 of the chip plan).
2. Take the top certificate of the chain, the one that issued the rest. Check its fingerprint against NXP's published value for the SE05x attestation root before trusting it.
3. Save it as PEM in this folder (for example `nxp-se05x-attestation-root.pem`), add it to `NXP_ROOTS`, and write its SHA-256 fingerprints here.
4. If the chain has an intermediate certificate, make `verify` walk it, and decide whether to check validity dates, `basicConstraints` and `keyUsage` at the same time.
5. Confirm the layout of the bytes the chip signs and change `attested_payload` in `src/attestation.rs` to match. The current layout is the software chip's.
6. Run `cargo test -p cahoots-objects`. The software-chip tests pass their own root explicitly, so they keep working. Change `test_roots_are_the_software_chips_ca_and_nxp_roots_are_empty`, which checks that `NXP_ROOTS` is empty.

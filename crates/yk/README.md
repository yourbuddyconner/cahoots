# cahoots-yk

Reads random bytes from a YubiKey and delivers them to a Cahoots client as a second entropy source. It is the first extra hardware source because YubiKeys are common and need no custom hardware.

## What it does

1. Connects to the first PC/SC reader that holds a card.
2. Selects the OpenPGP card application (AID `D2 76 00 01 24 01`).
3. Reads the extended-capabilities data object (`C0` inside `6E`) to learn the maximum `GET CHALLENGE` length.
4. Sends `GET CHALLENGE` (`00 84 00 00 Le`) in chunks of that size until the request is filled.

This is the same path `gpg-connect-agent "SCD RANDOM n" /bye` uses. No PIN is needed.

## Fail-closed rules

The read aborts, and the output buffer is zeroed, when any of these happen:

- the card is removed or stops answering;
- a chunk is shorter than requested;
- the card returns any status word other than `9000`;
- a 32-byte block is all zeros;
- a 32-byte block repeats an earlier block in the same read;
- the Repetition Count Test fires.

### Repetition Count Test

YubiKey output is conditioned inside the chip, so the raw-noise tests of NIST SP 800-90B do not apply. We run the Repetition Count Test only, to catch a stuck or failed device. Each byte is one sample at the nominal rate of 8 bits of entropy. With `alpha = 2^-30` the cutoff is `C = 1 + ceil(30 / 8) = 5`: five identical consecutive bytes abort the read. Over a 256 KiB read the chance of a false alarm is about `2^-14`.

## Why a YubiKey is never the only source

The YubiKey's firmware and chip are closed. In 2019, Yubico's advisory YSA-2019-02 reported that YubiKey FIPS series 4 devices returned random values with reduced randomness after power-up. A device with that kind of fault would pass every check above. Cahoots therefore always XORs YubiKey bytes with the device TRNG or the platform CSPRNG. A YubiKey adds entropy to that source and never replaces it.

## CLI

```sh
cahoots-yk info                                    # readers and the card's max challenge length
cahoots-yk random --bytes 262144 --out contribution.bin
cahoots-yk random --bytes 262144 > contribution.bin  # refuses to write to a terminal
cahoots-yk bench --bytes 262144                    # throughput and a byte-histogram chi-square
```

`bench` prints a chi-square of the byte histogram against uniform with 255 degrees of freedom. It is a quick `ent`-style sanity check, not a certification. Values near 255 are expected.

## Throughput

Not yet measured. No YubiKey was attached when this crate was written. Run `cahoots-yk bench` per model and record the results here. If a full 256 KiB contribution takes too long, a later version may allow a fixed hardware prefix with the rest from the platform CSPRNG. Version 1 requires the full pad length from hardware for the `TRNG` label.

| Model | Max challenge | Time for 256 KiB | KiB/s |
|---|---|---|---|
| | | | |

## How the bytes reach each client

- **Desktop or laptop client.** Browsers block WebUSB access to the smart-card interface, so the browser can't talk to a YubiKey directly. Run `cahoots-yk random` to a file and import that file into the desktop client. A native-messaging or localhost helper is a later option.
- **Phone client.** WebNFC can't send smart-card APDUs. Run `cahoots-yk` on a laptop and transfer the bytes to the phone over animated QR.
- **Cahoots badge.** The badge is a USB device, not a host, so a YubiKey can't plug into it. The laptop runs `cahoots-yk` and sends the bytes over the badge's USB link before the ceremony. The badge XORs them into its own TRNG output.

## Library

`cahoots_yk::OpenPgp<C: Card>` wraps any `Card` transport. `PcscCard` is the real one. It implements `cahoots_hal::Entropy`, so firmware test harnesses and the host simulator can use a YubiKey as an `Entropy` source directly.

On macOS the `pcsc` crate links the system PCSC framework. On Linux install `libpcsclite-dev` and run `pcscd`.

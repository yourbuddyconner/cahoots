# Screen bring-up: Arduino Nano ESP32 and the Waveshare 1.54" V2

The first thing to check on the ESP32-S3 family's hardware, before any firmware exists: does the manufactured badge's panel, driven from a board we can buy today, show the badge's screens the way [Build a badge](../../docs/diy-badge.md) claims? The sketch in `epaper_bringup/` draws those screens and prints how long each refresh takes. It is a C++ Arduino sketch, not the badge firmware, which will be Rust on `esp-hal` with the `epd-waveshare` crate's `epd1in54_v2` driver for this same panel.

## Parts

- Arduino Nano ESP32 with headers, ABX00083. An ESP32-S3 with 16 MB of flash and 8 MB of PSRAM, USB-C, 3.3 V logic.
- Waveshare 1.54" e-Paper Module V2: 200 × 200, SSD1681 controller, SPI, the 8-pin cable. The same panel as the manufactured badge.

## Wiring

Read the labels printed on the module's board, not the wire colours. The module takes 3.3 V or 5 V on VCC; use 3V3 so every signal matches the Nano's logic level.

| Module | Nano ESP32 | Signal |
|---|---|---|
| VCC | 3V3 | power |
| GND | GND | ground |
| DIN | D11 | SPI MOSI |
| CLK | D13 | SPI SCK |
| CS | D10 | chip select |
| DC | D9 | data / command |
| RST | D8 | reset |
| BUSY | D7 | busy, high while the panel refreshes |

## Running it

In the Arduino IDE:

1. Boards Manager: install "Arduino ESP32 Boards". Select the board "Arduino Nano ESP32". Tools > Pin Numbering: "By Arduino pin (default)".
2. Library Manager: install GxEPD2. It pulls in Adafruit GFX Library and Adafruit BusIO. The QR generator is part of the board package.
3. Open `epaper_bringup/epaper_bringup.ino`, upload, open the serial monitor at 115200.

If the upload cannot find the port, double-press the reset button: the LED fades green and the board is in its bootloader.

The same from a terminal, with `arduino-cli`:

```sh
arduino-cli core install arduino:esp32
arduino-cli lib install GxEPD2
arduino-cli compile --fqbn arduino:esp32:nano_nora hardware/nano-esp32-epaper/epaper_bringup
arduino-cli upload -p /dev/cu.usbmodem* --fqbn arduino:esp32:nano_nora hardware/nano-esp32-epaper/epaper_bringup
arduino-cli monitor -p /dev/cu.usbmodem* -c baudrate=115200
```

## What it shows, and what to write down

The sketch runs three cycles of five screens, then puts the panel to sleep. Press reset to run it again.

| Screen | Refresh | What to check |
|---|---|---|
| Words: circle name, tier label, six words in two columns, the confirm line | full | Can you read six 9pt mono words from a normal reading distance? This is the layout from the site's render, `wordsScreen()` in `site/badge.js`. Change `WORD_FONT` to 12pt to see larger type overflow on 8-letter words. |
| Status line counting "commits 1/4" to "4/4" | partial, footer band only | How long does a partial refresh take, and does the rest of the screen flicker? This is what `show_status()` does during a ceremony. |
| Version 4 QR at 6 px per module, 198 px wide | full | The DIY guide's claim: a phone reads it at arm's length. Try a phone camera from 60 cm. The quiet zone is only the panel's bezel. |
| The same QR at 5 px per module, 165 px wide | full | The fallback if the first needs a real quiet zone. |
| Waiting: "touching..." / "wait for the buzz" | full | Legibility of the ceremony prompt. |

Numbers worth recording: the full refresh time (about 2 s is normal for this panel), the partial refresh time, how many partial refreshes it takes before ghosting shows, and the distance at which each QR scans. Those feed the firmware's screen driver and the DIY guide's claims.

## What this does not test

One board tests one screen. The touch link needs a second board, or a laptop standing in as a peer over USB serial once the host crate has a serial link. The ceremony, the pad store on flash, and the entropy health tests need the ESP32-S3 crate, which does not exist yet: see [crates/esp32s3](../../crates/esp32s3/README.md).

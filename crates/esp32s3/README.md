# ESP32-S3 target (M4b)

Planned: `esp-hal` + Embassy `no_std` firmware for the ESP32-S3-DevKitC-1 (N8R8) and the Arduino Nano ESP32, selected by Cargo feature. Built outside the workspace with the esp toolchain. Flash with `espflash`.

Randomness gotcha to document and enforce: the S3 hardware RNG is true random only while an entropy source runs. Use `esp_hal::rng::Trng` with the ADC noise source on, keep the radio off, and run the SP 800-90B health tests. Not started.

Hardware on hand: an Arduino Nano ESP32 and the Waveshare 1.54" V2 panel. The screen bring-up sketch for that pair, with wiring, is in [hardware/nano-esp32-epaper](../../hardware/nano-esp32-epaper/README.md).

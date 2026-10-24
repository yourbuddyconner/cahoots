// Screen bring-up for the ESP32-S3 badge family.
// Board: Arduino Nano ESP32 (ABX00083). Panel: Waveshare 1.54" e-Paper Module V2, SSD1681, 200 x 200.
//
// This is not the badge firmware. It draws the screens the firmware will draw, so the panel, the cable
// and the legibility claims in docs/diy-badge.md can be checked on real glass before the firmware exists.
//
// Library, from the Arduino Library Manager: GxEPD2, which pulls in Adafruit GFX Library and Adafruit BusIO.
// The QR generator is the board package's own (the ESP-IDF qrcode component). Board package: "Arduino ESP32
// Boards", board "Arduino Nano ESP32", Tools > Pin Numbering > "By Arduino pin (default)".
//
// Wiring, module label -> Nano ESP32 pin:
//   VCC  -> 3V3          GND  -> GND
//   DIN  -> D11 (MOSI)   CLK  -> D13 (SCK)
//   CS   -> D10          DC   -> D9
//   RST  -> D8           BUSY -> D7
//
// It cycles these screens three times, printing each refresh time on the serial monitor at 115200,
// then puts the panel to sleep. Press reset to run it again.
//   1. The words screen: circle name, source label, six words, the confirm line.  Full refresh.
//   2. The status line counting "commits 1/4" to "commits 4/4".                   Partial refresh.
//   3. A version 4 QR at 6 px per module: the size the DIY guide says a phone reads at arm's length.
//   4. The same QR at 5 px per module, which leaves a quiet zone of about three modules.
//   5. The waiting screen: "touching..." / "wait for the buzz".

#include <GxEPD2_BW.h>
#include <Fonts/FreeMono9pt7b.h>
#include <Fonts/FreeMonoBold9pt7b.h>
#include <Fonts/FreeMonoBold12pt7b.h>
#include "qrcode.h" // the board package's generator, not the QRCode library

#define EPD_CS 10
#define EPD_DC 9
#define EPD_RST 8
#define EPD_BUSY 7

// The words font. 9pt bold mono fits two columns of 8-letter words in 200 px (11 px per letter).
// 12pt is closer to the manufactured badge's design but overflows the right column on 8-letter words:
// try it to see how much larger the type could be.
#define WORD_FONT FreeMonoBold9pt7b

GxEPD2_BW<GxEPD2_154_D67, GxEPD2_154_D67::HEIGHT> display(GxEPD2_154_D67(EPD_CS, EPD_DC, EPD_RST, EPD_BUSY));

static const char* WORDS[6] = {"sunset", "valve", "present", "raven", "curtain", "pudding"};
static const char* CIRCLE = "sf-meetup";
static const char* LABEL = "TRNG T2"; // an ESP32-S3 with its radio off is tier 2
// A stand-in for what the badge will encode: an address with 43 base64 characters, the size of a 32-byte hash.
static const char* QR_TEXT = "https://cahoots.network/#c=3kq1v9ZrWw2m8Hc5TtLxPfDgYn7eJbQ4uR6sVaK0oMI";

static uint32_t t0;
static void tick(const char* what) { t0 = millis(); Serial.printf("%-28s", what); }
static void tock() { Serial.printf("%5lu ms\n", (unsigned long)(millis() - t0)); }

// Mirrors wordsScreen() in site/badge.js: the same positions on the same 200 x 200 grid.
static void drawWords() {
  display.setTextColor(GxEPD_BLACK);
  display.setFont(&FreeMono9pt7b);
  display.setCursor(8, 19);
  display.print(CIRCLE);
  display.setFont(NULL); // the built-in 5 x 7 font plays the 9 px label role; its cursor is the glyph's top-left
  display.setTextSize(1);
  int16_t x1, y1;
  uint16_t w, h;
  display.getTextBounds(LABEL, 0, 0, &x1, &y1, &w, &h);
  display.setCursor(192 - w, 12);
  display.print(LABEL);
  display.fillRect(8, 26, 184, 1, GxEPD_BLACK);
  display.setFont(&WORD_FONT);
  for (int i = 0; i < 6; i++) {
    display.setCursor(8 + (i % 2) * 96, 62 + (i / 2) * 36);
    display.print(WORDS[i]);
  }
  display.fillRect(8, 170, 184, 1, GxEPD_BLACK);
  display.setFont(NULL);
  display.setCursor(8, 181);
  display.print("read aloud . B to confirm");
}

static void showWords() {
  tick("words, full refresh");
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    drawWords();
  } while (display.nextPage());
  tock();
}

// The footer band only: the firmware's show_status() during a ceremony.
static void showStatus(const char* s) {
  tick(s);
  display.setPartialWindow(0, 168, 200, 32);
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    display.fillRect(8, 170, 184, 1, GxEPD_BLACK);
    display.setFont(NULL);
    display.setTextColor(GxEPD_BLACK);
    display.setCursor(8, 181);
    display.print(s);
  } while (display.nextPage());
  tock();
}

static uint8_t qrPx; // the generator's display callback has no user pointer

static void drawQrModules(esp_qrcode_handle_t qr) {
  const int n = esp_qrcode_get_size(qr); // 33 modules for version 4
  const int16_t off = (200 - n * qrPx) / 2;
  Serial.printf("v%d, %d px wide, ", (n - 17) / 4, n * qrPx);
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    for (int y = 0; y < n; y++)
      for (int x = 0; x < n; x++)
        if (esp_qrcode_get_module(qr, x, y)) display.fillRect(off + x * qrPx, off + y * qrPx, qrPx, qrPx, GxEPD_BLACK);
  } while (display.nextPage());
}

static void showQr(uint8_t px) {
  qrPx = px;
  char what[32];
  snprintf(what, sizeof what, "QR, %u px/module", px);
  tick(what);
  esp_qrcode_config_t cfg;
  cfg.display_func = drawQrModules;
  cfg.max_qrcode_version = 4;                // the generator picks the smallest version that fits, capped here
  cfg.qrcode_ecc_level = ESP_QRCODE_ECC_MED; // 70 bytes need version 4 at this level: 33 x 33 modules
  if (esp_qrcode_generate(&cfg, QR_TEXT) != ESP_OK) Serial.print("QR text does not fit in version 4, ");
  tock();
}

static void showWaiting() {
  tick("waiting screen, full");
  display.setFullWindow();
  display.firstPage();
  do {
    display.fillScreen(GxEPD_WHITE);
    display.setTextColor(GxEPD_BLACK);
    display.setFont(&FreeMonoBold12pt7b);
    display.setCursor(8, 96);
    display.print("touching...");
    display.setFont(NULL);
    display.setCursor(8, 112);
    display.print("wait for the buzz");
  } while (display.nextPage());
  tock();
}

void setup() {
  Serial.begin(115200);
  uint32_t until = millis() + 3000;
  while (!Serial && millis() < until) delay(10); // runs unattended too
  Serial.println("cahoots screen bring-up: Nano ESP32 + Waveshare 1.54 V2");
  display.init(115200, true, 2, false); // 2 ms reset pulse: what Waveshare boards want
  display.setRotation(0);
}

void loop() {
  for (int cycle = 1; cycle <= 3; cycle++) {
    Serial.printf("--- cycle %d\n", cycle);
    showWords();
    delay(6000);
    for (int n = 1; n <= 4; n++) {
      char s[20];
      snprintf(s, sizeof s, "commits %d/4", n);
      showStatus(s);
      delay(1000);
    }
    delay(3000);
    showQr(6);
    delay(8000);
    showQr(5);
    delay(8000);
    showWaiting();
    delay(4000);
  }
  display.hibernate();
  Serial.println("done: panel asleep. Press reset to run again.");
  while (true) delay(1000);
}

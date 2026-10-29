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
// Two tactile switches, one leg to the pin and the other to GND, using the chip's pull-ups:
//   A (next)    -> D2      B (confirm) -> D3
// A four-pin switch has two pairs of legs joined inside it. Put the GND wire and the pin wire on
// legs from different pairs; if they share a pair the sketch sees the button held down at boot and says so.
//
// The screens are button-driven, as on the badge. A steps through them, B confirms on the ones that take it.
// Each refresh time is printed on the serial monitor at 115200.
//   1. The words screen: circle name, source label, six words, the confirm line.  B: footer "confirmed", partial.
//   2. A version 4 QR at 6 px per module: the size the DIY guide says a phone reads at arm's length.
//   3. The same QR at 5 px per module, which leaves a quiet zone of about three modules.
//   4. The waiting screen: "touching..." / "wait for the buzz".  B: counts "commits n/4" on the footer, partial.
// After two idle minutes the panel sleeps; the next press redraws the screen.

#include <GxEPD2_BW.h>
#include <Fonts/FreeMono9pt7b.h>
#include <Fonts/FreeMonoBold9pt7b.h>
#include <Fonts/FreeMonoBold12pt7b.h>
#include "FreeMonoBold10pt7b.h" // the words: FreeMono Bold 15% larger than the stock 9pt, see the README
#include "qrcode.h" // the board package's generator, not the QRCode library
#include "types.h"

#define EPD_CS 10
#define EPD_DC 9
#define EPD_RST 8
#define EPD_BUSY 7
#define BTN_A 2 // next
#define BTN_B 3 // confirm

// The words font, 12 px per letter. The left column is left-aligned and the right column right-aligned,
// so a row holds up to 15 letters plus a 4 px gap. The one pair that cannot fit, 8 + 8 letters, is 0.2%
// of rows; that screen falls back to WORD_FONT_SMALL, the stock 9pt at 11 px per letter.
#define WORD_FONT FreeMonoBold10pt7b
#define WORD_FONT_SMALL FreeMonoBold9pt7b

GxEPD2_BW<GxEPD2_154_D67, GxEPD2_154_D67::HEIGHT> display(GxEPD2_154_D67(EPD_CS, EPD_DC, EPD_RST, EPD_BUSY));

static const char* WORDS[6] = {"sunset", "valve", "present", "raven", "curtain", "pudding"};
static const char* CIRCLE = "sf-meetup";
static const char* LABEL = "TRNG T2"; // an ESP32-S3 with its radio off is tier 2
// A stand-in for what the badge will encode: a bare address plus 43 base64 characters, the size of a 32-byte hash.
// 62 bytes, which is exactly what a version 4 code holds at error level M.
static const char* QR_TEXT = "cahoots.network/#c=3kq1v9ZrWw2m8Hc5TtLxPfDgYn7eJbQ4uR6sVaK0oMI";

static uint32_t t0;
static void tick(const char* what) { t0 = millis(); Serial.printf("%-28s", what); }
static void tock() { Serial.printf("%5lu ms\n", (unsigned long)(millis() - t0)); }

// Width of a string in the current font, as the renderer will draw it.
static uint16_t textWidth(const char* s, int16_t* x1) {
  int16_t y1;
  uint16_t w, h;
  display.getTextBounds(s, 0, 0, x1, &y1, &w, &h);
  return w;
}

// Three rows of two words between x = 8 and x = 192: left word left-aligned, right word right-aligned.
// Measures every row first; if one would not leave a 4 px gap, the whole grid drops to the small font.
static void drawWordGrid() {
  const GFXfont* font = &WORD_FONT;
  display.setFont(font);
  for (int r = 0; r < 3; r++) {
    int16_t x1;
    uint16_t wl = textWidth(WORDS[2 * r], &x1), wr = textWidth(WORDS[2 * r + 1], &x1);
    if (wl + 4 + wr > 184) {
      font = &WORD_FONT_SMALL;
      Serial.print("(a row of 16 letters: small font) ");
      break;
    }
  }
  display.setFont(font);
  for (int r = 0; r < 3; r++) {
    const int16_t y = 62 + r * 36;
    display.setCursor(8, y);
    display.print(WORDS[2 * r]);
    int16_t x1;
    uint16_t w = textWidth(WORDS[2 * r + 1], &x1);
    display.setCursor(192 - w - x1, y);
    display.print(WORDS[2 * r + 1]);
  }
}

// Mirrors the words screen in crates/badge/src/screens.rs: the same positions on the same 200 x 200 grid.
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
  drawWordGrid();
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
  cfg.qrcode_ecc_level = ESP_QRCODE_ECC_MED; // 62 bytes fill version 4 at this level: 33 x 33 modules
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

static Btn btnA = {BTN_A, "A", false, 0}, btnB = {BTN_B, "B", false, 0};

// True once per press, on the press edge, with a 30 ms debounce. Every change is logged, so a
// button that never says "down" is open, and one that says "down" once and never "up" is shorted.
static bool pressed(Btn& b) {
  bool now = digitalRead(b.pin) == LOW;
  if (now != b.down && millis() - b.t > 30) {
    b.down = now;
    b.t = millis();
    Serial.printf("[%s %s] ", b.name, now ? "down" : "up");
    return now;
  }
  return false;
}

static Screen screen = SCREEN_WORDS;
static int commits = 0;
static uint32_t lastPress = 0;
static bool asleep = false;

static void show(Screen s) {
  asleep = false;
  switch (s) {
    case SCREEN_WORDS: showWords(); break;
    case SCREEN_QR6: showQr(6); break;
    case SCREEN_QR5: showQr(5); break;
    case SCREEN_WAITING: commits = 0; showWaiting(); break;
  }
}

void setup() {
  Serial.begin(115200);
  uint32_t until = millis() + 3000;
  while (!Serial && millis() < until) delay(10); // runs unattended too
  Serial.println("cahoots screen bring-up: Nano ESP32 + Waveshare 1.54 V2, two buttons");
  pinMode(BTN_A, INPUT_PULLUP);
  pinMode(BTN_B, INPUT_PULLUP);
  delay(50);
  for (Btn* b : {&btnA, &btnB}) {
    b->down = digitalRead(b->pin) == LOW; // a held button is not a press
    if (b->down)
      Serial.printf("button %s reads held down at boot: its two wires are joined, by the switch's own pair or by a breadboard row. Move one.\n", b->name);
  }
  // 1 MHz, not the library's 10 MHz: at 10 MHz nothing got through the module's level shifter and the
  // jumper wires, and every refresh "finished" in a few milliseconds with a blank panel. A frame is 5000
  // bytes, so 1 MHz costs 40 ms. Raise it later with short wires or a soldered build.
  display.epd2.selectSPI(SPI, SPISettings(1000000, MSBFIRST, SPI_MODE0));
  display.init(115200, true, 2, false); // 2 ms reset pulse: what Waveshare boards want
  display.setRotation(0);
  show(screen);
  lastPress = millis();
  Serial.println("A: next screen. B: confirm.");
}

void loop() {
  if (pressed(btnA)) {
    lastPress = millis();
    screen = (Screen)((screen + 1) % 4);
    Serial.print("A -> ");
    show(screen);
  }
  if (pressed(btnB)) {
    lastPress = millis();
    Serial.print("B -> ");
    if (asleep) show(screen);
    else if (screen == SCREEN_WORDS) showStatus("confirmed");
    else if (screen == SCREEN_WAITING) { char s[20]; commits = commits % 4 + 1; snprintf(s, sizeof s, "commits %d/4", commits); showStatus(s); }
    else Serial.println("nothing to confirm here");
  }
  if (!asleep && millis() - lastPress > 120000) {
    display.hibernate();
    asleep = true;
    Serial.println("idle: panel asleep, the image stays. Any press redraws.");
  }
  delay(5);
}

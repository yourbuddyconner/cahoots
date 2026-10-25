# V1: the first badge you can order

A 54 × 86 mm two-layer board that a Raspberry Pi Pico 2 solders onto. Front: the raw 1.54" e-paper (Good Display GDEY0154D67) on its 24-pin flex, two tactile switches, five gold pads for the card above, a lanyard slot. Back: the Pico 2 crosswise with its USB at the right edge, a CR2032 holder, a power switch, the panel's boost circuit, five spring contacts for the card below, four pockets for 5 × 2 mm magnets. About 6 mm thick with the panel on.

The plan it belongs to is [docs/manufacturing.md](../../docs/manufacturing.md). The hand-built version that comes first, a Pico 2 and the Waveshare module on a proto board, uses the same pin map; the table is at the bottom.

## Status

Drawn and checked, not yet reviewed or ordered. The schematic passes KiCad's electrical check, the board passes its design rule check, and the three agree on every net (`check.py`). What has not happened: a person looking at the 3D view and a 1:1 printout, the part numbers confirmed against the assembler's stock, and the flex connector's contact side confirmed against the panel's flex. See the review list.

## Files

| File | What |
|---|---|
| `design.py` | The single source: every part, pad, net and position, the Pico pin map, the panel pinout. |
| `make_sch.py` | Draws `v1.kicad_sch` from it: each part with a labelled stub on every pin. Plain python3. |
| `make_pcb.py` | Builds `v1.kicad_pcb`: footprints placed from the table, outline, slot, pours, edge keep-outs. KiCad's Python. |
| `route.py` | Exports the board for Freerouting and imports the result, then fills the pours. KiCad's Python. |
| `check.py` | Schematic netlist versus `design.py` versus the board. KiCad's Python. |
| `make_bom.py` | `bom.csv` in the assembler's columns. |
| `make.sh`, `hero.sh` | The whole pipeline; the angled two-sided render. |
| `v1.kicad_pro`, `v1.kicad_sch`, `v1.kicad_pcb`, `bom.csv` | The generated project. Open `v1.kicad_pro` in KiCad to look. |
| `schematic.png`, `front.png`, `back.png`, `hero.png` | Renders of the schematic, both sides of the board, and an angled view of both for an article. |
| `models/panel.wrl` | A box model of the panel, so the 3D view shows the glass and the flex. |

Regenerate everything:

```sh
cd hardware/v1
PY=/Applications/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
python3 make_sch.py && python3 make_bom.py
$PY make_pcb.py && $PY route.py export
java -jar freerouting.jar -de v1.dsn -do v1.ses -mp 80 -da -dl     # Freerouting 2.1 on Java 21+
$PY route.py import && $PY check.py
kicad-cli pcb drc --severity-error v1.kicad_pcb
kicad-cli pcb render --side top -o front.png v1.kicad_pcb
kicad-cli sch export svg -o out v1.kicad_sch          # schematic.png is a rendering of that SVG
```

Edit `design.py`, never the generated files: the next run overwrites them.

## What is on it

**Pico 2.** Soldered flat through its castellations, components facing out, USB overhanging the right edge, BOOTSEL reachable. Flashing is drag-and-drop over that USB. The library footprint is the Pico 1's; the Pico 2 has no castellated debug pads, so those three are dropped.

**Panel.** The flex leaves the panel's bottom edge 8.5 mm from its left corner and runs straight down into the connector on the front. The boost circuit is the panel specification's reference circuit, page 28: 47 µH, a MOSFET switched by the panel's own GDR pin, one Schottky up to VGH, a capacitor and two Schottkys pumping VGL negative, a 2.2 Ω sense resistor, and a 1 µF on each driven rail. BS1 to ground selects 4-wire SPI. The MOSFET is the AO3400A rather than the reference's Si1308EDL because it is a stocked basic part; its 30 V rating clears the panel's gate voltages.

**Power.** CR2032 into a slide switch, through a Schottky into the Pico's VSYS. The Pico's buck-boost makes 3.3 V from anything above 1.8 V. The diode keeps USB power out of the cell. Whether a coin cell survives a ceremony is the open measurement in the manufacturing plan; the holder is the easy part to swap.

**Buttons.** A is next, B is confirm, each to ground on the chip's pull-ups.

**Pile contacts.** Five on the face, left to right: GND, TX, RX, GND, DET. Five spring contacts on the back at the same places: GND, RX, TX, DET, GND. Stack two the same way up and contact i meets contact i: grounds meet, each TX meets the other's RX, and each card's detect input lands on the other card's ground, so both know the other is there. Up is UART0, down is UART1.

## Pin map

| Signal | Pico 2 GPIO | Pico pin | Hand build: Waveshare module / part |
|---|---|---|---|
| EPD_SCK | GP18 | 24 | CLK |
| EPD_MOSI | GP19 | 25 | DIN |
| EPD_CS | GP17 | 22 | CS |
| EPD_DC | GP16 | 21 | DC |
| EPD_RST | GP20 | 26 | RST |
| EPD_BUSY | GP21 | 27 | BUSY |
| BTN_A | GP14 | 19 | switch A to GND |
| BTN_B | GP15 | 20 | switch B to GND |
| UP_TX | GP0 | 1 | pad 2 |
| UP_RX | GP1 | 2 | pad 3 |
| UP_DET | GP2 | 4 | pad 5 |
| DN_RX | GP5 | 7 | finger 2 |
| DN_TX | GP4 | 6 | finger 3 |
| DN_DET | GP3 | 5 | finger 4 |
| 3V3 | 3V3 | 36 | module VCC |
| GND | GND | 3, 8, 13, 18, 23, 28, 38 | module GND, switches, pads 1 and 4 |
| VSYS | VSYS | 39 | CR2032 + through the switch and a Schottky |

The Pico 2 firmware must use this table. For the hand build the Waveshare module replaces the panel, the connector and the whole boost circuit, since it carries them on its own board.

## Review before ordering

1. **Flex connector contact side.** The footprint is a bottom-contact, flip-lock 0.5 mm connector. The panel's flex must present its contacts on the matching side when it runs straight down from the panel. If the flex's contacts face the other way, swap to a top-contact part of the same footprint family, or fold the flex. This is the classic mistake with e-paper; check it against a real panel in hand.
2. **Connector orientation.** Pin 1 of the connector must meet pin 1 of the flex, which is the panel's left pin in its drawing. Confirm in the 3D view with the flex overlaid.
3. **Part numbers.** Every "verify" in `bom.csv`: the inductor, the sense resistor, the battery holder, the pogo pins, the slide switch. Confirm stock and footprint against the datasheet in the assembler's library before upload.
4. **Battery holder footprint.** Drawn as MYOUNG BS-07; if the stocked holder is another family, the footprint changes.
5. **Pogo pins.** The footprint is a 2.4 mm round pad. Confirm the pin's base diameter and that its free height clears the tray; the pad on the face of the card below is 3.5 mm, enough for the magnets' alignment tolerance.
6. **Thickness and the tray.** Panel 1 mm plus the connector's 2 mm on the front; Pico 1 mm plus its parts and the holder on the back. The tray design is not here yet.
7. **Look at it.** Open `v1.kicad_pro`, press Alt-3 for the 3D view, and print the front at 1:1.

## Not in V1

A secure element, flash encryption, the dock, the thin cell, the final tray. Those are V2's questions once the power measurement is in.

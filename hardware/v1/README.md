# V1: the first badge you can order

A 54 × 86 mm two-layer board that a Raspberry Pi Pico 2 solders onto. Front: the raw 1.54" e-paper (Good Display GDEY0154D67) on its 24-pin flex, two tactile switches, the pin half of a magnetic pogo connector at the top end with its pins up through the lid, two JST-SH sockets and a Qwiic socket at the bottom edge, and a three-turn NFC loop around the board's edge. Back: the Pico 2 crosswise with its USB at the right edge, a CR2032 holder, a power switch, the panel's boost circuit, the NXP SE051 secure element with a switch that opens its antenna, a 10-pin expansion socket, four pockets for 5 × 2 mm magnets.

The plan it belongs to is [docs/manufacturing.md](../../docs/manufacturing.md). The hand-built version that comes first, a Pico 2 and the Waveshare module on a proto board, uses the same pin map; the table is at the bottom.

## Status

Drawn, checked and quoted (2026-10-27, with the secure element and the loop): five boards, through ProvenMetal at JLCPCB, about to be ordered and not yet received. The schematic passes KiCad's electrical check, the board passes its design rule check, the three agree on every net (`check.py`), and every part in `bom.csv` is in JLCPCB's stock under the code written next to it (checked 2026-10-27 through ProvenMetal). What has not happened: a person looking at the 3D view and a 1:1 printout, and the flex connector's contact side confirmed against the panel's flex. See the review list.

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
| `case.py`, `case/` | The tray and lid: STEP, STL for the printer, transparent WRL for the render. |
| `explode.py`, `assembly.png` | The parts sheet: the badge exploded into lid, panel, board, cell, pins and tray beside the assembled badge, with a numbered list. |
| `parts.py`, `parts.png` | Each piece alone, front and back: lid, panel, board, tray. |
| `models/` | Models for the parts the libraries lack: panel, magnets, cell, the two pogo halves. |

Regenerate everything:

```sh
cd hardware/v1
PY=/Applications/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
python3 make_sch.py && python3 make_bom.py
$PY make_pcb.py && $PY route.py export
java -jar freerouting.jar -de v1.dsn -do v1.ses -mp 400 -da        # Freerouting 2.5 on Java 25; 2.1 never finished this board, and 2.5 needs the passes
$PY route.py import && $PY check.py
kicad-cli pcb drc --severity-error v1.kicad_pcb
kicad-cli pcb render --side top -o front.png v1.kicad_pcb
kicad-cli sch export svg -o out v1.kicad_sch          # schematic.png is a rendering of that SVG
```

Edit `design.py`, never the generated files: the next run overwrites them.

## What is on it

**Pico 2.** Soldered flat through its castellations, components facing out, USB overhanging the right edge, BOOTSEL reachable. Flashing is drag-and-drop over that USB. The library footprint is the Pico 1's; the Pico 2 has no castellated debug pads, so those three are dropped.

**Panel.** The flex leaves the panel's bottom edge 8.5 mm from its left corner and runs straight down into the connector on the front. The boost circuit is the panel specification's reference circuit, page 28: 47 µH, a MOSFET switched by the panel's own GDR pin, one Schottky up to VGH, a capacitor and two Schottkys pumping VGL negative, a 2.2 Ω sense resistor, and a 1 µF on each driven rail. BS1 to ground selects 4-wire SPI. The MOSFET is the AO3400A rather than the reference's Si1308EDL because it is a stocked basic part; its 30 V rating clears the panel's gate voltages.

**Rules.** Tracks are 0.25 mm with 0.2 mm clearance everywhere the router draws them. The board's minimums are JLCPCB's floor, 0.127 mm for both, which only the SE051's escape stubs use: its pads are on a 0.4 mm pitch, and each used pad gets a short locked stub at that width before the router takes over.

**Power.** CR2032 into a slide switch, through a Schottky into the Pico's VSYS. The Pico's buck-boost makes 3.3 V from anything above 1.8 V. The diode keeps USB power out of the cell. Whether a coin cell survives a ceremony is the open measurement in the manufacturing plan; the holder is the easy part to swap.

**Buttons.** A is next, B is confirm, each to ground on the chip's pull-ups.

**Secure element.** U2 on the back at the top right is the NXP SE051C2 in its 3 × 3 mm HX2QFN20, on I2C0 beside the Qwiic socket, with 3.3 kΩ pull-ups on the bus (a Qwiic board adds its own in parallel, which is fine at 3.3 V). Its ENA pin is on GP2 through a 100 kΩ pull-up: the chip is on until the firmware pulls the pin low for deep power-down. VOUT is tied to VCC on the board, as the data sheet asks when ENA is used, so the chip's own rail (SE_VCC) has its own 100 nF. RST_N is tied high; the ISO 7816 pins are left open; the centre pad is grounded.

**Antenna.** ANT1 is a rectangular spiral on the front around the whole board, just inside the magnet pockets: three turns of 0.4 mm copper on a 0.8 mm pitch, 52 × 71 mm outside, its two ends at the top-right corner over the chip. It is drawn as copper lines inside the ANT1 footprint, a net tie between its two pads, so the board's rules understand that one piece of copper joins the switch's net to the chip's. The SE051 resonates its antenna with 56 pF, which asks for about 2.5 µH at 13.56 MHz; the loop is about 1.4 µH in free space (`design.py` prints the estimate), and C15, 39 pF across LA and LB at the chip, brings it to 13.6 MHz, with C16's footprint (27 pF) left empty for trimming. The Pico, the cell and the back pour sit inside the loop and will pull the inductance down, so expect the first measurement to land high and need C16. SW4, the same slide switch as the power switch, is in the LA line between the chip and the loop: open, the loop is not a closed conductor and the chip sees only C15. The ground pours stay 1.2 mm clear of the turns on both layers, the front has no pour inside the loop, and no tracks or vias cross the turns on the front, which is why the three sockets at the bottom edge, the only parts outside the loop, each get a via of their own above their contacts. The firmware keeps the contactless interface disabled regardless of the switch; both off is the shipped state. (A loop around the panel alone was drawn first; its bottom edge ran along the Pico's lower pad row and the router could not get the top half of the board out.)

**Peripherals.** J5 at the bottom centre of the front is a Qwiic (STEMMA QT) socket, the same JST-SH 4-pin part as the pile ports, in the Qwiic pin order: 1 GND, 2 3V3, 3 SDA, 4 SCL, on the Pico's I2C0 (GP8 and GP9). Any Qwiic board plugs in with its own cable. J6 on the back, under the Qwiic socket at the bottom edge, is a 10-pin JST-SH socket, the same low side-entry family as the three front sockets: 3V3, GND and eight of the Pico's spare GPIO, for whatever a firmware fork needs to drive, on a cable out of the bottom end. Nothing else on the board uses those pins. (It was a 2x5 pin header until 2026-10-28; the badge is meant to stay low, and a header is 8 mm tall.)

**Pile links.** Badges stack face to face. J4 at the top end of the front is the pin half of an Adafruit 5413 magnetic connector (Weilichuang MG005): five pogo pins on 2.54 mm, a magnet at each end, 2 A rated, 0.9 mm stroke, standing on the board with its pins up through a window in the lid, 1.2 mm proud of the top. The pad half is not on the board: it sits face down in a window in the tray floor, flush with the outside, and a short JST-SH pigtail wires it to the DN socket J3 (its pin 1 to J3 pin 1, pin 2 to J3 pin 3, pin 4 to J3 pin 4). Stack two badges and the lower one's pins meet the upper one's pads, pin for pin, the magnets holding them; pins 1 and 5 are ground, 2 and 4 the UART, 3 spare. Pin 3 carries no power: feeding one badge's regulator from another's is a question for V2, and two UART lines and ground are the pile. J2 and J3 are JST-SH 4-pin sockets at the bottom edge, UP and DN, for a cable to a badge on the bench or a laptop; a straight cable works because TX and RX are crossed between the two. There is no detect wire: a connected peer's TX holds our RX high, and the firmware keeps a pull-down on each RX. UP is UART0, DN is UART1.

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
| I2C_SDA | GP8 | 11 | J5 pin 3 (Qwiic SDA), U2 pin 9 |
| I2C_SCL | GP9 | 12 | J5 pin 4 (Qwiic SCL), U2 pin 10 |
| SE_ENA | GP2 | 4 | U2 pin 11; low is deep power-down, 100 kΩ pull-up |
| GP6, GP7 | GP6, GP7 | 9, 10 | J6 pins 3, 4 (J6 pin 1 is 3V3, pin 2 GND) |
| GP10, GP11, GP12, GP13 | GP10 to GP13 | 14, 15, 16, 17 | J6 pins 5, 6, 7, 8 |
| GP22, GP26 | GP22, GP26 | 29, 31 | J6 pins 9, 10 |
| UP_TX | GP0 | 1 | J4 pin 2, J2 pin 3 |
| UP_RX | GP1 | 2 | J4 pin 4, J2 pin 4; pull-down, high means a peer |
| DN_RX | GP5 | 7 | J3 pin 3, and through the pigtail the floor pad 2; pull-down, high means a peer |
| DN_TX | GP4 | 6 | J3 pin 4, floor pad 4 |
| 3V3 | 3V3 | 36 | module VCC |
| GND | GND | 3, 8, 13, 18, 23, 28, 38 | module GND, switches, pogo pins 1 and 5, both sockets' pin 1 |
| VSYS | VSYS | 39 | CR2032 + through the switch and a Schottky |

The Pico 2 firmware must use this table. For the hand build the Waveshare module replaces the panel, the connector and the whole boost circuit, since it carries them on its own board.

## The case

Two printed parts that click together, drawn by `case.py` from the same table (`CASE` in `design.py`), in `case/`: `tray.step`/`.stl` and `lid.step`/`.stl`, plus `.wrl` copies KiCad shows in the 3D view, transparent like the acrylic version will be.

- **Tray.** The board drops in face up onto a ledge. The floor sits 5.6 mm under the board, past the Pico and the cell in its holder, with a notch for the USB socket, three openings in the end wall for the cables (the middle one continues below the board for the expansion socket on the back), two openings in the floor under the slide switches, the pad half's window, four magnet pockets.
- **Lid.** A 2 mm plate on a 4 mm skirt that wraps outside the tray's wall. The panel is taped to the board, not to the lid, so the board comes out of the case as one unit with its screen (decided 2026-10-28; the first drawing glued the panel into the lid for a flush top). The plate's window shows the active area with a 1.5 mm lip over the panel's edge, and a frame under the lip comes down to 0.15 mm above the glass so the panel cannot lift, with a gap where the flex leaves; the glass sits about 3 mm below the lid's top. Two 7 mm holes over the switch stems, the pogo window, three socket windows, four magnet pockets, and the cable openings continued through its skirt, which reaches down past the board to the snaps. Four ribs press the board onto the tray's ledge.
- **The click.** Four wedge bumps on the inside of the skirt, on short slit tabs so they flex, drop into four windows through the tray's wall. PETG flexes better than PLA for this; if a tab cracks, make `snap` smaller or `skirt_t` thinner in the table. Screws are the fallback and need no redesign: the corner pockets become holes.
- **Printing.** Tray floor down, lid plate down, no supports needed except the USB notch bridge. Clear PETG shows the board; resin or cut acrylic later from the same STEP.
- **Sizes.** Tray 57.8 × 89.8 mm; the badge in its case 60.4 × 92.4 × 15 mm.

The lid's cavity is 2.2 mm high, enough for the flex connector and the taped panel (1.15); the pogo body (4.0) and the socket bodies (4.3) rise into windows in the 2 mm plate, and the pad half sits in a matching window in the floor. The badge is 13.4 mm thick.

Regenerate: `python3 case.py` with CadQuery installed (`python3 -m venv cq && cq/bin/pip install cadquery && cq/bin/python case.py`), then `make.sh` picks the models up.

## Review before ordering

1. **Flex connector contact side.** The footprint is a bottom-contact, flip-lock 0.5 mm connector. The panel's flex must present its contacts on the matching side when it runs straight down from the panel. If the flex's contacts face the other way, swap to a top-contact part of the same footprint family, or fold the flex. This is the classic mistake with e-paper; check it against a real panel in hand.
2. **Connector orientation.** Pin 1 of the connector must meet pin 1 of the flex, which is the panel's left pin in its drawing. Confirm in the 3D view with the flex overlaid.
3. **Part numbers.** Done 2026-10-27: every line in `bom.csv` resolves to a part in JLCPCB's stock (the 2.2 Ω sense resistor had the code of a 22 Ω part; fixed). The Pico 2 and the pogo pins are marked not fitted and soldered by hand; so is C16, the antenna trim.
4. **Battery holder footprint.** Drawn as MYOUNG BS-07; if the stocked holder is another family, the footprint changes.
5. **Pogo pair.** The footprint is drawn from the Weilichuang drawing, not a library part: check the 1.0 mm drills on 2.54 mm and the 4.0 mm body height against a real one before ordering boards. Print a tray and lid and stack two: the pins should land on the pads with the magnets pulling.
6. **Thickness and the tray.** Panel 1 mm plus the connector's 2 mm on the front; Pico 1 mm plus its parts and the holder on the back. The tray design is not here yet.
7. **Look at it.** Open `v1.kicad_pro`, press Alt-3 for the 3D view, and print the front at 1:1.
8. **The antenna.** When boards arrive: with SW4 closed and the chip's contactless interface enabled, read the chip with an NFC reader and note the distance. If it reads only when touching or not at all, measure the loop's resonance (a nanoVNA across LA and LB with the chip out of circuit, or by sweeping the reader) and fit C16 to pull it to 13.56 MHz. The loop geometry and the two capacitor footprints are in `design.py`; the exposed-pad size of the QFN-20 footprint is KiCad's generic 1.65 mm, not NXP's drawing, which only matters for the solder joint of a pad that is electrically unused.
9. **The switches.** SW1 and SW4 are on the back, facing the tray floor, which has a 9 × 5 mm opening under each (`switch_win` in the case table) for a fingernail. Check the openings land on the actuators with a printed tray before gluing anything.

## Not in V1

Flash encryption in firmware, the dock, the thin cell, the final tray. Those are V2's questions once the power measurement is in.

"""V1: the first Cahoots badge you can order. The single source for the schematic and the board.

A 54 x 86 mm two-layer board that a Raspberry Pi Pico 2 solders onto. On the front: the raw
1.54" e-paper (GDEY0154D67) on its 24-pin flex, two tactile switches, five gold pads for the
card above. On the back: the Pico 2, the coin cell, a power switch, the panel's boost
circuit, five spring contacts for the card below, and four magnet pockets.

Coordinates are millimetres from the board centre, x to the right, y up, as seen from the
front. KiCad's y runs down; make_pcb.py flips it. Front parts are on F.Cu, back parts on B.Cu.

Sources: GDEY0154D67 specification 24.10.22 (pinout page 7, drawing page 6, reference circuit
page 28); Raspberry Pi Pico 2 datasheet (pinout figure 2, SMT footprint figure 5).
"""

BOARD_W, BOARD_H, CORNER_R = 54.0, 86.0, 3.0
MAGNETS = [(-21, 39), (21, 39), (-21, -39), (21, -39)]  # 5 x 2 mm discs, glued into 5.1 mm holes
# Pile links: badges stack face to face. A straight magnetic pogo pair, Adafruit 5413 (Weilichuang
# MG005, drawings C-8100121-5 pins and C-8200121-5 pads): the pin half J4 stands on the board's front at
# the top end, its pins up through a window in the lid; the pad half sits in a window in the tray's
# floor, face down and flush, wired to the DN socket J3 with a short JST-SH pigtail. Stack two badges
# and the pins of the lower one meet the pads of the upper one. Two JST-SH sockets at the bottom edge
# take a cable instead, for a badge on the bench or a laptop.
POGO_Y = 40.0                                        # the pin row, where the lanyard slot was
POGO = dict(w=20.08, d=4.2, tall=4.0, pitch=2.54, pin_to_back=1.75)   # body width, depth, height; pins 1.75 from the back face
PORT_Y = -39.5
PORTS = {"UP": 13.0, "DN": -13.0}                    # x of each JST socket

# Pico 2 pin number -> name, from the datasheet's figure 2.
PICO_PINS = {
    1: "GP0", 2: "GP1", 3: "GND", 4: "GP2", 5: "GP3", 6: "GP4", 7: "GP5", 8: "GND", 9: "GP6",
    10: "GP7", 11: "GP8", 12: "GP9", 13: "GND", 14: "GP10", 15: "GP11", 16: "GP12", 17: "GP13",
    18: "GND", 19: "GP14", 20: "GP15", 21: "GP16", 22: "GP17", 23: "GND", 24: "GP18", 25: "GP19",
    26: "GP20", 27: "GP21", 28: "GND", 29: "GP22", 30: "RUN", 31: "GP26", 32: "GP27", 33: "AGND",
    34: "GP28", 35: "ADC_VREF", 36: "3V3", 37: "3V3_EN", 38: "GND", 39: "VSYS", 40: "VBUS",
}

# Which Pico GPIO does what. The firmware's pin map must match this table.
GPIO = {
    "EPD_SCK": "GP18", "EPD_MOSI": "GP19", "EPD_CS": "GP17", "EPD_DC": "GP16",
    "EPD_RST": "GP20", "EPD_BUSY": "GP21",
    "BTN_A": "GP14", "BTN_B": "GP15",
    "UP_TX": "GP0", "UP_RX": "GP1",     # UART0, the badge above. A peer is present while its TX holds our RX high.
    "DN_TX": "GP4", "DN_RX": "GP5",     # UART1, the badge below. Same rule: the RX pins carry pull-downs in firmware.
}

def pico_pin(name):
    return next(n for n, v in PICO_PINS.items() if v == name)

# Panel flex pins, GDEY0154D67 page 7.
EPD_PINS = {
    1: "NC", 2: "GDR", 3: "RESE", 4: "NC", 5: "VSH2", 6: "TSCL", 7: "TSDA", 8: "BS1", 9: "BUSY",
    10: "RES", 11: "DC", 12: "CS", 13: "SCL", 14: "SDA", 15: "VDDIO", 16: "VCI", 17: "VSS",
    18: "VDD", 19: "VPP", 20: "VSH1", 21: "VGH", 22: "VSL", 23: "VGL", 24: "VCOM",
}

# ref: (symbol "Lib:Name", footprint "Lib:Name", value, LCSC or note, side, (x, y), rotation, {pad: net})
# Pads are the footprint's pad numbers. Nets are plain names; GND and 3V3 are the power nets.
PARTS = {}

def part(ref, symbol, footprint, value, lcsc, side, at, rot, pins):
    PARTS[ref] = dict(symbol=symbol, footprint=footprint, value=value, lcsc=lcsc, side=side, at=at, rot=rot, pins=pins)

# ---- the Pico 2, on the back, crosswise, USB overhanging the right edge ----
pico_nets = {str(n): v for n, v in PICO_PINS.items() if v in ("GND",)}
pico_nets.update({"33": "GND"})
for signal, gp in GPIO.items():
    pico_nets[str(pico_pin(gp))] = signal
pico_nets[str(pico_pin("3V3"))] = "3V3"
pico_nets[str(pico_pin("VSYS"))] = "VSYS"
part("U1", "cahoots:Pico2", "Module:RaspberryPi_Pico_SMD", "Raspberry Pi Pico 2 (no headers, no W)",
     "consigned: solder by hand", "back", (0.6, 10.0), 90, pico_nets)

# ---- the panel and its flex connector, on the front ----
epd_nets = {
    "2": "GDR", "3": "RESE", "5": "VSH2", "8": "GND", "9": "EPD_BUSY", "10": "EPD_RST",
    "11": "EPD_DC", "12": "EPD_CS", "13": "EPD_SCK", "14": "EPD_MOSI", "15": "3V3", "16": "3V3",
    "17": "GND", "18": "VDD", "20": "VSH1", "21": "VGH", "22": "VSL", "23": "VGL", "24": "VCOM",
    "MP": "GND",
}
part("J1", "Connector_Generic:Conn_01x24", "Connector_FFC-FPC:Hirose_FH12-24S-0.5SH_1x24-1MP_P0.50mm_Horizontal",
     "FPC 24P 0.5mm bottom contact", "C2856805 XUNPU FPC-05F-24PH20 (verify contact side against the flex)",
     "front", (-7.4, -12.0), 0, epd_nets)

# ---- the boost circuit, GDEY0154D67 page 28, on the back under the panel ----
part("L1", "Device:L", "Inductor_SMD:L_Changjiang_FNR4030S", "47uH 500mA", "FNR4030S470MT (verify)", "back", (-21.0, -6.0), 0, {"1": "3V3", "2": "SW"})
part("C4", "Device:C", "Capacitor_SMD:C_0805_2012Metric", "4.7uF 25V", "C1779", "back", (-21.0, -11.0), 0, {"1": "3V3", "2": "GND"})
part("Q1", "Transistor_FET:AO3400A", "Package_TO_SOT_SMD:SOT-23", "AO3400A", "C20917 (Si1308EDL in the reference; 30 V, 5.7 A)", "back", (-15.0, -6.0), 0, {"1": "GDR", "2": "RESE", "3": "SW"})
part("R1", "Device:R", "Resistor_SMD:R_0603_1608Metric", "1M", "C22935", "back", (-16.5, -10.0), 0, {"1": "GDR", "2": "GND"})
part("R2", "Device:R", "Resistor_SMD:R_0603_1608Metric", "2.2R", "C23345 (verify)", "back", (-13.0, -10.0), 0, {"1": "RESE", "2": "GND"})
part("D3", "Device:D_Schottky", "Diode_SMD:D_SOD-123", "MBR0530", "C77336", "back", (-9.5, -6.0), 0, {"1": "VGH", "2": "SW"})
part("C3", "Device:C", "Capacitor_SMD:C_0805_2012Metric", "4.7uF 25V", "C1779", "back", (-9.5, -10.0), 0, {"1": "SW", "2": "N2"})
part("D1", "Device:D_Schottky", "Diode_SMD:D_SOD-123", "MBR0530", "C77336", "back", (-4.5, -10.0), 0, {"1": "GND", "2": "N2"})
part("D2", "Device:D_Schottky", "Diode_SMD:D_SOD-123", "MBR0530", "C77336", "back", (-4.5, -6.0), 0, {"1": "N2", "2": "VGL"})
part("C5", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "1uF 25V", "C15849", "back", (-9.5, -13.5), 0, {"1": "VGH", "2": "GND"})
part("C11", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "1uF 25V", "C15849", "back", (-4.5, -13.5), 0, {"1": "VGL", "2": "GND"})
# the panel's own rails, each a capacitor to ground next to the connector
for ref, net, x in (("C2", "VSH2", -24.0), ("C6", "3V3", -21.0), ("C7", "VDD", -18.0), ("C9", "VSH1", -15.0), ("C10", "VSL", -12.0), ("C12", "VCOM", -9.0)):
    part(ref, "Device:C", "Capacitor_SMD:C_0603_1608Metric", "1uF 25V", "C15849", "back", (x, -17.5), 90, {"1": net, "2": "GND"})

# ---- buttons, front ----
part("SW2", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "A: next", "C318884", "front", (-14.0, -24.0), 0, {"1": "BTN_A", "2": "GND"})
part("SW3", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "B: confirm", "C318884", "front", (14.0, -24.0), 0, {"1": "BTN_B", "2": "GND"})

# ---- power: coin cell -> switch -> Schottky -> VSYS, on the back ----
part("BT1", "Device:Battery_Cell", "Battery:BatteryHolder_MYOUNG_BS-07-A1BJ001_CR2032", "CR2032", "MYOUNG BS-07-A1BJ001 (LCSC number to confirm)", "back", (12.0, -22.0), 0, {"1": "BAT+", "2": "GND"})
part("SW1", "Switch:SW_SPDT", "Button_Switch_SMD:SW_SPDT_Shouhan_MSK12C02", "power", "C431540 MSK-12C02", "back", (-20.0, -30.0), 90, {"1": "BAT+", "2": "BAT_SW"})
part("D4", "Device:D_Schottky", "Diode_SMD:D_SOD-123", "MBR0530", "C77336", "back", (-14.0, -30.0), 0, {"1": "VSYS", "2": "BAT_SW"})

# ---- the pile ports: two JST-SH sockets at the bottom edge, cables out of the end ----
# Pin 1 GND, pin 2 unused, pins 3 and 4 TX and RX, crossed between UP and DN so a straight cable
# joins TX to RX. Presence is the idle-high UART line, so no detect wire is needed.
part("J2", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal", "UP port, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["UP"], PORT_Y), 180, {"1": "GND", "3": "UP_TX", "4": "UP_RX", "MP": "GND"})
part("J3", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal", "DN port, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["DN"], PORT_Y), 180, {"1": "GND", "3": "DN_RX", "4": "DN_TX", "MP": "GND"})

# ---- the magnetic pogo pair: the pin half on the board, the pad half loose in the tray floor ----
# Pins 1 and 5 ground, 2 and 4 the UART, 3 spare. Face to face, pin k meets pin k and the magnets
# attract. The pad half's pigtail: its pin 1 to J3 pin 1 (GND), pin 2 to J3 pin 3 (DN_RX), pin 4 to
# J3 pin 4 (DN_TX); so the lower badge's UP_TX on pin 2 reaches the upper badge's DN_RX.
part("J4", "Connector_Generic:Conn_01x05", "cahoots:Pogo_MG005_1x05", "UP, magnetic pogo pins", "Adafruit 5413 pair, pin half", "front", (0.0, POGO_Y + POGO["d"] / 2 - POGO["pin_to_back"]), 0, {"1": "GND", "2": "UP_TX", "4": "UP_RX", "5": "GND"})
part("P1", None, "cahoots:Pogo_MG005_tray", "DN, magnetic pads, in the tray floor", "Adafruit 5413 pair, pad half; JST-SH 4 pigtail to J3", "front", (0.0, POGO_Y + POGO["d"] / 2 - POGO["pin_to_back"]), 0, {})

# ---- magnet pockets: holes, no copper ----
for i, (x, y) in enumerate(MAGNETS):
    part(f"M{i+1}", None, "cahoots:Magnet_NPTH_D5.1mm", "5x2 mm disc", "mechanical", "front", (x, y), 0, {})

# Panel outline on the front, for the silkscreen and the courtyard: 31.8 wide, 37.32 tall,
# flex leaving the bottom edge 8.5 mm from the panel's left edge (drawing page 6).
PANEL = dict(center=(0.0, 17.0), w=31.8, h=37.32, flex_x=-15.9 + 8.5, flex_w=9.6)

def nets():
    out = {}
    for ref, p in PARTS.items():
        for pad, net in p["pins"].items():
            out.setdefault(net, []).append((ref, pad))
    return out

if __name__ == "__main__":
    for net, pins in sorted(nets().items()):
        print(f"{net:10s} {' '.join(f'{r}.{p}' for r, p in pins)}")
    print(len(PARTS), "parts,", len(nets()), "nets")

# Routes drawn by hand before the autorouter runs, for the nets it keeps leaving out: the three
# panel signals from the Pico's bottom row to the flex connector, and the down-link RX to its
# spring contact. Each step is a pad, a via (layer change), or a point on a layer; consecutive
# points on the same layer are joined by a track. Coordinates as everywhere else in this file.
MANUAL = {
    # From the Pico's bottom row: drop on the back to a level of its own, run left past the
    # connector to a via, climb on the front to a second level, run right to above the pad, drop
    # in. Levels and via positions chosen so no two of the three cross on either layer.
    "EPD_CS":   [("pad", "U1", "22"), ("pt", "B", None, -4.5), ("via", -13.0, -4.5), ("pt", "F", -13.0, -7.8), ("pt", "F", ("J1", "12"), -7.8), ("pad", "J1", "12")],
    "EPD_SCK":  [("pad", "U1", "24"), ("pt", "B", None, -3.5), ("via", -12.0, -3.5), ("pt", "F", -12.0, -7.3), ("pt", "F", ("J1", "13"), -7.3), ("pad", "J1", "13")],
    "EPD_MOSI": [("pad", "U1", "25"), ("pt", "B", None, -2.5), ("via", -11.0, -2.5), ("pt", "F", -11.0, -6.8), ("pt", "F", ("J1", "14"), -6.8), ("pad", "J1", "14")],
}

# ---- the case: a tray the board drops into face up, and a lid that clicks over it ----
# Heights are from the board's top face (z = 0). The board is 1.6 thick. Everything the case must
# clear: on the front the panel (1.0, glued into the lid's window so its glass is flush), the flex
# connector (2.0), the switches (about 5.0 with their stems, which reach up into holes in the lid);
# on the back the Pico 2 (about 4.0 with its USB socket) and the coin cell in its holder (5.4).
CASE = dict(
    board_t=1.6, gap=0.3,             # clearance around the board's edge
    wall=1.6, floor=2.0, lid_t=2.0,   # tray wall, tray floor, lid plate
    below=5.6,                        # cavity under the board, for the cell and the Pico
    above=2.2,                        # cavity over the board: the flex connector is 2.0; the pogo body (4.0) and the sockets (4.3) rise into lid windows
    skirt=6.0, skirt_t=1.2, skirt_gap=0.15,  # the lid's skirt overlaps the tray's wall outside, down past the board
    snap_z=-3.2,                      # the snap windows' bottom, below the board so the short wall above it stays whole
    snap=0.45, snap_w=8.0,            # bump height and width; four of them, two per long side
    button_hole=7.0, port_w=8.0, port_h=4.6,   # cable openings in the end wall over each socket, and lid windows over the socket bodies
    pogo_win=(20.6, 4.7),             # lid window the pin half's body rises into, and the floor window the pad half sits in
    usb=(27.0, 10.0, 12.0, 6.5),      # x of the edge, y centre, width, height of the USB notch in the tray wall
    magnet_d=5.2, magnet_t=2.2,       # pockets in the lid top and the tray floor at MAGNETS
    corner_r=3.0,
)

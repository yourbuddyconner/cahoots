"""V1: the first Cahoots badge you can order. The single source for the schematic and the board.

A 54 x 86 mm two-layer board that a Raspberry Pi Pico 2 solders onto. On the front: the raw
1.54" e-paper (GDEY0154D67) on its 24-pin flex, two tactile switches, the pin half of a
magnetic pogo pair for the card above, two JST-SH pile sockets and a Qwiic socket at the
bottom edge, and a four-turn NFC loop around the panel. On the back: the Pico 2, the coin cell,
a power switch, the panel's boost circuit, a 10-pin expansion socket, the SE051 secure element
with a switch that opens its antenna loop, and four magnet pockets.

Coordinates are millimetres from the board centre, x to the right, y up, as seen from the
front. KiCad's y runs down; make_pcb.py flips it. Front parts are on F.Cu, back parts on B.Cu.

Sources: GDEY0154D67 specification 24.10.22 (pinout page 7, drawing page 6, reference circuit
page 28); Raspberry Pi Pico 2 datasheet (pinout figure 2, SMT footprint figure 5); NXP SE051
data sheet rev 2.0 (pinning table 3, 56 pF resonance capacitance section 4.2); SE051 Plug&Trust
Click schematic (the antenna switch in the LA line, 27 pF across LA/LB).
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
PORTS = {"UP": 13.0, "DN": -13.0, "QW": 0.0}         # x of each JST socket; QW is the Qwiic socket, between the pile ports

# The NFC loop, on the front around the whole board, just inside the magnet pockets: a rectangular
# spiral of copper, its two ends at the top-right corner where the SE051 sits on the back. Around the
# perimeter the turns fence nothing in: the front stays one open field for routing, and only the pile
# sockets at the bottom edge lie outside the loop, reached by vias between them. (A loop around the
# panel alone was tried first: its bottom edge ran along the Pico's lower pad row and left every net
# from the top half one way out, between those pads on the back; the router never finished.)
# The chip resonates with 56 pF (data sheet 4.2), so the loop wants about 2.5 uH at 13.56 MHz; this
# loop is drawn under that and C15 (fitted) plus C16 (not fitted, for trimming) bring it down to
# frequency, as the Click board does. Copper inside the loop (the Pico, the cell, the back pour)
# lowers the inductance in practice, which the same capacitors absorb; expect to fit C16. No ground
# pour under or beside the turns on either layer, and none on the front inside the loop.
ANTENNA = dict(x0=-26.0, y0=-35.5, x1=26.0, y1=35.5, turns=3, width=0.4, gap=0.4, keepout=1.2)   # 0.75 mm clear of the magnet pockets, 0.6 of the edge

def antenna_path():
    """The spiral as design-coordinate points, outer end first, inner end last. Starts at the top-right
    corner and runs left along the top, so both ends land at the top right, by the chip."""
    a = ANTENNA; p = a["width"] + a["gap"]
    x0, y0, x1, y1 = a["x0"], a["y0"], a["x1"], a["y1"]
    pts = [(x1, y1)]
    for k in range(a["turns"]):
        pts += [(x0 + k * p, y1 - k * p), (x0 + k * p, y0 + k * p), (x1 - k * p, y0 + k * p), (x1 - k * p, y1 - (k + 1) * p)]
    return pts

def antenna_inductance_uH():
    """Mohan's current-sheet estimate for a square spiral (K1 2.34, K2 2.75), on the loop's mean side.
    Good to about ten percent in free space; copper inside the loop lowers it further."""
    a = ANTENNA; n = a["turns"]; p = a["width"] + a["gap"]
    d_out = ((a["x1"] - a["x0"]) + (a["y1"] - a["y0"])) / 2
    d_in = d_out - 2 * (n * a["width"] + (n - 1) * a["gap"])
    d_avg = (d_out + d_in) / 2 * 1e-3; rho = (d_out - d_in) / (d_out + d_in)
    return 2.34 * 1.2566e-6 * n * n * d_avg / (1 + 2.75 * rho) * 1e6

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
    "I2C_SDA": "GP8", "I2C_SCL": "GP9", # I2C0, the Qwiic socket J5 and the SE051.
    "SE_ENA": "GP2",                    # SE051 ENA: low puts the chip in deep power-down; a 100k pull-up keeps it on until the firmware says otherwise.
}
# The expansion port J6, pin 1 to 10. Nothing else on the board uses these GPIO.
HEADER = ["3V3", "GND", "GP6", "GP7", "GP10", "GP11", "GP12", "GP13", "GP22", "GP26"]

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
for gp in HEADER:
    if gp.startswith("GP"):
        pico_nets[str(pico_pin(gp))] = gp
part("U1", "cahoots:Pico2", "Module:RaspberryPi_Pico_SMD", "Raspberry Pi Pico 2 (no headers, no W)",
     "not fitted: solder by hand", "back", (0.6, 10.0), 90, pico_nets)

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
part("L1", "Device:L", "Inductor_SMD:L_Changjiang_FNR4030S", "47uH 500mA", "C167888 FNR4030S470MT", "back", (-21.0, -6.0), 0, {"1": "3V3", "2": "SW"})
part("C4", "Device:C", "Capacitor_SMD:C_0805_2012Metric", "4.7uF 25V", "C1779", "back", (-21.0, -11.0), 0, {"1": "3V3", "2": "GND"})
part("Q1", "Transistor_FET:AO3400A", "Package_TO_SOT_SMD:SOT-23", "AO3400A", "C20917 (Si1308EDL in the reference; 30 V, 5.7 A)", "back", (-15.0, -6.0), 0, {"1": "GDR", "2": "RESE", "3": "SW"})
part("R1", "Device:R", "Resistor_SMD:R_0603_1608Metric", "1M", "C22935", "back", (-16.5, -10.0), 0, {"1": "GDR", "2": "GND"})
part("R2", "Device:R", "Resistor_SMD:R_0603_1608Metric", "2.2R", "C22939 0603WAF220KT5E, 2.2 ohm", "back", (-13.0, -10.0), 0, {"1": "RESE", "2": "GND"})
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
part("SW2", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "A: next", "C318884 TS-1187A-B-A-B", "front", (-14.0, -24.0), 0, {"1": "BTN_A", "2": "GND"})
part("SW3", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "B: confirm", "C318884 TS-1187A-B-A-B", "front", (14.0, -24.0), 0, {"1": "BTN_B", "2": "GND"})

# ---- power: coin cell -> switch -> Schottky -> VSYS, on the back ----
part("BT1", "Device:Battery_Cell", "Battery:BatteryHolder_MYOUNG_BS-07-A1BJ001_CR2032", "CR2032", "C2979167 MYOUNG BS-07-A1BJ001", "back", (12.0, -22.0), 0, {"1": "BAT+", "2": "GND"})
part("SW1", "Switch:SW_SPDT", "Button_Switch_SMD:SW_SPDT_Shouhan_MSK12C02", "power", "C431540 MSK-12C02", "back", (-20.0, -30.0), 90, {"1": "BAT+", "2": "BAT_SW"})
part("D4", "Device:D_Schottky", "Diode_SMD:D_SOD-123", "MBR0530", "C77336", "back", (-13.5, -37.5), 0, {"1": "VSYS", "2": "BAT_SW"})

# ---- the pile ports: two JST-SH sockets at the bottom edge, cables out of the end ----
# KiCad's side-entry SH footprint opens on the side away from its solder tails, +y in its own frame,
# so rotation 0 here puts the opening at the board's edge. (They were drawn at 180 until 2026-10-28,
# opening inward; Conner caught it in the 3D view.)
# Pin 1 GND, pin 2 unused, pins 3 and 4 TX and RX, crossed between UP and DN so a straight cable
# joins TX to RX. Presence is the idle-high UART line, so no detect wire is needed.
part("J2", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal", "UP port, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["UP"], PORT_Y), 0, {"1": "GND", "3": "UP_TX", "4": "UP_RX", "MP": "GND"})
part("J3", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal", "DN port, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["DN"], PORT_Y), 0, {"1": "GND", "3": "DN_RX", "4": "DN_TX", "MP": "GND"})

# ---- the Qwiic / STEMMA QT socket: the maker world's I2C connector, so any Qwiic board plugs in ----
part("J5", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal", "Qwiic / STEMMA QT, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["QW"], PORT_Y), 0, {"1": "GND", "2": "3V3", "3": "I2C_SDA", "4": "I2C_SCL", "MP": "GND"})

# ---- the expansion port: a 10-pin JST-SH socket on the back at the bottom edge, under the Qwiic socket,
# the same low family as the three front sockets (about 3 mm tall), its cable out of the bottom end.
# Was a 2x5 header until 2026-10-28; the pin order is unchanged.
_j6 = {str(i + 1): net for i, net in enumerate(HEADER)}; _j6["MP"] = "GND"
part("J6", "Connector_Generic:Conn_01x10", "Connector_JST:JST_SH_SM10B-SRSS-TB_1x10-1MP_P1.00mm_Horizontal", "expansion, JST-SH 10", "C160409 JST SM10B-SRSS-TB", "back", (PORTS["QW"], PORT_Y), 0, _j6)

# ---- the magnetic pogo pair: the pin half on the board, the pad half loose in the tray floor ----
# Pins 1 and 5 ground, 2 and 4 the UART, 3 spare. Face to face, pin k meets pin k and the magnets
# attract. The pad half's pigtail: its pin 1 to J3 pin 1 (GND), pin 2 to J3 pin 3 (DN_RX), pin 4 to
# J3 pin 4 (DN_TX); so the lower badge's UP_TX on pin 2 reaches the upper badge's DN_RX.
part("J4", "Connector_Generic:Conn_01x05", "cahoots:Pogo_MG005_1x05", "UP, magnetic pogo pins", "Adafruit 5413 pair, pin half", "front", (0.0, POGO_Y + POGO["d"] / 2 - POGO["pin_to_back"]), 0, {"1": "GND", "2": "UP_TX", "4": "UP_RX", "5": "GND"})
part("P1", None, "cahoots:Pogo_MG005_tray", "DN, magnetic pads, in the tray floor", "Adafruit 5413 pair, pad half; JST-SH 4 pigtail to J3", "front", (0.0, POGO_Y + POGO["d"] / 2 - POGO["pin_to_back"]), 0, {})

# ---- the secure element: NXP SE051 on I2C0, on the back at the top right, above the Pico (which ends at y 21.6; the chip sits high enough that its escape stubs clear the Pico's pad row) and inside the loop ----
# HX2QFN20 pins (data sheet table 3): 1 LB, 9 SDA, 10 SCL, 11 ENA, 12 VIN, 14 RST_N, 15 VOUT, 17 LA, 18 VCC,
# 19 VSS; 2-8, 13, 16, 20 unused. The centre pad is not connected inside the chip (data sheet 7.1.1) and is
# left unconnected here: tying it to ground would need a via in the pad, which wicks solder. VOUT is tied to VCC on
# the board, as the data sheet asks when ENA is used, so SE_VCC is the chip's own switched rail. RST_N is the
# ISO 7816 reset, tied high. The I2C pull-ups are 3.3k, as on the Click board; a Qwiic board adds its own.
part("U2", "cahoots:SE051", "Package_DFN_QFN:QFN-20-1EP_3x3mm_P0.4mm_EP1.65x1.65mm", "SE051C2", "C5202100 NXP SE051C2HQ1/Z01XDZ", "back", (11.5, 26.5), 0,
     {"1": "LB", "9": "I2C_SDA", "10": "I2C_SCL", "11": "SE_ENA", "12": "3V3", "14": "SE_VCC", "15": "SE_VCC", "17": "LA", "18": "SE_VCC", "19": "GND"})
part("C13", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "100nF 50V", "C14663", "back", (7.0, 33.0), 0, {"1": "3V3", "2": "GND"})
part("C14", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "100nF 50V", "C14663", "back", (7.0, 30.0), 0, {"1": "SE_VCC", "2": "GND"})
part("R5", "Device:R", "Resistor_SMD:R_0603_1608Metric", "100k", "C25803", "back", (20.5, 32.0), 0, {"1": "SE_ENA", "2": "3V3"})
part("R3", "Device:R", "Resistor_SMD:R_0603_1608Metric", "3.3k", "C22978", "back", (-14.0, 25.0), 0, {"1": "I2C_SDA", "2": "3V3"})
part("R4", "Device:R", "Resistor_SMD:R_0603_1608Metric", "3.3k", "C22978", "back", (-14.0, 28.0), 0, {"1": "I2C_SCL", "2": "3V3"})
# The antenna: tuning across LA/LB at the chip, then the switch in the LA line, then the loop. With the
# switch open the loop is not a closed conductor and the chip sees only C15.
part("C15", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "39pF C0G", "C107049 CC0603JRNPO9BN390, 39 pF", "back", (11.5, 33.0), 0, {"1": "LA", "2": "LB"})
part("C16", "Device:C", "Capacitor_SMD:C_0603_1608Metric", "27pF C0G", "C107045 not fitted: antenna trim", "back", (15.5, 33.0), 0, {"1": "LA", "2": "LB"})
part("SW4", "Switch:SW_SPDT", "Button_Switch_SMD:SW_SPDT_Shouhan_MSK12C02", "antenna", "C431540 MSK-12C02", "back", (21.0, 25.5), 0, {"1": "LA", "2": "ANT"})
_ap = antenna_path()
part("ANT1", "Device:Antenna_Loop", "cahoots:NFC_loop", "NFC loop, 4 turns", "board copper", "front", _ap[0], 0, {"1": "ANT", "2": "LB"})

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
    pins_in_use = list(GPIO.values()) + [n for n in HEADER if n.startswith("GP")]
    assert len(pins_in_use) == len(set(pins_in_use)), "a Pico pin is used twice"
    assert GPIO["I2C_SDA"] == "GP8" and GPIO["I2C_SCL"] == "GP9"
    assert PARTS["J5"]["pins"] == {"1": "GND", "2": "3V3", "3": "I2C_SDA", "4": "I2C_SCL", "MP": "GND"}
    assert [PARTS["J6"]["pins"][str(i)] for i in range(1, 11)] == HEADER and PARTS["J6"]["pins"]["MP"] == "GND"
    assert "3" not in PARTS["J4"]["pins"], "pogo pin 3 stays spare"
    L = antenna_inductance_uH() * 1e-6
    f = lambda c: 1e-6 / (6.2832 * (L * c) ** 0.5)
    print(f"antenna: {ANTENNA['turns']} turns, about {L*1e6:.2f} uH in free space; 56 pF alone resonates at {f(56e-12):.1f} MHz, with C15 (39 pF) at {f(95e-12):.1f} MHz, with C16 (27 pF) too at {f(122e-12):.1f} MHz")
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
# The pile sockets sit outside the NFC loop, at the bottom edge, and their pads are on the front: each
# signal gets a via of its own below its pad, beside the socket's mounting pads, so the router only
# has to reach the via on the back. The Qwiic socket gets none: the expansion socket on the back
# escapes straight up through the same spot, so its vias go in the gaps beside the sockets instead.
# A key "NET@REF" routes one pad of a net that has several such pads.
PORT_VIA_Y = -39.4
for _ref, _pins in (("J2", {"3": "UP_TX", "4": "UP_RX", "1": "GND"}), ("J3", {"3": "DN_RX", "4": "DN_TX", "1": "GND"})):
    for _num, _net in _pins.items():
        MANUAL[f"{_net}@{_ref}"] = [("pad", _ref, _num), ("via", None, PORT_VIA_Y)]
# The antenna line from the chip to its switch: the router keeps missing it, so it is drawn. It starts
# at the end of pin 17's escape stub (the longest on the chip's lower side, 1.8 mm below the pad at
# U2's y - 1.45) and runs under the other stubs to SW4's pin 1.
_u2x, _u2y = PARTS["U2"]["at"]
_la0 = (_u2x + 0.4, _u2y - 1.45 - 1.8)
MANUAL["LA"] = [("pt", "B", _la0[0], _la0[1]), ("pt", "B", _la0[0], _u2y - 3.9), ("pt", "B", PARTS["SW4"]["at"][0] - 2.25, _u2y - 3.9), ("pad", "SW4", "1")]

# The SE051's pads are on a 0.4 mm pitch, 0.2 mm wide: no track of the board's 0.25 mm with 0.2 mm
# clearance can leave one. Each used pad gets a stub at the fab's 0.127 mm minimum, straight out from
# the chip, locked, and the router picks up at the stub's end. Along each side the stubs form a
# staircase, each 0.5 mm longer than its neighbour, so every end is reachable from the short side with
# the router's wider track clearing the ends of the shorter stubs.
STUBS = {"U2": ("1", "9", "10", "11", "12", "14", "15", "17", "18", "19")}
STUB_W, STUB_LEN0, STUB_STEP = 0.127, 0.8, 0.5

# ---- the case: a tray the board drops into face up, and a lid that clicks over it ----
# Heights are from the board's top face (z = 0). The board is 1.6 thick. Everything the case must
# clear: on the front the panel (1.0, taped to the board, so the board comes out of the case as one
# unit with its screen; decided 2026-10-28, it was glued into the lid before), the flex connector
# (2.0), the switches (about 5.0 with their stems, which reach up into holes in the lid); on the
# back the Pico 2 (about 4.0 with its USB socket) and the coin cell in its holder (5.4).
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
    panel_tape=0.15, panel_lip=1.5,   # the panel sits on the board on thin tape; the lid's window leaves a lip this wide over its edge
    panel_press=0.15,                 # the frame under the lip stops this far above the glass, so it holds the panel without loading it
    switch_win=(9.0, 5.0),            # openings in the tray floor under the two slide switches, long side along the switch
)

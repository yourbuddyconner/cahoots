"""Build v1.kicad_sch from design.py. Plain python3.

Two drawing styles on one sheet. The Pico and the flex connector are blocks with a net label
on every used pin. The boost circuit and the power path are drawn with wires, laid out to
read like the panel specification's reference circuit, with labels only where a net leaves
the drawing. The net names are the same table the board is built from, and check.py proves
the schematic KiCad reads back matches it."""
import math
import os
import re
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import design as D

SYM_DIR = "/Applications/KiCad.app/Contents/SharedSupport/symbols"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "v1.kicad_sch")
ROOT = "8e3d2c5a-0000-4000-8000-00000000c0de"
G = 2.54  # the grid

# ---------- s-expressions ----------
def parse(text):
    tok = re.findall(r'"(?:\\.|[^"\\])*"|\(|\)|[^\s()"]+', text)
    pos = 0
    def rd():
        nonlocal pos
        t = tok[pos]; pos += 1
        if t == "(":
            out = []
            while tok[pos] != ")":
                out.append(rd())
            pos += 1
            return out
        return t
    return rd()

def dump(node, ind=0):
    if isinstance(node, str):
        return node
    if all(isinstance(x, str) for x in node) and len(node) <= 8:
        return "(" + " ".join(node) + ")"
    parts = ["(" + dump(node[0])]
    for x in node[1:]:
        if isinstance(x, str):
            parts[-1] += " " + x
        else:
            parts.append("\n" + "  " * (ind + 1) + dump(x, ind + 1))
    return "".join(parts) + ")"

def q(s):
    return '"' + str(s).replace('"', '\\"') + '"'

def find(node, key):
    return [x for x in node if isinstance(x, list) and x and x[0] == key]

# ---------- symbols ----------
_libcache = {}
def lib_symbol(lib, name):
    if lib == "cahoots":
        return pico_symbol()
    if lib not in _libcache:
        _libcache[lib] = parse(open(os.path.join(SYM_DIR, lib + ".kicad_sym")).read())
    syms = {s[1].strip('"'): s for s in find(_libcache[lib], "symbol")}
    s = syms[name]
    ext = find(s, "extends")
    if ext:
        parent = syms[ext[0][1].strip('"')]
        body = [x for x in parent if not (isinstance(x, list) and x and x[0] == "property")]
        props = {p[1]: p for p in find(parent, "property")}
        for p in find(s, "property"):
            props[p[1]] = p
        s = rename_units(body[:2] + list(props.values()) + body[2:], ext[0][1].strip('"'), name)
    s = list(s)
    s[1] = q(f"{lib}:{name}")
    return s

def rename_units(node, old, new):
    if isinstance(node, str):
        return node
    out = [rename_units(x, old, new) for x in node]
    if out and out[0] == "symbol" and len(out) > 1 and isinstance(out[1], str) and out[1].startswith(f'"{old}_'):
        out[1] = '"' + new + out[1][1 + len(old):]
    return out

def pico_symbol():
    pins = []
    for n in range(1, 41):
        name = D.PICO_PINS[n]
        kind = "power_out" if name == "3V3" else "power_in" if name in ("GND", "AGND", "VSYS", "VBUS") else "bidirectional"
        if n <= 20:
            x, y, ang = -17.78, 24.13 - (n - 1) * G, 0
        else:
            x, y, ang = 17.78, 24.13 - (40 - n) * G, 180
        pins.append(parse(f'(pin {kind} line (at {x} {y} {ang}) (length 5.08) (name "{name}" (effects (font (size 1.27 1.27)))) (number "{n}" (effects (font (size 1.27 1.27)))))'))
    body = parse('(symbol "Pico2_1_1" (rectangle (start -12.7 27.94) (end 12.7 -27.94) (stroke (width 0.254) (type default)) (fill (type background))))')
    body += pins
    sym = parse('(symbol "cahoots:Pico2" (pin_names (offset 1.016)) (exclude_from_sim no) (in_bom yes) (on_board yes)'
                ' (property "Reference" "U" (at 0 30 0) (effects (font (size 1.27 1.27))))'
                ' (property "Value" "Pico2" (at 0 -30 0) (effects (font (size 1.27 1.27))))'
                ' (property "Footprint" "" (at 0 0 0) (effects (font (size 1.27 1.27)) hide))'
                ' (property "Datasheet" "https://datasheets.raspberrypi.com/pico/pico-2-datasheet.pdf" (at 0 0 0) (effects (font (size 1.27 1.27)) hide)))')
    sym.append(body)
    return sym

def pin_positions(sym):
    out = {}
    for unit in find(sym, "symbol"):
        for pin in find(unit, "pin"):
            at = find(pin, "at")[0]
            num = find(pin, "number")[0][1].strip('"')
            out[num] = (float(at[1]), float(at[2]), float(at[3]) if len(at) > 3 else 0.0)
    return out

# ---------- drawing primitives ----------
items = []
lib_symbols = {}

def uid():
    return str(uuid.uuid4())

def wire(a, b):
    items.append(f'(wire (pts (xy {a[0]:.2f} {a[1]:.2f}) (xy {b[0]:.2f} {b[1]:.2f})) (stroke (width 0) (type default)) (uuid "{uid()}"))')

def route(*pts):
    for a, b in zip(pts, pts[1:]):
        wire(a, b)

def junction(p):
    items.append(f'(junction (at {p[0]:.2f} {p[1]:.2f}) (diameter 0) (color 0 0 0 0) (uuid "{uid()}"))')

def label(net, p, angle):
    """A global label at p. angle 0: text runs right of p; 180: left; 270: up; 90: down."""
    just = "left" if angle in (0, 270) else "right"
    items.append(f'(global_label {q(net)} (shape input) (at {p[0]:.2f} {p[1]:.2f} {angle}) (fields_autoplaced yes) (effects (font (size 1.27 1.27)) (justify {just})) (uuid "{uid()}") '
                 f'(property "Intersheetrefs" "${{INTERSHEET_REFS}}" (at {p[0]:.2f} {p[1]:.2f} 0) (effects (font (size 1.27 1.27)) hide)))')

def netname(net, p):
    items.append(f'(label {q(net)} (at {p[0]:.2f} {p[1]:.2f} 0) (fields_autoplaced yes) (effects (font (size 1.0 1.0)) (justify left bottom)) (uuid "{uid()}"))')

def text(t, x, y, size=1.5):
    items.append(f'(text {q(t)} (exclude_from_sim no) (at {x:.2f} {y:.2f} 0) (effects (font (size {size} {size})) (justify left bottom)) (uuid "{uid()}"))')

def no_connect(p):
    items.append(f'(no_connect (at {p[0]:.2f} {p[1]:.2f}) (uuid "{uid()}"))')

def place(ref, x, y, rot=0, mirror=None, ref_at=None, val_at=None):
    """Put a part at (x, y) with rotation rot (degrees, counter-clockwise in symbol space).
    Returns pin number -> sheet point and its free direction in degrees (0 right, 90 up, ...)."""
    p = D.PARTS[ref]
    lib, name = p["symbol"].split(":")
    sym = lib_symbol(lib, name)
    lib_symbols[p["symbol"]] = sym
    pins = pin_positions(sym)
    rx, ry = (ref_at or (3.0, -2.5))
    vx, vy = (val_at or (3.0, 0.5))
    mir = f" (mirror {mirror})" if mirror else ""
    props = [
        f'(property "Reference" {q(ref)} (at {x + rx:.2f} {y + ry:.2f} { {0: 0, 90: 270, 180: 0, 270: 90}[rot] }) (effects (font (size 1.27 1.27)) (justify left)))',
        f'(property "Value" {q(p["value"])} (at {x + vx:.2f} {y + vy:.2f} { {0: 0, 90: 270, 180: 0, 270: 90}[rot] }) (effects (font (size 1.27 1.27)) (justify left)))',
        f'(property "Footprint" {q(p["footprint"])} (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide))',
        f'(property "Datasheet" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide))',
        f'(property "LCSC" {q(p["lcsc"])} (at {x:.2f} {y:.2f} 0) (effects (font (size 1.0 1.0)) hide))',
    ]
    pin_uuids = " ".join(f'(pin "{n}" (uuid "{uid()}"))' for n in pins)
    items.append(f'(symbol (lib_id {q(p["symbol"])}) (at {x:.2f} {y:.2f} {rot}){mir} (unit 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (dnp no) (uuid "{uid()}") '
                 + " ".join(props) + f' {pin_uuids} (instances (project "v1" (path "/{ROOT}" (reference {q(ref)}) (unit 1)))))')
    out = {}
    a = math.radians(rot)
    for num, (px, py, ang) in pins.items():
        sx, sy = px, py
        if mirror == "x":
            sy = -sy
        if mirror == "y":
            sx = -sx
        rx_, ry_ = sx * math.cos(a) - sy * math.sin(a), sx * math.sin(a) + sy * math.cos(a)
        free = (ang + 180 + rot) % 360
        if mirror == "y":
            free = (180 - free) % 360
        if mirror == "x":
            free = (-free) % 360
        out[num] = ((x + rx_, y - ry_), round(free) % 360)
    return out

def stub(p, free, net, length=G):
    """A short wire out of a pin in its free direction, ending in a label."""
    dx, dy = {0: (1, 0), 90: (0, -1), 180: (-1, 0), 270: (0, 1)}[free]
    e = (p[0] + length * dx, p[1] + length * dy)
    wire(p, e)
    label(net, e, {0: 0, 180: 180, 90: 270, 270: 90}[free])
    return e

def labelled_block(ref, x, y, ref_at, val_at):
    pins = place(ref, x, y, ref_at=ref_at, val_at=val_at)
    nets = D.PARTS[ref]["pins"]
    for num, (p, free) in pins.items():
        if num in nets:
            stub(p, free, nets[num])
        else:
            no_connect(p)

def power(kind, p, up=True):
    """A power symbol (GND below a point, +3V3 above) wired to p."""
    name = "GND" if kind == "GND" else "+3V3"
    sym = lib_symbol("power", name); lib_symbols["power:" + name] = sym
    pins = pin_positions(sym)
    (px, py, ang) = pins["1"]
    # the symbol's pin sits at (0,0); GND hangs below its point, +3V3 stands above
    y = p[1] + (G if kind == "GND" else -G)
    x = p[0]
    ref = f"#PWR_{len(items)}"
    items.append(f'(symbol (lib_id {q("power:" + name)}) (at {x:.2f} {y:.2f} 0) (unit 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (dnp no) (uuid "{uid()}") '
                 f'(property "Reference" {q(ref)} (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) (property "Value" {q(name)} (at {x:.2f} {y + (3 if kind == "GND" else -3):.2f} 0) (effects (font (size 1.27 1.27)))) '
                 f'(property "Footprint" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) (property "Datasheet" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) '
                 f'(pin "1" (uuid "{uid()}")) (instances (project "v1" (path "/{ROOT}" (reference {q(ref)}) (unit 1)))))')
    wire(p, (x, y))

def pwr_flag(p):
    sym = lib_symbol("power", "PWR_FLAG"); lib_symbols["power:PWR_FLAG"] = sym
    x, y = p[0], p[1] - G
    ref = f"#FLG_{len(items)}"
    items.append(f'(symbol (lib_id "power:PWR_FLAG") (at {x:.2f} {y:.2f} 0) (unit 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (dnp no) (uuid "{uid()}") '
                 f'(property "Reference" {q(ref)} (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) (property "Value" "PWR_FLAG" (at {x:.2f} {y - 4:.2f} 0) (effects (font (size 1.0 1.0)))) '
                 f'(property "Footprint" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) (property "Datasheet" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide)) '
                 f'(pin "1" (uuid "{uid()}")) (instances (project "v1" (path "/{ROOT}" (reference {q(ref)}) (unit 1)))))')
    wire(p, (x, y))

# ---------- the sheet ----------
# Pico: pins out left and right, labels on all used pins.
text("Pico 2. Module pin numbers outside, GPx inside.", 25.4, 45, 1.3)
labelled_block("U1", 55.88, 91.44, ref_at=(-6, -32), val_at=(-12, 32))

# Flex connector: a 24-pin block, labels for the signals, power on its rails.
text("Panel flex, GDEY0154D67 pin order (spec p. 7).", 101.6, 45, 1.3)
labelled_block("J1", 121.92, 86.36, ref_at=(4, -33), val_at=(4, 33))

# Boost circuit, drawn as in the specification's reference circuit (p. 28), left to right:
# 3V3 -> L1 -> SW node -> D3 -> VGH, with Q1 switching SW to ground through the sense resistor,
# and C3/D1/D2 pumping VGL negative from the same SW node.
BX, BY = 175.26, 73.66
text("Boost, as the panel spec's reference circuit (p. 28).", BX - 12.7, BY - 35.56, 1.3)
l1 = place("L1", BX, BY, rot=90, ref_at=(-3.5, -4), val_at=(-3.5, 3.5))          # horizontal: pin 1 left, pin 2 right
sw = l1["2"][0]
in3 = l1["1"][0]
label("3V3", (in3[0] - G * 4, in3[1]), 180); wire((in3[0] - G * 4, in3[1]), in3)
c4 = place("C4", in3[0] - G * 4, BY + 10.16, ref_at=(2, -1.5), val_at=(2, 1.5))        # input cap to ground
wire((in3[0] - G * 4, in3[1]), c4["1"][0]); junction((in3[0] - G * 4, in3[1])); power("GND", c4["2"][0])
d3 = place("D3", sw[0] + 17.78, BY, rot=180, ref_at=(-3, -3), val_at=(-3, 3.5))      # anode at SW (right pin 2), cathode to VGH (left pin 1)
wire(sw, d3["2"][0]); junction(sw)
vgh = d3["1"][0]
route(vgh, (vgh[0] + G * 2, vgh[1])); label("VGH", (vgh[0] + G * 2, vgh[1]), 0)
c5 = place("C5", vgh[0] + G, BY + 10.16, ref_at=(2, -1.5), val_at=(2, 1.5))
wire((vgh[0] + G, vgh[1]), c5["1"][0]); junction((vgh[0] + G, vgh[1])); power("GND", c5["2"][0])
q1 = place("Q1", sw[0] + 2.54, BY + 17.78, ref_at=(5, -2), val_at=(5, 1.5))        # G left, D up, S down
d = q1["3"][0]
route(sw, (sw[0], d[1] - G), (d[0], d[1] - G), d); netname("SW", (sw[0], d[1] - G))
g = q1["1"][0]
route(g, (g[0] - G * 2, g[1])); label("GDR", (g[0] - G * 2, g[1]), 180)
r1 = place("R1", g[0] - G, g[1] + 10.16, ref_at=(2, -1.5), val_at=(2, 1.5))
wire((g[0] - G, g[1]), r1["1"][0]); junction((g[0] - G, g[1])); power("GND", r1["2"][0])
s = q1["2"][0]
r2 = place("R2", s[0], s[1] + 10.16, ref_at=(2, -1.5), val_at=(2, 1.5))
wire(s, r2["1"][0]); power("GND", r2["2"][0])
route((s[0], s[1] + G), (s[0] + G * 3, s[1] + G)); label("RESE", (s[0] + G * 3, s[1] + G), 0); junction((s[0], s[1] + G))
# negative pump off the same switch node
c3 = place("C3", sw[0] + 7.62, BY - 22.86, rot=90, ref_at=(-3, -4), val_at=(-3, 3.5))   # horizontal, pin 1 left at SW side
route(sw, (sw[0], c3["1"][0][1]), c3["1"][0])
n2 = c3["2"][0]
d1 = place("D1", n2[0] + 5.08, n2[1] + 12.7, rot=270, ref_at=(3.5, -1.5), val_at=(3.5, 2))  # cathode (pin 1) down to GND, anode (pin 2) up at N2
route(n2, (d1["2"][0][0], n2[1]), d1["2"][0]); junction((d1["2"][0][0], n2[1])); power("GND", d1["1"][0])
d2 = place("D2", n2[0] + 20.32, n2[1], rot=0, ref_at=(-3, -3), val_at=(-3, 3.5))          # cathode (pin 1) left at N2, anode (pin 2) right at VGL
route((d1["2"][0][0], n2[1]), d2["1"][0])
vgl = d2["2"][0]
route(vgl, (vgl[0] + G * 2, vgl[1])); label("VGL", (vgl[0] + G * 2, vgl[1]), 0)
c11 = place("C11", vgl[0] + G, vgl[1] + 10.16, ref_at=(2, -1.5), val_at=(2, 1.5))
wire((vgl[0] + G, vgl[1]), c11["1"][0]); junction((vgl[0] + G, vgl[1])); power("GND", c11["2"][0])
netname("N2", (d1["2"][0][0], n2[1]))

# The panel's other rails: one capacitor each, in a row.
RX0, RY = 170.18, 139.7
text("Rail capacitors, one per driven pin of the panel.", RX0 - 12.7, RY - 12.7, 1.3)
for i, (ref, net) in enumerate([("C2", "VSH2"), ("C6", "3V3"), ("C7", "VDD"), ("C9", "VSH1"), ("C10", "VSL"), ("C12", "VCOM")]):
    x = RX0 + i * 20.32
    c = place(ref, x, RY, ref_at=(2, -1.5), val_at=(2, 1.5))
    top = c["1"][0]
    route(top, (top[0], top[1] - G)); label(net, (top[0], top[1] - G), 270)
    power("GND", c["2"][0])

# Power path: cell -> switch -> Schottky -> VSYS.
PX0, PY = 25.4, 165.1
text("Power: cell, switch, Schottky into VSYS.", PX0, PY - 15.24, 1.3)
bt = place("BT1", PX0 + 10.16, PY + 2.54, ref_at=(3, -3), val_at=(3, 0.5))
power("GND", bt["2"][0])
sw1 = place("SW1", PX0 + 30.48, PY - 2.54, mirror="y", ref_at=(-3, -5), val_at=(-3, 5))
route(bt["1"][0], (bt["1"][0][0], sw1["1"][0][1]), sw1["1"][0]); netname("BAT+", (bt["1"][0][0], sw1["1"][0][1]))
no_connect(sw1["3"][0])
d4 = place("D4", PX0 + 50.8, sw1["2"][0][1], rot=180, ref_at=(-3, -3), val_at=(-3, 3.5))   # anode (pin 2) at the switch, cathode (pin 1) to VSYS
route(sw1["2"][0], d4["2"][0]); netname("BAT_SW", (sw1["2"][0][0] + G * 2, sw1["2"][0][1]))
route(d4["1"][0], (d4["1"][0][0] + G * 2, d4["1"][0][1])); label("VSYS", (d4["1"][0][0] + G * 2, d4["1"][0][1]), 0)
pwr_flag(bt["1"][0]); pwr_flag(d4["1"][0])
gnd_flag = (PX0 + 68.58, PY + 7.62); pwr_flag((gnd_flag[0], gnd_flag[1])); power("GND", gnd_flag)

# Buttons: to ground, labels to the Pico.
QX, QY = 121.92, 165.1
text("Buttons to ground. A next, B confirm.", QX - 10.16, QY - 15.24, 1.3)
for i, (ref, net) in enumerate([("SW2", "BTN_A"), ("SW3", "BTN_B")]):
    s_ = place(ref, QX + i * 25.4, QY, ref_at=(-6, -4), val_at=(-6, 4))
    a, b = s_["1"][0], s_["2"][0]
    route(a, (a[0] - G, a[1])); label(net, (a[0] - G, a[1]), 180)
    power("GND", b)

# Pile ports: two JST-SH sockets, UP and DN, TX and RX crossed between them.
CX, CY = 190.5, 165.1
text("Pile links. J4 and J5: the magnetic pogo pair, pads at the top end, pins at the bottom; pins 1 and 5 GND, 2 and 4 the UART, mirrored so pin 2 meets pin 4.", CX - 10.16, CY - 17.78, 1.3)
text("J2 and J3: JST-SH sockets for a cable instead. Pin 1 GND, pins 3 and 4 TX and RX, crossed between UP and DN.", CX - 10.16, CY - 13.97, 1.3)
for i, ref in enumerate(("J4", "J5", "J2", "J3")):
    labelled_block(ref, CX + i * 40.64, CY + 5.08, ref_at=(4, -9), val_at=(4, 9))

text("Cahoots badge V1: a board a Raspberry Pi Pico 2 solders onto. 54 x 86 mm, two layers. Labels with the same name are one net.", 25.4, 228.6, 1.5)
text("design.py is the source; make_sch.py and make_pcb.py draw it; check.py confirms the schematic, the table and the board agree.", 25.4, 233, 1.5)

lib_block = "(lib_symbols\n" + "\n".join(dump(s, 1) for s in lib_symbols.values()) + "\n)"
sheet = f'''(kicad_sch (version 20231120) (generator "cahoots_make_sch") (generator_version "2")
  (uuid "{ROOT}")
  (paper "A3")
  (title_block (title "Cahoots badge V1") (date "2026-10-25") (rev "A") (company "cahoots.network"))
  {lib_block}
  {chr(10).join("  " + i for i in items if i)}
  (sheet_instances (path "/" (page "1")))
)
'''
open(OUT, "w").write(sheet)
print("wrote", OUT)

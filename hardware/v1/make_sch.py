"""Build v1.kicad_sch from design.py: every part placed with a short wire and a global label on
each connected pin, so the schematic and the board come from the same table. Plain python3.
KiCad reads it, runs ERC on it, and exports the same netlist the board was built from
(check.py compares the two)."""
import os
import re
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import design as D

SYM_DIR = "/Applications/KiCad.app/Contents/SharedSupport/symbols"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "v1.kicad_sch")
ROOT = "8e3d2c5a-0000-4000-8000-00000000c0de"

# ---------- a small s-expression parser/printer ----------
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
    return '"' + s.replace('"', '\\"') + '"'

def find(node, key):
    return [x for x in node if isinstance(x, list) and x and x[0] == key]

# ---------- symbol library access ----------
_libcache = {}
def lib_symbol(lib, name):
    """The symbol's definition from KiCad's library, flattened if it extends another, renamed lib:name."""
    if lib == "cahoots":
        return pico_symbol() if name == "Pico2" else None
    if lib not in _libcache:
        _libcache[lib] = parse(open(os.path.join(SYM_DIR, lib + ".kicad_sym")).read())
    syms = {s[1].strip('"'): s for s in find(_libcache[lib], "symbol")}
    s = syms[name]
    ext = find(s, "extends")
    if ext:
        parent = syms[ext[0][1].strip('"')]
        body = [x for x in parent if not (isinstance(x, list) and x and x[0] in ("property",))]
        props = find(s, "property")
        parent_props = {p[1]: p for p in find(parent, "property")}
        for p in props:
            parent_props[p[1]] = p
        s = body[:2] + list(parent_props.values()) + body[2:]
        # sub-symbol names carry the parent's name; rename to the child's
        s = rename_units(s, ext[0][1].strip('"'), name)
    s = [x for x in s]  # copy
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
    """A 40-pin module symbol: physical pin order, 1 to 20 down the left, 21 to 40 up the right."""
    pins = []
    for n in range(1, 41):
        name = D.PICO_PINS[n]
        kind = "power_out" if name == "3V3" else "power_in" if name in ("GND", "AGND", "VSYS", "VBUS") else "bidirectional"
        if n <= 20:
            x, y, ang = -17.78, 24.13 - (n - 1) * 2.54, 0
        else:
            x, y, ang = 17.78, 24.13 - (40 - n) * 2.54, 180
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
    """pin number -> (x, y, angle) in symbol coordinates (y up), unit 1."""
    out = {}
    for unit in find(sym, "symbol"):
        for pin in find(unit, "pin"):
            at = find(pin, "at")[0]
            num = find(pin, "number")[0][1].strip('"')
            out[num] = (float(at[1]), float(at[2]), float(at[3]) if len(at) > 3 else 0.0)
    return out

# ---------- sheet layout ----------
# Columns of parts; each part gets a cell and its pins get stubs with global labels.
COLS = {
    "pico": ["U1"],
    "panel": ["J1"],
    "boost": ["L1", "C4", "Q1", "R1", "R2", "D3", "C3", "D1", "D2", "C5", "C11"],
    "rails": ["C2", "C6", "C7", "C9", "C10", "C12"],
    "ui": ["SW2", "SW3", "BT1", "SW1", "D4"],
    "contacts": ["P1", "P2", "P3", "P4", "P5", "K1", "K2", "K3", "K4", "K5"],
}
COL_X = {"pico": 40.64, "panel": 111.76, "boost": 162.56, "rails": 215.9, "ui": 261.62, "contacts": 312.42}
ROW_STEP = {"pico": 0, "panel": 0, "boost": 15.24, "rails": 15.24, "ui": 20.32, "contacts": 10.16}

items = []
lib_symbols = {}
refs_placed = {}

def place(ref, x, y):
    p = D.PARTS[ref]
    if p["symbol"] is None:
        return
    lib, name = p["symbol"].split(":")
    sym = lib_symbol(lib, name)
    lib_symbols[p["symbol"]] = sym
    pins = pin_positions(sym)
    u = str(uuid.uuid4())
    props = [
        f'(property "Reference" {q(ref)} (at {x + 3:.2f} {y - 2:.2f} 0) (effects (font (size 1.27 1.27)) (justify left)))',
        f'(property "Value" {q(p["value"])} (at {x + 3:.2f} {y + 0.5:.2f} 0) (effects (font (size 1.27 1.27)) (justify left)))',
        f'(property "Footprint" {q(p["footprint"])} (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide))',
        f'(property "Datasheet" "" (at {x:.2f} {y:.2f} 0) (effects (font (size 1.27 1.27)) hide))',
        f'(property "LCSC" {q(p["lcsc"])} (at {x + 3:.2f} {y + 3:.2f} 0) (effects (font (size 1.0 1.0)) (justify left)))',
    ]
    pin_uuids = " ".join(f'(pin "{n}" (uuid "{uuid.uuid4()}"))' for n in pins)
    items.append(f'(symbol (lib_id {q(p["symbol"])}) (at {x:.2f} {y:.2f} 0) (unit 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (dnp no) (uuid "{u}") '
                 + " ".join(props) + f' {pin_uuids} (instances (project "v1" (path "/{ROOT}" (reference {q(ref)}) (unit 1)))))')
    for num, (px, py, ang) in pins.items():
        net = p["pins"].get(num)
        cx, cy = x + px, y - py           # connection point on the sheet (KiCad y is down)
        if net is None:
            items.append(f'(no_connect (at {cx:.2f} {cy:.2f}) (uuid "{uuid.uuid4()}"))')
            continue
        free = (ang + 180) % 360
        dx, dy = {0: (1, 0), 90: (0, -1), 180: (-1, 0), 270: (0, 1)}[int(free)]
        ex, ey = cx + 2.54 * dx, cy + 2.54 * dy
        items.append(f'(wire (pts (xy {cx:.2f} {cy:.2f}) (xy {ex:.2f} {ey:.2f})) (stroke (width 0) (type default)) (uuid "{uuid.uuid4()}"))')
        items.append(f'(global_label {q(net)} (shape input) (at {ex:.2f} {ey:.2f} {int(free)}) (fields_autoplaced yes) (effects (font (size 1.27 1.27)) (justify {"left" if free == 0 else "right" if free == 180 else "left"})) (uuid "{uuid.uuid4()}") '
                     f'(property "Intersheetrefs" "${{INTERSHEET_REFS}}" (at {ex:.2f} {ey:.2f} 0) (effects (font (size 1.27 1.27)) hide)))')

for col, refs in COLS.items():
    x = COL_X[col]
    y = 60.96 if col in ("pico", "panel") else 30.48
    for ref in refs:
        place(ref, x, y)
        y += ROW_STEP[col]

# Power flags so ERC knows the nets a battery and a module drive.
def pwr_flag(net, x, y):
    sym = lib_symbol("power", "PWR_FLAG"); lib_symbols["power:PWR_FLAG"] = sym
    u = str(uuid.uuid4())
    items.append(f'(symbol (lib_id "power:PWR_FLAG") (at {x} {y} 0) (unit 1) (exclude_from_sim no) (in_bom yes) (on_board yes) (dnp no) (uuid "{u}") '
                 f'(property "Reference" "#FLG_{net}" (at {x} {y - 3} 0) (effects (font (size 1.27 1.27)) hide)) (property "Value" "PWR_FLAG" (at {x} {y - 5} 0) (effects (font (size 1.27 1.27)))) '
                 f'(property "Footprint" "" (at {x} {y} 0) (effects (font (size 1.27 1.27)) hide)) (property "Datasheet" "" (at {x} {y} 0) (effects (font (size 1.27 1.27)) hide)) '
                 f'(pin "1" (uuid "{uuid.uuid4()}")) (instances (project "v1" (path "/{ROOT}" (reference "#FLG_{net}") (unit 1)))))')
    items.append(f'(wire (pts (xy {x} {y}) (xy {x} {y + 2.54})) (stroke (width 0) (type default)) (uuid "{uuid.uuid4()}"))')
    items.append(f'(global_label {q(net)} (shape input) (at {x} {y + 2.54} 270) (effects (font (size 1.27 1.27)) (justify left)) (uuid "{uuid.uuid4()}") '
                 f'(property "Intersheetrefs" "${{INTERSHEET_REFS}}" (at {x} {y} 0) (effects (font (size 1.27 1.27)) hide)))')
for i, net in enumerate(["GND", "VSYS", "BAT+", "BAT_SW"]):
    pwr_flag(net, 40.64 + 12.7 * i, 139.7)

notes = [
    (40, 20, "Cahoots badge V1: a board a Raspberry Pi Pico 2 solders onto. 54 x 86 mm, two layers."),
    (40, 24, "Connections are by net name: every pin carries a global label. design.py is the source; make_sch.py and make_pcb.py draw it."),
    (110, 20, "Panel: GDEY0154D67 on a 24-pin 0.5 mm flex. Pinout and boost circuit from its specification (pp. 7, 28)."),
    (160, 20, "Boost: L1 47uH, Q1 switched by the panel's GDR pin, D3 to VGH (+), C3/D1/D2 pump VGL (-). R2 is the current sense."),
    (260, 20, "Power: CR2032 > slide switch > Schottky > Pico VSYS. Buttons to ground on the chip's pull-ups."),
    (310, 20, "Pile contacts: face pads P1-P5 (GND TX RX GND DET) meet the spring contacts K1-K5 of the card above."),
]
for x, y, t in notes:
    items.append(f'(text {q(t)} (exclude_from_sim no) (at {x} {y} 0) (effects (font (size 1.5 1.5)) (justify left bottom)) (uuid "{uuid.uuid4()}"))')

lib_block = "(lib_symbols\n" + "\n".join(dump(s, 1) for s in lib_symbols.values()) + "\n)"
sheet = f'''(kicad_sch (version 20231120) (generator "cahoots_make_sch") (generator_version "1")
  (uuid "{ROOT}")
  (paper "A2")
  (title_block (title "Cahoots badge V1") (date "2026-10-25") (rev "A") (company "cahoots.network"))
  {lib_block}
  {chr(10).join("  " + i for i in items)}
  (sheet_instances (path "/" (page "1")))
)
'''
open(OUT, "w").write(sheet)
print("wrote", OUT, "with", len(lib_symbols), "symbols")

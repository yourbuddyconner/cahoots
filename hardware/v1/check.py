"""Cross-check: the nets in design.py, in the schematic KiCad exports, and on the board must agree.
Run with KiCad's Python."""
import os
import subprocess
import sys
import xml.etree.ElementTree as ET

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import design as D

CLI = "/Applications/KiCad.app/Contents/MacOS/kicad-cli"
xml_path = "/tmp/cahoots-v1/v1-netlist.xml"
subprocess.run([CLI, "sch", "export", "netlist", "--format", "kicadxml", "-o", xml_path, os.path.join(HERE, "v1.kicad_sch")], check=True, capture_output=True)
tree = ET.parse(xml_path)
sch = {}
for net in tree.getroot().iter("net"):
    name = net.get("name")
    pins = frozenset((n.get("ref"), n.get("pin")) for n in net.findall("node"))
    if name.startswith("unconnected-"):
        continue
    sch[name] = pins

# the flex connector's mechanical pad is on the board, not in the symbol
design = {n: frozenset(x for x in p if x[1] != "MP") for n, p in D.nets().items()}

board = pcbnew.LoadBoard(os.path.join(HERE, "v1.kicad_pcb"))
brd = {}
for fp in board.GetFootprints():
    for pad in fp.Pads():
        if pad.GetNetname() and pad.GetNumber() != "MP":
            brd.setdefault(pad.GetNetname(), set()).add((fp.GetReference(), pad.GetNumber()))
brd = {n: frozenset(p) for n, p in brd.items()}

def compare(a_name, a, b_name, b):
    ok = True
    for net in sorted(set(a) | set(b)):
        if a.get(net) != b.get(net):
            ok = False
            print(f"  {net}: {a_name} {sorted(a.get(net, []))} vs {b_name} {sorted(b.get(net, []))}")
    return ok

print("schematic vs design:", "match" if compare("sch", sch, "design", design) else "DIFFER")
print("board vs design:    ", "match" if compare("board", brd, "design", design) else "DIFFER")

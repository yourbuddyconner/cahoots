"""Build v1.kicad_pcb from design.py. Run with KiCad's own Python, which has pcbnew:
/Applications/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 make_pcb.py
Placement comes from design.py; routing is done afterwards (see README)."""
import math
import os
import sys

import pcbnew
from pcbnew import VECTOR2I, VECTOR2I_MM as MM

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import design as D

FP_DIR = "/Applications/KiCad.app/Contents/SharedSupport/footprints"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "v1.kicad_pcb")

board = pcbnew.BOARD()
ds = board.GetDesignSettings()
ds.SetCopperLayerCount(2)

def P(x, y):
    """Design coordinates (mm, y up, origin at board centre) to KiCad (y down, origin at 100,100)."""
    return MM(100.0 + x, 100.0 - y)

# ---- nets ----
nets = {}
for name in D.nets():
    n = pcbnew.NETINFO_ITEM(board, name)
    board.Add(n)
    nets[name] = n

# ---- custom footprints ----
def lset(*layers):
    ls = pcbnew.LSET()
    for l in layers:
        ls.addLayer(l)
    return ls

def custom_footprint(name):
    fp = pcbnew.FOOTPRINT(board)
    fp.SetFPID(pcbnew.LIB_ID("cahoots", name))
    pad = pcbnew.PAD(fp)
    if name == "Pad_D3.5mm":
        pad.SetNumber("1"); pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE); pad.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
        pad.SetSize(MM(3.5, 3.5)); pad.SetLayerSet(lset(pcbnew.F_Cu, pcbnew.F_Mask))
        fp.Add(pad)
    elif name == "Pogo_SMD_D2.4mm":
        pad.SetNumber("1"); pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE); pad.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
        pad.SetSize(MM(2.4, 2.4)); pad.SetLayerSet(lset(pcbnew.F_Cu, pcbnew.F_Mask, pcbnew.F_Paste))
        fp.Add(pad)
    elif name == "Magnet_NPTH_D5.1mm":
        pad.SetNumber(""); pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE); pad.SetAttribute(pcbnew.PAD_ATTRIB_NPTH)
        pad.SetSize(MM(5.1, 5.1)); pad.SetDrillSize(MM(5.1, 5.1)); pad.SetLayerSet(pcbnew.LSET.AllCuMask())
        fp.Add(pad)
        # keep copper away from the hole
        ring = pcbnew.PCB_SHAPE(fp); ring.SetShape(pcbnew.SHAPE_T_CIRCLE); ring.SetLayer(pcbnew.F_SilkS)
        ring.SetStart(MM(0, 0)); ring.SetEnd(MM(2.9, 0)); ring.SetWidth(pcbnew.FromMM(0.15)); fp.Add(ring)
    return fp

def load_footprint(spec):
    lib, name = spec.split(":")
    if lib == "cahoots":
        return custom_footprint(name)
    fp = pcbnew.FootprintLoad(os.path.join(FP_DIR, lib + ".pretty"), name)
    if fp is None:
        print(f"MISSING footprint {spec}; candidates:")
        for f in sorted(os.listdir(os.path.join(FP_DIR, lib + ".pretty"))):
            if name.split("_")[0].lower() in f.lower() or name[:6].lower() in f.lower():
                print("   ", f)
        sys.exit(1)
    return fp

missing_pads = []
for ref, p in D.PARTS.items():
    fp = load_footprint(p["footprint"])
    fp.SetReference(ref)
    fp.SetValue(p["value"])
    board.Add(fp)
    x, y = p["at"]
    fp.SetPosition(P(x, y))
    if p["side"] == "back":
        fp.SetLayerAndFlip(pcbnew.B_Cu)
    fp.SetOrientationDegrees(p["rot"] if p["side"] == "front" else -p["rot"])
    # Place by the courtyard's centre, not the footprint origin, so (x, y) means the middle of the part.
    bb = fp.GetCourtyard(pcbnew.F_CrtYd if p["side"] == "front" else pcbnew.B_CrtYd).BBox()
    if bb.GetWidth() > 0:
        c = bb.Centre()
        fp.Move(VECTOR2I(P(x, y).x - c.x, P(x, y).y - c.y))
    # references on the fab layer: the silkscreen stays clean
    fp.Reference().SetLayer(pcbnew.F_Fab if p["side"] == "front" else pcbnew.B_Fab)
    pads = {pad.GetNumber(): pad for pad in fp.Pads()}
    for num, net in p["pins"].items():
        if num not in pads:
            missing_pads.append((ref, num))
            continue
        pads[num].SetNet(nets[net])
    for pad in fp.Pads():
        if pad.GetNumber() and pad.GetNumber() not in p["pins"] and pad.GetAttribute() != pcbnew.PAD_ATTRIB_NPTH:
            pass  # unconnected pins stay unconnected (NC, test points)
if missing_pads:
    print("pads not found:", missing_pads)
    sys.exit(1)

# ---- hand routes from design.MANUAL, locked so the autorouter keeps them ----
def pad_of(ref, num):
    for fp in board.GetFootprints():
        if fp.GetReference() == ref:
            for pad in fp.Pads():
                if pad.GetNumber() == num:
                    return pad
    raise KeyError((ref, num))

def layer_of(pad):
    return pcbnew.B_Cu if pad.IsOnLayer(pcbnew.B_Cu) else pcbnew.F_Cu

for net, steps in getattr(D, "MANUAL", {}).items():
    cur = None   # (VECTOR2I, layer)
    last_x = None
    for step in steps:
        if step[0] == "pad":
            pad = pad_of(step[1], step[2]); pos = pad.GetPosition(); layer = layer_of(pad)
            last_x = pcbnew.ToMM(pos.x) - 100.0
            nxt = (pos, layer)
        elif step[0] == "via":
            x = step[1] if step[1] is not None else last_x
            pos = P(x, step[2]); last_x = x
            v = pcbnew.PCB_VIA(board); v.SetPosition(pos); v.SetWidth(pcbnew.FromMM(0.6)); v.SetDrill(pcbnew.FromMM(0.3))
            v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNet(nets[net]); v.SetLocked(True); board.Add(v)
            # a via joins both layers: the track to it stays on the current layer, the next starts on the other
            if cur is not None:
                t = pcbnew.PCB_TRACK(board); t.SetStart(cur[0]); t.SetEnd(pos); t.SetLayer(cur[1]); t.SetWidth(pcbnew.FromMM(0.25)); t.SetNet(nets[net]); t.SetLocked(True); board.Add(t)
            cur = (pos, pcbnew.F_Cu if cur is None or cur[1] == pcbnew.B_Cu else pcbnew.B_Cu)
            continue
        else:
            x = step[2]
            if isinstance(x, tuple):
                x = pcbnew.ToMM(pad_of(*x).GetPosition().x) - 100.0
            elif x is None:
                x = last_x
            pos = P(x, step[3]); last_x = x
            nxt = (pos, pcbnew.F_Cu if step[1] == "F" else pcbnew.B_Cu)
        if cur is not None:
            t = pcbnew.PCB_TRACK(board); t.SetStart(cur[0]); t.SetEnd(nxt[0]); t.SetLayer(nxt[1]); t.SetWidth(pcbnew.FromMM(0.25)); t.SetNet(nets[net]); t.SetLocked(True); board.Add(t)
        cur = nxt

# ---- outline: rounded rectangle with a lanyard slot ----
W, H, R = D.BOARD_W, D.BOARD_H, D.CORNER_R
def seg(a, b, layer=pcbnew.Edge_Cuts):
    s = pcbnew.PCB_SHAPE(board); s.SetShape(pcbnew.SHAPE_T_SEGMENT); s.SetLayer(layer)
    s.SetStart(P(*a)); s.SetEnd(P(*b)); s.SetWidth(pcbnew.FromMM(0.1)); board.Add(s)
def arc(center, start, angle_deg, layer=pcbnew.Edge_Cuts):
    s = pcbnew.PCB_SHAPE(board); s.SetShape(pcbnew.SHAPE_T_ARC); s.SetLayer(layer)
    s.SetCenter(P(*center)); s.SetStart(P(*start)); s.SetArcAngleAndEnd(pcbnew.EDA_ANGLE(angle_deg, pcbnew.DEGREES_T), False)
    s.SetWidth(pcbnew.FromMM(0.1)); board.Add(s)
hw, hh = W / 2, H / 2
seg((-hw + R, hh), (hw - R, hh)); seg((hw, hh - R), (hw, -hh + R)); seg((hw - R, -hh), (-hw + R, -hh)); seg((-hw, -hh + R), (-hw, hh - R))
# corner arcs, drawn as quarter circles; KiCad's y is flipped so the sweep sign flips too
arc((hw - R, hh - R), (hw - R, hh), 90); arc((hw - R, -hh + R), (hw, -hh + R), 90)
arc((-hw + R, -hh + R), (-hw + R, -hh), 90); arc((-hw + R, hh - R), (-hw, hh - R), 90)
# lanyard slot: a stadium 12 x 3
sx, sy, sw, sh = 0.0, D.SLOT_Y, D.SLOT_W, D.SLOT_H
r = sh / 2
seg((sx - sw / 2 + r, sy + r), (sx + sw / 2 - r, sy + r)); seg((sx - sw / 2 + r, sy - r), (sx + sw / 2 - r, sy - r))
arc((sx + sw / 2 - r, sy), (sx + sw / 2 - r, sy + r), 180); arc((sx - sw / 2 + r, sy), (sx - sw / 2 + r, sy - r), 180)

# ---- panel outline and flex on the front silkscreen ----
pc = D.PANEL["center"]; pw, ph = D.PANEL["w"], D.PANEL["h"]
for a, b in [((pc[0]-pw/2, pc[1]+ph/2), (pc[0]+pw/2, pc[1]+ph/2)), ((pc[0]+pw/2, pc[1]+ph/2), (pc[0]+pw/2, pc[1]-ph/2)),
             ((pc[0]+pw/2, pc[1]-ph/2), (pc[0]-pw/2, pc[1]-ph/2)), ((pc[0]-pw/2, pc[1]-ph/2), (pc[0]-pw/2, pc[1]+ph/2))]:
    seg(a, b, pcbnew.F_SilkS)
fx, fw = D.PANEL["flex_x"], D.PANEL["flex_w"]
seg((fx - fw/2, pc[1]-ph/2), (fx - fw/2, pc[1]-ph/2 - 8), pcbnew.F_SilkS); seg((fx + fw/2, pc[1]-ph/2), (fx + fw/2, pc[1]-ph/2 - 8), pcbnew.F_SilkS)
for text, x, y, layer in [("cahoots V1", 0, -31, pcbnew.F_SilkS), ("A", -14, -29, pcbnew.F_SilkS), ("B", 14, -29, pcbnew.F_SilkS),
                          ("GND  TX  RX  GND  DET", 0, -36.5, pcbnew.F_SilkS)]:
    t = pcbnew.PCB_TEXT(board); t.SetText(text); t.SetLayer(layer); t.SetPosition(P(x, y)); t.SetTextSize(MM(1.2, 1.2)); t.SetTextThickness(pcbnew.FromMM(0.2)); board.Add(t)

# ---- ground pours on both layers ----
for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
    z = pcbnew.ZONE(board); z.SetLayer(layer); z.SetNet(nets["GND"]); z.SetIsFilled(False)
    pts = [(-hw, hh), (hw, hh), (hw, -hh), (-hw, -hh)]
    outline = z.Outline(); outline.NewOutline()
    for x, y in pts:
        outline.Append(P(x, y))
    z.SetLocalClearance(pcbnew.FromMM(0.3)); z.SetMinThickness(pcbnew.FromMM(0.25)); z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
    board.Add(z)

# ---- keep-out bands along the edge, so routing stays 0.45 mm in: the fab's minimum is 0.3 ----
BAND = 0.45
for (x0, y0, x1, y1) in [(-hw, hh, hw, hh - BAND), (-hw, -hh + BAND, hw, -hh), (-hw, hh, -hw + BAND, -hh), (hw - BAND, hh, hw, -hh)]:
    k = pcbnew.ZONE(board); k.SetIsRuleArea(True); k.SetDoNotAllowTracks(True); k.SetDoNotAllowVias(True)
    k.SetDoNotAllowZoneFills(False); k.SetDoNotAllowPads(False); k.SetDoNotAllowFootprints(False)
    k.SetLayerSet(lset(pcbnew.F_Cu, pcbnew.B_Cu))
    o = k.Outline(); o.NewOutline()
    for x, y in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]:
        o.Append(P(x, y))
    board.Add(k)

# ---- rules: minimums here, the default net class in v1.kicad_pro ----
ds.m_MinClearance = pcbnew.FromMM(0.2); ds.m_TrackMinWidth = pcbnew.FromMM(0.15)
ds.m_ViasMinSize = pcbnew.FromMM(0.5); ds.m_MinThroughDrill = pcbnew.FromMM(0.3); ds.m_CopperEdgeClearance = pcbnew.FromMM(0.3)
board.Save(OUT)

# The library footprint is the Pico 1's; the Pico 2 has no castellated debug pads. Strip them
# from the saved file, since removing pads through the API does not stick.
def strip_pads(text, names):
    out, i = [], 0
    while True:
        j = text.find("(pad ", i)
        if j < 0:
            out.append(text[i:]); break
        name = text[j + 5:].split()[0].strip('"')
        if name in names:
            depth, k = 0, j
            while True:
                if text[k] == "(": depth += 1
                elif text[k] == ")":
                    depth -= 1
                    if depth == 0: break
                k += 1
            out.append(text[i:j]); i = k + 1
        else:
            out.append(text[i:j + 5]); i = j + 5
    return "".join(out)
with open(OUT) as f:
    text = f.read()
with open(OUT, "w") as f:
    f.write(strip_pads(text, {"D1", "D2", "D3"}))
print("wrote", OUT, "with", len(list(board.GetFootprints())), "footprints and", len(nets), "nets")

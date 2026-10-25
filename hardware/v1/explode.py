"""assembly.png: the badge exploded into its parts beside the assembled badge, for a reader or
an image model to see what goes where. Run with KiCad's Python; needs the repo's video/ node_modules."""
import os
import subprocess
import tempfile

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
CLI = "/Applications/KiCad.app/Contents/MacOS/kicad-cli"
import sys
sys.path.insert(0, HERE)
import design as D
C = D.CASE

board = pcbnew.LoadBoard(os.path.join(HERE, "v1.kicad_pcb"))

def add_model(ref, path, z=0.0, scale=1.0):   # models/ are written in KiCad's 0.1 inch units already
    for fp in board.GetFootprints():
        if fp.GetReference() == ref:
            m = pcbnew.FP_3DMODEL(); m.m_Filename = "${KIPRJMOD}/models/" + path
            m.m_Scale.x = m.m_Scale.y = m.m_Scale.z = scale
            m.m_Offset.z = z
            fp.Models().push_back(m)
            return

# the parts that have no footprint model: magnets in both the lid and the tray, the cell, pins, standoffs
z_floor = -(C["board_t"] + C["below"] + C["floor"])
z_lid_top = C["above"] + C["lid_t"]
for i in range(1, 5):
    add_model(f"M{i}", "magnet.wrl", z_lid_top - C["magnet_t"])    # in the lid's top pocket
    add_model(f"M{i}", "magnet.wrl", z_floor)                       # in the tray's floor pocket
# back-side footprints are flipped: their models' z runs into the board's back, so offsets are from the back face
add_model("BT1", "cell.wrl", 2.2)                                   # the cell sits 2.2 mm off the back face in its holder

# rotate, zoom, perspective, pivot (cm from the board centre), image size
CAMERA = {"assembled": ("-30,0,28", "0.72", True, None, (1800, 2600), "top"), "exploded": ("-30,0,28", "0.33", True, None, (1800, 2600), "top"),
          "exploded-side": ("-82,0,12", "0.50", False, None, (1800, 2600), "top"), "end": ("-30,0,28", "0.72", True, None, (1800, 2600), "bottom")}

def variant(name, lift):
    """Write a copy of the board with each part group lifted by lift[group] mm, and render it."""
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        models = fp.Models()
        for k in range(len(models)):
            m = models[k]
            f = m.m_Filename
            group = ("lid" if "lid.wrl" in f else "tray" if "tray.wrl" in f else "panel" if "panel.wrl" in f
                     else "cell" if "cell.wrl" in f else "pin" if "pin.wrl" in f else "standoff" if "standoff.wrl" in f
                     else "tray" if "pogo_pads.wrl" in f
                     else ("magnet_lid" if k == 0 else "magnet_tray") if "magnet.wrl" in f else "board")
            if (ref, k) not in base_z:
                continue   # the stacked badge model, placed by with_stack
            m.m_Offset.z = base_z[(ref, k)] + lift.get(group, 0.0) * (-1 if fp.IsFlipped() else 1)
    path = os.path.join(HERE, f"v1-{name}.kicad_pcb")
    board.Save(path)
    out = os.path.join(tempfile.gettempdir(), f"cahoots-{name}.png")
    rot, zoom, persp, pivot, (w, h), side_ = CAMERA[name]
    subprocess.run([CLI, "pcb", "render", "--side", side_] + (["--perspective"] if persp else []) + (["--pivot", pivot] if pivot else [])
                   + ["--rotate", rot, "--zoom", zoom, "--quality", "high", "--width", str(w), "--height", str(h), "--background", "transparent", "--floor", "-o", out, path],
                   check=True, capture_output=True)
    os.remove(path)
    return out

base_z = {}
for fp in board.GetFootprints():
    for k, m in enumerate(fp.Models()):
        base_z[(fp.GetReference(), k)] = m.m_Offset.z

# a second badge on top: the whole badge (board, parts and case) exported as one VRML model, lifted one badge height
BADGE_H = C["board_t"] + C["below"] + C["floor"] + C["above"] + C["lid_t"]
whole = os.path.join(tempfile.gettempdir(), "cahoots-badge.wrl")
subprocess.run([CLI, "pcb", "export", "vrml", "--units", "tenths", "--user-origin", "100x100mm", "-f", "-o", whole, os.path.join(HERE, "v1.kicad_pcb")], check=True, capture_output=True)
def with_stack(on):
    for fp in board.GetFootprints():
        if fp.GetReference() == "CASE1":
            models = fp.Models()
            if on:
                m = pcbnew.FP_3DMODEL(); m.m_Filename = whole; m.m_Offset.z = BADGE_H; models.push_back(m)
            else:
                while "cahoots-badge.wrl" in models[len(models) - 1].m_Filename:
                    models.pop()
CAMERA["stack"] = ("-68,0,22", "0.95", True, "0,0,0.7", (2000, 1300), "top")
with_stack(True)
stack = variant("stack", {})
with_stack(False)
assembled = variant("assembled", {})
end = variant("end", {})
# world-space lifts, up positive; parts under the board go down
LIFT = {"lid": 84, "magnet_lid": 104, "panel": 44, "cell": -26, "magnet_tray": -88, "tray": -64}
exploded = variant("exploded", LIFT)
side = variant("exploded-side", {"lid": 58, "magnet_lid": 72, "panel": 32, "cell": -18, "magnet_tray": -62, "tray": -46})

html = f'''<!doctype html><meta charset="utf-8">
<link href="https://fonts.googleapis.com/css2?family=Inter+Tight:wght@600&family=Inter:wght@400;500&display=swap" rel="stylesheet">
<style>
html,body{{margin:0;background:#0b0d10}}
.sheet{{position:relative;width:2400px;height:1800px;background:#0f1216;color:#E8E4DA;font:400 24px/1.35 Inter,system-ui,sans-serif}}
h1{{position:absolute;left:80px;top:56px;margin:0;font:600 56px/1 "Inter Tight",sans-serif;letter-spacing:-0.03em}}
.sub{{position:absolute;left:80px;top:124px;color:#9aa0a6;font-size:24px}}
img{{position:absolute}}
.ex{{left:0px;top:150px;width:860px}}
.side{{left:840px;top:150px;width:760px}}
.as{{left:1600px;top:200px;width:780px}}
.end{{left:1420px;top:1180px;width:380px}}
.stack{{left:1810px;top:1270px;width:570px}}
.cap{{position:absolute;color:#9aa0a6;font-size:21px;letter-spacing:0.06em;text-transform:uppercase;top:1300px}}
ol{{position:absolute;left:80px;top:1370px;margin:0;padding:0 0 0 40px;width:1320px;font-size:23px;line-height:1.3}}
ol li{{margin:0 0 12px;break-inside:avoid}} ol b{{color:#fff;font-weight:500}}
</style>
<div class="sheet">
<h1>Cahoots badge V1: parts and assembly</h1>
<div class="sub">54 × 86 mm board; clear case 60 × 92 × 13 mm. Badges stack face to face: the pogo pins on top of one meet the magnetic pads in the floor of the next.</div>
<img class="ex" src="{exploded}"><div class="cap" style="left:260px">Exploded, from above</div>
<img class="side" src="{side}"><div class="cap" style="left:1060px">Exploded, from the side</div>
<img class="as" src="{assembled}"><div class="cap" style="left:1880px">Assembled</div>
<img class="end" src="{end}"><div class="cap" style="left:1440px;top:1760px">Underside: pads in the floor</div>
<img class="stack" src="{stack}"><div class="cap" style="left:1960px;top:1760px">Two badges stacked</div>
<ol>
<li><b>Lid</b>, clear. Window for the panel, two button holes, a window the pogo pins stand up through, two over the cable sockets. Snaps onto the tray.</li>
<li><b>Magnets</b>, 5 × 2 mm, four in the lid and four in the tray floor.</li>
<li><b>E-paper panel</b>, 1.54", glued into the lid's window, glass flush with the top. Its flex drops to the connector.</li>
<li><b>Board</b>: flex connector, two switches, the pogo pin half at the top end facing up, two JST-SH sockets at the bottom edge; on the back the Raspberry Pi Pico 2, the boost circuit, the coin cell holder.</li>
<li><b>CR2032 cell</b> in its holder on the back.</li>
<li><b>Magnetic connector</b>, Adafruit 5413: a pin half on the board, pins up through the lid, and a pad half set face down in the tray floor, wired to the DN socket. Five contacts on 2.54 mm, a magnet at each end.</li>
<li><b>Tray</b>, clear. The board sits on its ledge; the pad half flush in a window in the floor; notches for the USB and the two cables; four magnet pockets.</li>
</ol>
</div>'''
page = os.path.join(tempfile.gettempdir(), "cahoots-assembly.html")
open(page, "w").write(html)
out = os.path.join(HERE, "assembly.png")
subprocess.run(["node", "-e", '''
const { chromium } = require("playwright");
(async () => { const b = await chromium.launch({ channel: "chrome" }); const p = await b.newPage({ viewport: { width: 2400, height: 1800 } });
  await p.goto("file://" + process.argv[1]); await p.waitForTimeout(1500); await p.evaluate(() => document.fonts.ready);
  await p.screenshot({ path: process.argv[2], clip: {x:0,y:0,width:2400,height:1800} }); await b.close(); })();''', page, out],
               cwd=os.path.join(HERE, "..", "..", "video"), check=True)
print("wrote", out)

"""parts.png: each piece on its own, front and back. KiCad renders boards, so a piece is isolated by
moving it 40 cm sideways and panning the camera to it; the rest of the badge stays out of frame.
Run with KiCad's Python; needs web/ node_modules (npm install in web/)."""
import os
import subprocess
import sys
import tempfile

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
CLI = "/Applications/KiCad.app/Contents/MacOS/kicad-cli"
board = pcbnew.LoadBoard(os.path.join(HERE, "v1.kicad_pcb"))

def group_of(k, f):
    return ("lid" if "lid.wrl" in f else "tray" if "tray.wrl" in f or "pogo_pads.wrl" in f else "panel" if "panel.wrl" in f
            else "magnet" if "magnet.wrl" in f else "cell" if "cell.wrl" in f else "board")

base = {(fp.GetReference(), k): (m.m_Offset.x, m.m_Offset.z) for fp in board.GetFootprints() for k, m in enumerate(fp.Models())}
FAR = 400.0

def render(name, piece, side, rot, zoom):
    """piece "board": every loose model goes far away. Otherwise that piece goes far away and the camera follows it."""
    for fp in board.GetFootprints():
        models = fp.Models()
        for k in range(len(models)):
            m = models[k]  # index access edits in place; iterating yields copies
            g = group_of(k, m.m_Filename)
            bx, bz = base[(fp.GetReference(), k)]
            move = (g != "board") if piece == "board" else (g == piece)
            dx = FAR if move else 0.0
            m.m_Offset.x = bx + dx * (-1 if fp.IsFlipped() else 1)
            m.m_Offset.z = bz
    path = os.path.join(HERE, f"v1-{name}.kicad_pcb"); board.Save(path)
    out = os.path.join(tempfile.gettempdir(), f"cahoots-part-{name}.png")
    args = [CLI, "pcb", "render", "--side", side, "--rotate", rot, "--zoom", str(zoom), "--quality", "high",
            "--width", "1200", "--height", "1500", "--background", "transparent", "--floor", "-o", out, path]
    if piece != "board":
        args += ["--pivot", f"{FAR / 10},0,0"]
    subprocess.run(args, check=True, capture_output=True)
    os.remove(path)
    return out

shots = [
    ("Lid, outside", render("lid-f", "lid", "top", "-25,0,15", 0.6)),
    ("Lid, inside", render("lid-b", "lid", "bottom", "-25,0,15", 0.6)),
    ("E-paper panel, front", render("panel-f", "panel", "top", "-25,0,15", 1.1)),
    ("E-paper panel, back, with its flex", render("panel-b", "panel", "bottom", "-25,0,15", 1.1)),
    ("Board, front", render("board-f", "board", "top", "-25,0,15", 0.6)),
    ("Board, back", render("board-b", "board", "bottom", "-25,0,15", 0.6)),
    ("Tray, inside", render("tray-f", "tray", "top", "-25,0,15", 0.6)),
    ("Tray, outside", render("tray-b", "tray", "bottom", "-25,0,15", 0.6)),
]

cells = "".join(f'<figure><img src="{p}"><figcaption>{t}</figcaption></figure>' for t, p in shots)
html = f'''<!doctype html><meta charset="utf-8">
<link href="https://fonts.googleapis.com/css2?family=Inter+Tight:wght@600&family=Inter:wght@400;500&display=swap" rel="stylesheet">
<style>
html,body{{margin:0;background:#0f1216}}
.sheet{{width:2400px;padding:60px 80px 80px;box-sizing:border-box;background:#0f1216;color:#E8E4DA;font:400 24px/1.3 Inter,system-ui,sans-serif}}
h1{{margin:0 0 8px;font:600 56px/1 "Inter Tight",sans-serif;letter-spacing:-0.03em}}
.sub{{color:#9aa0a6;margin:0 0 30px}}
.grid{{display:grid;grid-template-columns:repeat(4,1fr);gap:30px 24px}}
figure{{margin:0;background:#141920;border-radius:18px;padding:10px 10px 18px}}
img{{width:100%;display:block;border-radius:10px}}
figcaption{{margin-top:12px;font-size:24px;color:#E8E4DA}}
</style>
<div class="sheet"><h1>Cahoots badge V1: the pieces</h1>
<p class="sub">Each part alone, both faces. Clear parts are drawn translucent. The lid and tray are 3D printed; the board is 54 × 86 mm; the panel is 31.8 × 37.3 mm.</p>
<div class="grid">{cells}</div></div>'''
page = os.path.join(tempfile.gettempdir(), "cahoots-parts.html"); open(page, "w").write(html)
out = os.path.join(HERE, "parts.png")
subprocess.run(["node", "-e", '''
const { chromium } = require("playwright");
(async () => { const b = await chromium.launch({ channel: "chrome" }); const p = await b.newPage({ viewport: { width: 2400, height: 1200 } });
  await p.goto("file://" + process.argv[1]); await p.waitForTimeout(1500); await p.evaluate(() => document.fonts.ready);
  await p.screenshot({ path: process.argv[2], fullPage: true }); await b.close(); })();''', page, out],
               cwd=os.path.join(HERE, "..", "..", "web"), check=True)
print("wrote", out)

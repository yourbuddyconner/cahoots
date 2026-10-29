"""The V1 case from design.py: a tray and a lid, printable, that click together. Plain python3
with CadQuery. Writes case/tray.step, case/lid.step, .stl for the printer, and .wrl copies for
KiCad's 3D view (transparent, so the board shows through like the clear version will).

Run: python3 case.py"""
import os
import sys

import cadquery as cq

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import design as D

C = D.CASE
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "case")
os.makedirs(OUT, exist_ok=True)

BW, BH = D.BOARD_W, D.BOARD_H
R = C["corner_r"]
# the tray's outer footprint: the board plus clearance plus a wall
TW, TH = BW + 2 * (C["gap"] + C["wall"]), BH + 2 * (C["gap"] + C["wall"])
TR = R + C["gap"] + C["wall"]
# the lid's skirt wraps outside the tray's wall
LW, LH = TW + 2 * (C["skirt_gap"] + C["skirt_t"]), TH + 2 * (C["skirt_gap"] + C["skirt_t"])
LR = TR + C["skirt_gap"] + C["skirt_t"]

z_board_bottom = -C["board_t"]
z_floor_top = z_board_bottom - C["below"]
z_floor_bottom = z_floor_top - C["floor"]
z_wall_top = C["above"]                      # the tray's wall ends where the lid plate begins
z_lid_top = z_wall_top + C["lid_t"]
z_skirt_bottom = z_wall_top - C["skirt"]

def rrect(w, h, r, z0, z1):
    return cq.Workplane("XY").workplane(offset=z0).rect(w, h).extrude(z1 - z0).edges("|Z").fillet(r)

def slot(w, h, z0, z1, x=0.0, y=0.0):
    return cq.Workplane("XY").workplane(offset=z0).center(x, y).slot2D(w, h).extrude(z1 - z0)

# ---------- the tray ----------
tray = rrect(TW, TH, TR, z_floor_bottom, z_wall_top)
# cavity below the board, narrower than the board so a ledge carries it
ledge = 1.2
tray = tray.cut(rrect(BW - 2 * ledge, BH - 2 * ledge, max(R - ledge, 0.5), z_floor_top, z_board_bottom + 0.01))
# the board's pocket and the cavity above it, up to the wall top
tray = tray.cut(rrect(BW + 2 * C["gap"], BH + 2 * C["gap"], R + C["gap"], z_board_bottom, z_wall_top + 1))
# magnet pockets in the floor's outer face
for (x, y) in D.MAGNETS:
    tray = tray.cut(cq.Workplane("XY").workplane(offset=z_floor_bottom - 0.01).center(x, y).circle(C["magnet_d"] / 2).extrude(C["magnet_t"]))
# two openings in the end wall for the port cables, from the board's face up through the wall top
for x in D.PORTS.values():
    tray = tray.cut(cq.Workplane("XY").workplane(offset=-0.2).center(x, -TH / 2 + C["wall"] / 2).rect(C["port_w"], C["wall"] + 1).extrude(z_wall_top + 1))
# the expansion socket is on the back under the Qwiic socket: its opening continues below the board
tray = tray.cut(cq.Workplane("XY").workplane(offset=z_board_bottom - C["port_h"]).center(D.PORTS["QW"], -TH / 2 + C["wall"] / 2).rect(C["port_w"], C["wall"] + 1).extrude(C["port_h"] + 0.5))
# the window in the floor the pad half sits in, face down and flush with the outside
pw_, ph_ = C["pogo_win"]
pogo_c = (0, D.POGO_Y + D.POGO["d"] / 2 - D.POGO["pin_to_back"])
tray = tray.cut(cq.Workplane("XY").workplane(offset=z_floor_bottom - 1).center(*pogo_c).rect(pw_, ph_).extrude(C["floor"] + 2))
# openings in the floor under the two slide switches (on the back, facing the floor), so a fingernail reaches them
sw_w, sw_h = C["switch_win"]
for ref in ("SW1", "SW4"):
    x, y = D.PARTS[ref]["at"]; rot = D.PARTS[ref]["rot"] % 180
    w, h = (sw_w, sw_h) if rot == 0 else (sw_h, sw_w)
    tray = tray.cut(cq.Workplane("XY").workplane(offset=z_floor_bottom - 1).center(x, y).rect(w, h).extrude(C["floor"] + 2))
# a notch in the right wall for the Pico's USB socket
ux, uy, uw, uh = C["usb"]
tray = tray.cut(cq.Workplane("XY").workplane(offset=z_board_bottom - uh).center(ux + C["gap"] + C["wall"] / 2, uy).rect(C["wall"] + C["gap"] + 2, uw).extrude(uh + 0.5))
# four windows in the wall for the lid's snaps: through the wall, 1.6 above the board, two per long side
snap_z0 = C["snap_z"]
for sx in (-1, 1):
    for sy in (-24.0, 24.0):
        tray = tray.cut(cq.Workplane("XY").workplane(offset=snap_z0).center(sx * (TW / 2 - C["wall"] / 2), sy).rect(C["wall"] + 1, C["snap_w"] + 0.4).extrude(1.4))

# ---------- the lid ----------
lid = rrect(LW, LH, LR, z_skirt_bottom, z_lid_top)
# hollow: the skirt wraps the tray wall, the plate sits on the wall top
lid = lid.cut(rrect(TW + 2 * C["skirt_gap"], TH + 2 * C["skirt_gap"], TR + C["skirt_gap"], z_skirt_bottom - 1, z_wall_top))
# the panel window: the panel sits on the board on tape, and the plate's opening shows the active
# area with a lip over the panel's edge. Under the lip a frame comes down to just above the glass,
# so the panel cannot lift, with a gap at the bottom edge where the flex leaves
pc, pw, ph = D.PANEL["center"], D.PANEL["w"], D.PANEL["h"]
lip = C["panel_lip"]
lid = lid.cut(cq.Workplane("XY").workplane(offset=z_wall_top - 1).center(*pc).rect(pw - 2 * lip, ph - 2 * lip).extrude(C["lid_t"] + 2))
z_glass = C["panel_tape"] + 1.0                       # the panel is 1.0 thick
z_frame = z_glass + C["panel_press"]
fx, fw = D.PANEL["flex_x"], D.PANEL["flex_w"]
frame = (cq.Workplane("XY").workplane(offset=z_frame).center(*pc).rect(pw + 1.0, ph + 1.0).extrude(z_wall_top - z_frame)
         .cut(cq.Workplane("XY").workplane(offset=z_frame - 1).center(*pc).rect(pw - 2 * lip, ph - 2 * lip).extrude(z_wall_top + 2))
         .cut(cq.Workplane("XY").workplane(offset=z_frame - 1).center(fx, pc[1] - ph / 2).rect(fw + 2.0, 4.0).extrude(z_wall_top + 2)))
lid = lid.union(frame)
# button holes
for ref in ("SW2", "SW3"):
    x, y = D.PARTS[ref]["at"]
    lid = lid.cut(cq.Workplane("XY").workplane(offset=z_wall_top - 1).center(x, y).circle(C["button_hole"] / 2).extrude(C["lid_t"] + 2))
# magnet pockets in the lid top
for (x, y) in D.MAGNETS:
    lid = lid.cut(cq.Workplane("XY").workplane(offset=z_lid_top - C["magnet_t"]).center(x, y).circle(C["magnet_d"] / 2).extrude(C["magnet_t"] + 0.01))
# the cable openings continue through the lid's skirt, and the socket bodies (4.3 tall) rise into windows in the plate
for x in D.PORTS.values():
    lid = lid.cut(cq.Workplane("XY").workplane(offset=-0.2).center(x, -LH / 2 + C["skirt_t"] / 2).rect(C["port_w"], C["skirt_t"] + 1).extrude(C["port_h"] + 0.2))
    lid = lid.cut(cq.Workplane("XY").workplane(offset=z_wall_top - 1).center(x, D.PORT_Y).rect(C["port_w"], 7.0).extrude(C["lid_t"] + 2))
# the window the pin half's body rises into, its face just under the lid's top and its pins above it
lid = lid.cut(cq.Workplane("XY").workplane(offset=z_wall_top - 1).center(*pogo_c).rect(pw_, ph_).extrude(C["lid_t"] + 2))
# the snaps: bumps on the skirt's inner face that drop into the tray's windows
for sx in (-1, 1):
    for sy in (-24.0, 24.0):
        x_in = sx * (TW / 2 + C["skirt_gap"])
        # a wedge: flat catch on top, a slope below so the skirt rides over the wall top as the lid goes on
        h = C["snap"]
        prof = [(0, 0), (-sx * h, 0), (-sx * h, -0.5), (0, -1.6)]   # in the XZ plane, x inward from the skirt face
        bump = (cq.Workplane("XZ").polyline(prof).close().extrude(C["snap_w"] / 2, both=True)
                .translate((x_in, sy, snap_z0 + 1.0)))
        lid = lid.union(bump)
        # a slit each side of the bump so that part of the skirt can flex
        for dy in (-1, 1):
            lid = lid.cut(cq.Workplane("XY").workplane(offset=z_skirt_bottom - 0.5).center(x_in + sx * C["skirt_t"] / 2, sy + dy * (C["snap_w"] / 2 + 0.6)).rect(C["skirt_t"] + 1, 0.6).extrude(C["skirt"] + 0.5))
# ribs that hold the board down onto the tray's ledge at its corners (clear of the magnet holes)
for (x, y) in [(-24.5, 30), (24.5, 30), (-24.5, -30), (24.5, -30)]:
    lid = lid.union(cq.Workplane("XY").workplane(offset=0).center(x, y).rect(3.0, 6.0).extrude(z_wall_top))

def export(shape, name, rgb, alpha):
    cq.exporters.export(shape, os.path.join(OUT, name + ".step"))
    cq.exporters.export(shape, os.path.join(OUT, name + ".stl"), tolerance=0.02, angularTolerance=0.1)
    wrl = os.path.join(OUT, name + ".wrl")
    cq.exporters.export(shape, wrl, exportType=cq.exporters.ExportTypes.VRML, tolerance=0.05, angularTolerance=0.2)
    # KiCad reads VRML in 0.1 inch units, and we want the clear look: rewrite the material
    text = open(wrl).read()
    import re as _re
    text = _re.sub(r"Material \{[^}]*\}", f"Material {{ diffuseColor {rgb} transparency {alpha} specularColor 0.7 0.7 0.7 shininess 0.6 ambientIntensity 0.3 }}", text)
    open(wrl, "w").write(text)

export(tray, "tray", "0.88 0.92 0.97", 0.72)
export(lid, "lid", "0.88 0.92 0.97", 0.72)
print(f"tray {TW:.1f} x {TH:.1f} x {z_wall_top - z_floor_bottom:.1f} mm, lid {LW:.1f} x {LH:.1f}, badge {LW:.1f} x {LH:.1f} x {z_lid_top - z_floor_bottom:.1f} mm")
print(f"pogo pins stand {D.POGO['tall'] + 1.4 - z_lid_top:.1f} mm above the lid; the pad half's face is flush with the floor at z {z_floor_bottom:.1f}")
print(f"the panel's glass sits {z_lid_top - z_glass:.1f} mm below the lid's top, held by a frame {z_wall_top - z_frame:.2f} mm deep with a {lip} mm lip")

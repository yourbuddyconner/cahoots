"""Simple VRML models for the parts that have none: magnets, the coin cell, the magnetic pogo halves.
KiCad reads VRML in 0.1 inch units with z normal to the board, and draws Box and IndexedFaceSet
but not the Cylinder primitive, so round parts are written as 32-sided prisms."""
import math
import os
S = 1 / 2.54
HERE = os.path.dirname(os.path.abspath(__file__))

def material(rgb, shiny=0.5):
    return f"appearance Appearance {{ material Material {{ diffuseColor {rgb} specularColor 0.5 0.5 0.5 shininess {shiny} }} }}"

def prism(r, h, rgb, center, axis="z", n=32, shiny=0.5):
    """A closed cylinder of radius r and length h along `axis`, centred on `center` (mm), as a face set."""
    cx, cy, cz = center
    ring = [(r * math.cos(2 * math.pi * i / n), r * math.sin(2 * math.pi * i / n)) for i in range(n)]
    pts = []
    for end in (-h / 2, h / 2):
        for (u, v) in ring:
            if axis == "z":
                pts.append((cx + u, cy + v, cz + end))
            elif axis == "y":
                pts.append((cx + u, cy + end, cz + v))
            else:
                pts.append((cx + end, cy + u, cz + v))
    faces = []
    for i in range(n):
        j = (i + 1) % n
        faces.append([i, j, n + j, n + i])
    faces.append(list(range(n - 1, -1, -1)))
    faces.append(list(range(n, 2 * n)))
    coords = " ".join(f"{x * S:.4f} {y * S:.4f} {z * S:.4f}" for (x, y, z) in pts)
    index = " ".join(" ".join(str(k) for k in f) + " -1" for f in faces)
    return (f"Shape {{ {material(rgb, shiny)} geometry IndexedFaceSet {{ solid FALSE coord Coordinate {{ point [ {coords} ] }} "
            f"coordIndex [ {index} ] }} }}\n")

def box(size, rgb, center):
    sx, sy, sz = size; cx, cy, cz = center
    return (f"Transform {{ translation {cx * S:.4f} {cy * S:.4f} {cz * S:.4f} children [ Shape {{ {material(rgb)} "
            f"geometry Box {{ size {sx * S:.4f} {sy * S:.4f} {sz * S:.4f} }} }} ] }}\n")

HEAD = "#VRML V2.0 utf8\n"
def disc(r, h, rgb, z0=0.0, shiny=0.5):
    return HEAD + prism(r, h, rgb, (0, 0, z0 + h / 2), "z", shiny=shiny)

open(os.path.join(HERE, "magnet.wrl"), "w").write(disc(2.5, 2.0, "0.35 0.36 0.38", 0, 0.8))     # 5 x 2 mm disc
open(os.path.join(HERE, "cell.wrl"), "w").write(disc(10.0, 3.2, "0.75 0.75 0.72", 0, 0.7))      # CR2032

def pogo(pins):
    """A pogo half, Yiwei MG05254 right angle: black body 20.08 wide, 4 deep (y 1.4 to 5.4 from the pin row),
    4.2 tall, face toward +y; two nickel magnets and five gold contacts on the face, the pin half's standing proud."""
    out = HEAD + box((20.08, 4.0, 4.2), "0.12 0.12 0.13", (0, 3.4, 2.1))
    for x in (-8.5, 8.5):
        out += prism(1.6, 0.3, "0.8 0.8 0.82", (x, 5.5, 2.1), "y", shiny=0.9)
    for i in range(5):
        x = (i - 2) * 2.54
        h = 1.4 if pins else 0.3
        out += prism(0.5, h, "0.85 0.7 0.3", (x, 5.4 + h / 2, 2.1), "y", shiny=0.8)
        if pins:
            out += prism(0.3, 0.6, "0.85 0.7 0.3", (x, 6.8 + 0.3, 2.1), "y", shiny=0.8)   # the plunger tip
        out += prism(0.3, 1.6, "0.85 0.7 0.3", (x, 0, -0.5), "z", shiny=0.8)             # the tail through the board
    return out
open(os.path.join(HERE, "pogo.wrl"), "w").write(pogo(True))
open(os.path.join(HERE, "pogo_pads.wrl"), "w").write(pogo(False))
print("models written")

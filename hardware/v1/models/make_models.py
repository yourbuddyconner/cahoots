"""Simple VRML models for the parts that have none: magnets, the coin cell, spring pins, pad standoffs.
KiCad reads VRML in 0.1 inch units; z is normal to the board."""
import os
S = 1 / 2.54
HERE = os.path.dirname(os.path.abspath(__file__))

def cyl(r, h, rgb, z0=0.0, shiny=0.4):
    # a VRML Cylinder stands along Y; rotate it onto Z and lift its base to z0
    return (f'#VRML V2.0 utf8\nTransform {{ translation 0 0 {(z0 + h / 2) * S:.4f} rotation 1 0 0 1.5708 children [ Shape {{ '
            f'appearance Appearance {{ material Material {{ diffuseColor {rgb} specularColor 0.5 0.5 0.5 shininess {shiny} }} }} '
            f'geometry Cylinder {{ radius {r * S:.4f} height {h * S:.4f} }} }} ] }}\n')

open(os.path.join(HERE, "magnet.wrl"), "w").write(cyl(2.5, 2.0, "0.35 0.36 0.38", 0, 0.8))      # 5 x 2 mm disc
open(os.path.join(HERE, "cell.wrl"), "w").write(cyl(10.0, 3.2, "0.75 0.75 0.72", 0, 0.7))       # CR2032
open(os.path.join(HERE, "pin.wrl"), "w").write(cyl(0.75, 12.0, "0.85 0.7 0.3", 0, 0.8))         # spring pin, 12 mm, through the tray floor
open(os.path.join(HERE, "standoff.wrl"), "w").write(cyl(1.75, 5.0, "0.85 0.7 0.3", 0, 0.8))     # brass standoff on a pad
print("models written")

#!/usr/bin/env python3
"""Export the current V1 PCB assembly for the website. Requires KiCad 10."""
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
cli = os.environ.get('KICAD_CLI') or shutil.which('kicad-cli') or '/Applications/KiCad.app/Contents/MacOS/kicad-cli'
subprocess.run([
    cli, 'pcb', 'export', 'glb', '--force', '--subst-models',
    '--include-silkscreen', '--include-soldermask', '--include-pads',
    '-o', str(ROOT / 'site/assets/badge-v1.glb'),
    str(ROOT / 'hardware/v1/v1.kicad_pcb'),
], check=True)
shutil.copyfile(ROOT / 'hardware/v1/front.png', ROOT / 'site/assets/v1-front.png')

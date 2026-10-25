#!/bin/sh
# Regenerate V1 from design.py: schematic, BOM, board, routing, checks, renders.
set -e
cd "$(dirname "$0")"
PY=/Applications/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
CLI=/Applications/KiCad.app/Contents/MacOS/kicad-cli
JAVA=${JAVA:-java}; FR=${FREEROUTING:-freerouting.jar}
python3 make_sch.py && python3 make_bom.py
$PY make_pcb.py; $PY route.py export || true   # pcbnew can crash on exit after the export; the file is already written
test -s v1.dsn
$JAVA -Djava.awt.headless=true -jar "$FR" -de v1.dsn -do v1.ses -mp 200 -da | grep -i "incomplete connections" | tail -1
$PY route.py import && $PY check.py
$CLI sch erc --severity-error v1.kicad_sch | tail -1
$CLI pcb drc --severity-error v1.kicad_pcb | tail -2
$CLI pcb render --side top --width 1200 --height 1800 --zoom 1 --background opaque -o front.png v1.kicad_pcb >/dev/null
$CLI pcb render --side bottom --width 1200 --height 1800 --zoom 1 --background opaque -o back.png v1.kicad_pcb >/dev/null

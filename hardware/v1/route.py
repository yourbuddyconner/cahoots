"""Autoroute v1.kicad_pcb with Freerouting, then fill the ground pours. Run with KiCad's Python.
  python3 route.py export   -> writes v1.dsn
  python3 route.py import   -> reads v1.ses, fills zones, saves v1.kicad_pcb
Freerouting itself runs in between: java -jar freerouting.jar -de v1.dsn -do v1.ses -mp 50
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
PCB = os.path.join(HERE, "v1.kicad_pcb")
DSN = os.path.join(HERE, "v1.dsn")
SES = os.path.join(HERE, "v1.ses")

board = pcbnew.LoadBoard(PCB)
if sys.argv[1] == "export":
    # Export without the ground pours, so the router draws ground as tracks too. The pours come
    # back on import as extra copper; nothing then depends on a pour reaching a pad.
    for z in list(board.Zones()):
        if not z.GetIsRuleArea():
            board.Remove(z)
    ok = pcbnew.ExportSpecctraDSN(board, DSN)
    # Mark the hand routes as protected, or the router treats them as its own and moves them.
    import re
    text = open(DSN).read()
    text, n = re.subn(r"\(wire\s*\(path", "(wire (type protect) (path", text)
    open(DSN, "w").write(text)
    print("dsn:", ok, DSN, "protected wires:", n)
elif sys.argv[1] == "import":
    ok = pcbnew.ImportSpecctraSES(board, SES)
    print("ses:", ok)
    for z in board.Zones():
        if z.GetIsRuleArea():
            continue
        z.SetThermalReliefGap(pcbnew.FromMM(0.3)); z.SetThermalReliefSpokeWidth(pcbnew.FromMM(0.3))
        z.SetMinThickness(pcbnew.FromMM(0.2)); z.SetLocalClearance(pcbnew.FromMM(0.25))
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(board.Zones())
    board.Save(PCB)
    print("tracks:", len(board.GetTracks()), "saved", PCB)

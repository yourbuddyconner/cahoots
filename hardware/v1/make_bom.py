"""bom.csv in the columns JLCPCB's assembly form takes, from design.py. Plain python3."""
import csv
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import design as D

rows = {}
for ref, p in D.PARTS.items():
    if p["symbol"] is None or ref.startswith(("P", "M", "ANT")):
        continue  # pads, magnet holes and the antenna loop are the board itself
    key = (p["value"], p["footprint"], p["lcsc"])
    rows.setdefault(key, []).append(ref)
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bom.csv")
with open(out, "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #", "Note"])
    for (value, fp, lcsc), refs in sorted(rows.items(), key=lambda kv: kv[1][0]):
        code = lcsc.split()[0] if lcsc.startswith("C") and lcsc.split()[0][1:].isdigit() else ""
        w.writerow([value, ",".join(sorted(refs)), fp.split(":")[1], code, lcsc if not code else lcsc[len(code):].strip()])
print("wrote", out)
for row in csv.reader(open(out)):
    print("  " + " | ".join(row))

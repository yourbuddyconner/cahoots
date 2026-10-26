# V1 harness connectors: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** V1 gains a Qwiic socket and a spare-GPIO header, so peripherals plug in without soldering and firmware forks have pins to drive, and the pogo pair is recorded as carrying no power.

**Architecture:** `hardware/v1/design.py` is the single source for the schematic, the board and the BOM; `make.sh` regenerates everything, autoroutes with Freerouting, and runs the cross-check, ERC and DRC. The two connectors are two `part()` entries and two GPIO assignments; the rest is regeneration and the docs that carry the pin map.

**Tech Stack:** KiCad 9 with its Python, Freerouting, the existing generator scripts.

**Spec:** `docs/superpowers/specs/2026-10-25-custodian-architecture.md`, section "V1 board edits"

## Global Constraints

- Edit `design.py` only; never the generated `.kicad_*` files, `bom.csv` or the renders.
- `make.sh` ends with Freerouting reporting 0 incomplete connections, `check.py` reporting that the design, schematic and board nets agree, and ERC and DRC reporting no errors.
- Pin names exactly: `I2C_SDA` on `GP8`, `I2C_SCL` on `GP9`; header nets `3V3`, `GND`, `GP6`, `GP7`, `GP10`, `GP11`, `GP12`, `GP13`, `GP22`, `GP26` (the GPIO nets named after the pin).
- Qwiic pin order on J5: 1 GND, 2 3V3, 3 SDA, 4 SCL.
- Pogo pin 3 remains unconnected.
- Docs in the plain voice; no em dashes.

## Review Focus

1. The header's through-hole pins must not come out on the front under the panel glass (panel spans y from about -1.7 to 35.7 on the front). Checked by the placement in Task 1 and the front render in Task 2.
2. The Qwiic socket's cable must exit over the board edge, like the pile ports, so a plugged cable clears the tray wall. Same rotation as J2 and J3. Task 1.
3. No new net shares a Pico pin with an existing signal. `check.py` and the design-level assertion in Task 1.
4. DRC courtyard overlap between J5 and the DN port, or between J6 and the power switch or battery holder. Task 2 shifts placement until clean.
5. The DIY Pico 2 wiring table and the V1 pin map must list the same I2C pins. Task 3.

---

### Task 1: Two connectors in `design.py`

**Files:**
- Modify: `hardware/v1/design.py:37-44` (the `GPIO` map), `:25-26` (`PORTS`), after `:113` (new parts), `:3-6` (the docstring, which still says "five gold pads" and "five spring contacts"; make it say the pogo pair and the two sockets, and the new connectors)

**Interfaces:**
- Produces: `GPIO["I2C_SDA"] = "GP8"`, `GPIO["I2C_SCL"] = "GP9"`, `HEADER = ["3V3", "GND", "GP6", "GP7", "GP10", "GP11", "GP12", "GP13", "GP22", "GP26"]` (pin order 1 to 10), parts `J5` and `J6`.

- [ ] **Step 1: Add a design-level check that fails first**

At the end of `design.py`'s `__main__`, before the print, add:

```python
pins_in_use = list(GPIO.values()) + [n for n in HEADER if n.startswith("GP")]
assert len(pins_in_use) == len(set(pins_in_use)), "a Pico pin is used twice"
assert "I2C_SDA" in GPIO and GPIO["I2C_SDA"] == "GP8"
assert "J5" in PARTS and PARTS["J5"]["pins"] == {"1": "GND", "2": "3V3", "3": "I2C_SDA", "4": "I2C_SCL", "MP": "GND"}
assert "J6" in PARTS and [PARTS["J6"]["pins"][str(i)] for i in range(1, 11)] == ["3V3", "GND", "GP6", "GP7", "GP10", "GP11", "GP12", "GP13", "GP22", "GP26"]
assert "3" not in PARTS["J4"]["pins"], "pogo pin 3 stays spare"
```

Run: `python3 hardware/v1/design.py`
Expected: AssertionError on `I2C_SDA`.

- [ ] **Step 2: Add the pins, the header list and the parts**

`GPIO` gains `"I2C_SDA": "GP8", "I2C_SCL": "GP9"`. The header GPIOs are not in `GPIO` (they carry no signal); their nets are the pin names, added to `pico_nets` by a loop over `HEADER` for the entries that start with `GP`.

```python
PORTS = {"UP": 13.0, "DN": -13.0, "QW": -23.0}
part("J5", "Connector_Generic:Conn_01x04", "Connector_JST:JST_SH_SM04B-SRSS-TB_1x04-1MP_P1.00mm_Horizontal",
     "Qwiic / STEMMA QT, JST-SH 4", "C160404 JST SM04B-SRSS-TB", "front", (PORTS["QW"], PORT_Y), 180,
     {"1": "GND", "2": "3V3", "3": "I2C_SDA", "4": "I2C_SCL", "MP": "GND"})
part("J6", "Connector_Generic:Conn_02x05_Odd_Even", "Connector_PinHeader_2.54mm:PinHeader_2x05_P2.54mm_Vertical",
     "spare GPIO, 2x5", "C2337 or any 2x5 2.54 mm header", "back", (-4.0, -36.0), 0,
     {str(i + 1): net for i, net in enumerate(HEADER)})
```

If `make_sch.py` needs a symbol-to-footprint rule for the 2x5 header, add it where the 1x4 sockets are handled.

- [ ] **Step 3: Run the design check**

Run: `python3 hardware/v1/design.py | tail -1`
Expected: the parts and nets count line, no assertion.

- [ ] **Step 4: Commit**

```bash
git add hardware/v1/design.py
git commit -m "V1: a Qwiic socket on I2C0 and a 2x5 header of spare GPIO; pogo pin 3 stays spare"
```

---

### Task 2: Regenerate, route, check

**Files:**
- Modify (generated): `hardware/v1/v1.kicad_sch`, `v1.kicad_pcb`, `bom.csv`, `front.png`, `back.png`, `schematic.png`
- Modify if needed: `hardware/v1/design.py` placements only

- [ ] **Step 1: Regenerate**

Run: `cd hardware/v1 && sh make.sh`
Expected: Freerouting's line shows `0 incomplete connections`; `check.py` prints that all three net lists agree; ERC and DRC print no errors.

- [ ] **Step 2: If DRC reports a courtyard overlap**, move the offending part in `design.py` (J5 further left, down to x = -24.0 with the board edge at -27; J6 right, to x = -2.0) and rerun `make.sh` until clean. Record the final coordinates in the commit message.

- [ ] **Step 3: Look at the renders**

Open `front.png`: J5 sits on the bottom edge left of the DN port with its opening toward the edge, and no header pins show under the panel area. Open `back.png`: J6 sits between the power switch and the battery holder.

- [ ] **Step 4: Regenerate the assembly views**

Run: `cd hardware/v1 && python3 parts.py && sh hero.sh` if those scripts take no arguments (read their headers first); otherwise skip and note it.

- [ ] **Step 5: Commit**

```bash
git add hardware/v1
git commit -m "V1: regenerated with J5 and J6; routes, checks and renders"
```

---

### Task 3: The docs that carry the pin map

**Files:**
- Modify: `hardware/v1/README.md` ("What is on it": a paragraph for the Qwiic socket and the header, and the pin-map table rows for `I2C_SDA GP8 pin 11`, `I2C_SCL GP9 pin 12`, and the eight header GPIOs with their Pico pin numbers from `PICO_PINS`; the pile paragraph states that pin 3 carries nothing and why), `docs/manufacturing.md` (the V1 parts list gains the two connectors), `docs/diy-badge.md` (the Pico 2 wiring table gains an I2C row pair, and one sentence saying any Qwiic board plugs into V1 or, on the breadboard, into GP8 and GP9), `docs/plan.md` (M4b mentions the two connectors)

- [ ] **Step 1: Make the edits.** Pico physical pins: GP8 is pin 11, GP9 pin 12, GP6 pin 9, GP7 pin 10, GP10 pin 14, GP11 pin 15, GP12 pin 16, GP13 pin 17, GP22 pin 29, GP26 pin 31.
- [ ] **Step 2: Verify** the README's table against `python3 hardware/v1/design.py` output (every net the table lists appears with the right U1 pad).
- [ ] **Step 3: Commit**

```bash
git add hardware/v1/README.md docs/manufacturing.md docs/diy-badge.md docs/plan.md
git commit -m "Docs: V1's Qwiic socket and spare-GPIO header, and the pogo pair carries no power"
```

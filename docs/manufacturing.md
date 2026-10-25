# From a breadboard to a badge you can order

*Written 2026-10-25. How a custom board gets made, what ours needs, and the order to do it in. The benchmark it builds toward is the manufactured badge in [Build a badge](diy-badge.md): a 54 × 86 mm card about 3 mm thick, the 1.54" e-paper, two buttons, magnets and pads for the pile, a lanyard slot.*

## Where we are

A dev board and a display module on jumper wires, with firmware that is about to run its first ceremony. That is a prototype of the electronics, not of the product. Nothing here is on a board we designed, nothing holds together without a breadboard, and the parts are sized for convenience rather than for a card.

Yes, we design a PCB. In fact we design two: a simple one soon, to learn the process and to have something that works at a meetup, and the real card later, once two questions below are answered.

## How a board gets made, in plain words

1. **Block diagram and budget.** What parts, how they connect, and how much current, board area and money each gets. Half the design decisions are made here.
2. **Parts.** Choose exact part numbers with footprints, and check they are in stock at the assembler's supplier. For small runs that means the parts library at JLCPCB or PCBWay, fed by LCSC.
3. **Schematic.** Draw the circuit in KiCad, which is free and open. Most of a design like ours is copied from reference designs: Espressif publishes the ESP32-S3 hardware guidelines, and the e-paper's boost circuit is on every open Waveshare schematic.
4. **Layout.** Draw the board outline, place the parts, route the copper, run the design rule checks. Two copper layers for a simple board; four once a bare chip with its memory is on it.
5. **Review.** Someone who has done it before reads the schematic and the layout. Cheap compared with a wasted spin.
6. **Order.** Export Gerbers, drill files, the bill of materials and the placement file. Upload to JLCPCB or PCBWay with assembly. Five boards arrive in two to three weeks, for $100 to $400 depending on the parts.
7. **Bring-up.** Power first, with a current meter. Then flash the firmware, which already runs on this chip, and work through the self-test one peripheral at a time. Expect to find two or three mistakes, fix them, and order again. Two spins is normal, three is not a failure.
8. **Mechanical.** The tray, the top sheet, the magnets, the contacts. Prototyped by 3D printing in clear resin or by machining polycarbonate; injection moulded only at hundreds of units, because the tool costs thousands.
9. **Test and provisioning.** A fixture that powers each board through its pads, flashes it and runs the self-test, so a batch can be checked in minutes.
10. **Paperwork.** Electromagnetic compliance, battery shipping rules, RoHS. Priced below.

## Two questions to answer before the real card

**Power.** The 3 mm card assumed a CR2016 coin cell, because a ceremony is a few seconds of compute and e-paper draws only when it changes. But a CR2016 delivers a few milliamps continuously and short pulses of perhaps twenty; an ESP32-S3 at full clock draws tens of milliamps for the seconds the ceremony takes, and the panel draws about twenty for two seconds per refresh. The numbers may still work with a slower clock and a capacitor to cover the peaks, or they may not. Card-format products with displays use thin rechargeable pouch cells, half a millimetre to a millimetre and a half thick, charged through the same pads the dock uses. The measurement that decides this is one afternoon with a current meter on the Nano during a ceremony and a refresh. Until it is done, the battery, and with it the thickness, is open.

**Chip.** The product is designed around the Baochip-1x, which is why its tier is what it is: a hardware key store, on-chip memory, no radio. Those boards ship in December 2026, and the card cannot be laid out until the package height and the power draw are in hand. The ESP32-S3 is what we have now, and it has two problems for the card: the certified module is 3.2 mm tall on its own, and a bare chip needs its own flash, a crystal and a four-layer board. So the ESP32-S3 is the chip for the learning board, and the Baochip is the chip for the card, unless December brings a surprise.

## Stage 1: the meetup badge

A board we can design now, with low risk, that is a complete badge even if it is not a thin one. About 8 mm thick, 54 × 86 mm, two layers.

- The ESP32-S3-WROOM-1 module: certified, in every assembler's library, the same chip the firmware runs on today.
- The raw 1.54" panel, the GDEH0154D67 from Good Display, on its 24-pin flex connector, with the boost circuit from the open reference schematics. The same panel as the Waveshare module, five to seven dollars.
- Two tactile switches, a CR2032 holder for the prototype, a USB-C for flashing and the dock.
- Four gold pads on the face at the contacts end, four spring contacts underneath, four magnet pockets in the tray: the pile mechanics, so they get tested on real cards before the thin one is designed.
- A lanyard slot, and a clear tray printed in resin.

Cost for ten: about $40 to $60 a badge, plus the mechanical prototypes. Time: a week to design, three weeks per spin, two spins. I can draft the schematic, the bill of materials and a first layout in KiCad; you review, order and build the first one on the bench. What it buys: a meetup with ten real badges, the pile ritual tested with magnets and pads, and the whole process learned on something forgiving.

## Stage 2: the card

The product. Designed once the power and chip questions are answered, around the Baochip-1x, on a four-layer board the size of a credit card, with the e-paper's glass as the top surface, dome buttons under a sheet, a thin cell, the pads and magnets from Stage 1. This is a job for a contract hardware engineer and a mechanical designer: a few weeks of work and a five-figure fee, with three spins planned. Small batches of the tray are machined or cast; a mould is justified at five hundred units.

What the unit costs at five hundred: a bill of materials around $25 to $35, so a $60 to $90 badge. The e-paper and the chip are most of it.

## Money and paperwork

- **Prototypes.** Stage 1: under $1,000 all in. Stage 2: $15,000 to $40,000 for the engineering, plus $3,000 to $15,000 for a tool if moulded.
- **Compliance.** A board with a radio chip on it needs radio certification only if the radio transmits; ours never does, but a bare ESP32-S3 still needs emissions testing as an ordinary electronic device, and a module carries much of that already. The Baochip has no radio, which is simpler and cheaper. Budget $5,000 to $15,000 for testing before anything is sold in volume. Hobby quantities handed out at a meetup are a different matter from a product.
- **Batteries.** Coin cells and pouch cells both have shipping rules, and a pouch cell needs a protection circuit on the board.
- **Fulfilment.** Crowd Supply does campaigns, manufacturing introductions and warehousing for exactly this kind of open hardware, and the Baochip itself ships through them.

## In what order

1. Finish the firmware on the Nano and run the first ceremony with the laptop as the pile. In progress.
2. Measure power during a ceremony and a refresh. One afternoon. Decides the battery and the thickness.
3. Design and order the meetup badge. Ten units for the first meetup, the pile mechanics tested for real.
4. December: Dabao boards arrive. Port the firmware; measure the Baochip's power and height.
5. Brief a hardware engineer for the card with the measurements, the Stage 1 lessons and the benchmark from the DIY guide. Three spins.
6. Crowd Supply for the first run.

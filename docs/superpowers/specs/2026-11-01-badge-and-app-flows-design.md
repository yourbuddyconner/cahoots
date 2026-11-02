# Badge and app flows, from the first real tap

Design, 2026-11-01. Follows the [badge and phone design](2026-10-31-badge-and-phone-design.md),
which built pairing by tap and card circles, and the first real pairing on hardware that same
day. Conner asked for the badge's and the app's flows to be redesigned from first principles
around how the badge is actually used, and chose to have the design drafted without a round
of questions. Choices that were mine are marked as calls.

## 1. What the first real tap taught

- **A chip the badge powers never takes its antenna.** The SE051 picks one interface as it
  boots and keeps it until it loses power. A powered chip boots onto the wire. In 308 boots
  with a phone's field on the antenna, none took the antenna. The only way a phone reaches the
  chip is for the chip to be unpowered, so that the phone's field is what boots it. On the
  bench that is a PNP transistor on the Click board's 3.3 V, driven from GP3. On the V1 board
  it has to be a switch on the chip's supply. On the V2 card it is the natural state, since a
  card has no supply while a phone is held to it.
- **Field power is enough for everything a tap does.** Select, reads, and flash writes of 16,
  64 and 160 bytes all succeed on the phone's field through the Click board's small loop, in
  37 to 55 ms each. The hungriest work the chip does, programming flash, is fine.
- **Face ID cannot happen during a tap.** The phone lies on the badge, the camera sees the
  table. Every proof of presence has to come before the NFC sheet opens.
- **The badge's pages are the cost.** Pairing and a chat key worked. Hello words did not get
  tried, because reaching them meant opening a page on the badge, a seat under People on the
  phone, and holding the phone at the right moment. Each circle on the badge has five pages,
  the list of circles is one of them, and the phone's pages are another level under that.
  Scrolling and paging is where the minute goes.

## 2. How the badge is used

- The badge lives at home, possibly in a safe. It comes out when the phone needs something
  from it, and that visit has to be over in thirty seconds to a minute, buttons included.
- A card may travel, for ceremonies and for exchanging keys with other people, and may plug
  into a laptop. It is not the thing you reach for several times a day.
- The phone is what you carry. Day to day the phone should need nothing from the badge.

So the design question is not "how does the badge show every circle" but "how little does the
badge have to be involved, and how short is each visit".

## 3. The idea: a tap-ready badge and a phone with a pocket

**The chip is off unless the badge is talking to it, and the antenna is off unless the owner
opens it.** At rest, on every screen that is not in the middle of a conversation with the
chip, the badge cuts the chip's power. That is also the lowest power state the badge has. The
antenna is disconnected at rest too, by a switch the badge controls, so a badge that is on
is still unreachable by a tap until its owner opens a tap window: one press on the home
screen, and the badge says "ready for a tap" for two minutes or until the next press. (Call:
Conner's. The chip's power is not the only gate, the antenna is one as well, and the owner
holds it.) The pairing page opens the window by itself. The only pages that hold the chip
powered are the moments the badge itself needs it: keeping a circle, reading the pairing
inbox, installing a phone's key, unpairing, forgetting, signing, and badge-only hello words.

Until a board has an antenna switch of its own, the gate is the chip's power, which gives the
same result: a chip the badge powers is deaf to a phone. So at rest the badge keeps the chip
powered, in the chip's deep power-down that costs microamps, and cuts the power only inside
a tap window. The bench Click board's antenna switch stays on and is not part of anyone's
steps. (Conner's call, 2026-11-01: the bench is not real life, and the firmware works around
it rather than asking a person to flip a switch.) The V1 board adds an antenna switch in
software as a second gate when it comes, with no change to the flows.

**One tap fetches everything the phone will need for a while.** A tap is not "the key for
this circle". It is "fill the pocket": every card circle's current chat key, the bumps the
badge signed since the last tap (posted to the relay by the phone afterwards), and a batch of
hello words for every seat in every card circle. Hello words are HMACs the chip computes per
seat and counter, so the phone asks for the next few counters in the same session and keeps
them. The phone then shows hello words at once, like a software circle does, and needs the
badge again only when a circle resets or the batch runs low.

**The badge's buttons are for consent, nothing else.** Keep a circle, pair a phone, unpair,
forget, and the badge-only hello words for circles made of badges. Reading is not a button.

## 4. The flows, with their budgets

Each flow starts with the badge off and ends with it back in the drawer. Boot is a few
seconds of e-paper.

**Rotate keys after a reset, or top up hello words.** Badge on. One press opens the tap
window. Phone: Card, "Tap the card", Face ID, hold. Done. One badge press, twenty seconds.

**Hello words.** Phone only, from the pocket, exactly as in a software circle. When the
pocket is down to its last few, the Card screen and the seat's words say "tap the card soon".
Zero badge visits in the common case.

**Pair a phone.** Badge: A+B for the menu, "pair a phone", A+B. The page opens the tap
window. Phone: Settings, "Pair a card", a name, Face ID, hold. Phone says "Now
press both on the badge". Badge: A+B, the name and six words appear. Check, A+B. Badge says
"paired, hold the phone again" and goes back to its home screen with the tap window still
open. Phone: "Hold the phone again", hold. Done. About a minute, four badge presses.

**Unpair.** Badge: A+B, "phone", A+B. Done. The next phone that taps posts the bumps.

**New circle on badges.** Unchanged: the pile, or the laptop bridge for a circle a phone
hosts. The badge's keep prompt is a consent press.

**Badge-only hello words.** For a circle whose members are all badges: menu, the circle,
"hello". Still there, two levels down, because it is the rare case now.

## 5. The badge's screens

**Home.** The badge's name and face, the circles it holds as a short list of names, one line
of status, and the footer "tap  A+B  menu": the left button opens or closes the tap window,
A+B opens the menu. The status line says one of: "closed to taps", "open for a tap, 1:48
left", "a phone is paired: NAME", "no chip", or the current problem. Nothing to page through.
A tap works here while the window is open.

**Menu.** One list, A+B to choose, the side buttons to move: "pair a phone" or "phone: NAME",
"new circle", then each circle by name, then "back". Two levels deep at most anywhere.

**Circle.** One page per circle: the name, the seat, how many members, and three choices in
the footer and the list: "hello" (badge-only words), "key as code" (the existing export), and
"forget" (with its confirmation). The "circles" page that today lives inside each circle is
gone. Its job is the home screen.

**Pair a phone.** The page says "hold the phone to the chip, then press both". A+B reads the
inbox and shows the name and the six words with "A+B pair, side buttons decline". After A+B
it shows "paired with NAME, hold the phone again" for a few seconds and returns home, where a
tap works. Side buttons anywhere go back and close the pairing.

**Phone.** The paired phone's name, "unpair ends its taps now", "A+B unpair". No more "hold
the phone to the chip for its taps" here, because the home screen's tap window is where taps
happen.

**Notes.** Short messages after an action stay for a few seconds and return home by
themselves. No note needs a press to dismiss. (Call: five seconds, or any press.)

What disappears: the per-circle "circles" page, the five-page carousel, and the idea that a
phone's taps need a page of their own. What they need is the owner's press.

## 6. The app's screens

**Card.** One screen, reached from Settings and from any banner. It shows the badge's name
and face, "last tap: when", and a plain list of what a tap would do now: "2 circles need a
key", "3 bumps to post", "hello words: 6 left for Ada, 8 for Ben". One button: "Tap the
card". Face ID first, then the sheet. When nothing is pending it says "up to date" and the
button still works. Unpairing from the phone's side ("Forget this card") stays here.

**Circles and chat.** A card circle looks like any circle. When its key is missing the
composer is replaced by one banner, "Tap the card", which opens the Card screen's tap. The
banner names nothing technical.

**People and hello words.** From the pocket, at once. The seat's page shows the words and the
hello number as it does for software circles, plus "N hellos left from your card" when fewer
than three remain. "Next hello" moves to the next stored pair.

**Pairing.** As today, with two changes: Face ID before the sheet (built 2026-11-01), and the
words step says "Press both on the badge to see its words. If they match, press both again.
Then hold the phone to the card." The second tap no longer needs a badge page.

**Words.** The app says "tap the card" for every visit to the badge, never "catch up" or
"fetch keys". The badge says "hold the phone to the chip". One verb each.

## 7. The chip and the phone's pocket

- **Hello words in batches.** The phone asks the chip, inside the slot's session, for the
  words of counters n to n+K-1 for both speakers of each seat it does not yet hold, and stores
  them with the circle. (Call: K is 8. Three seats are 48 HMACs, about three seconds on the
  bench. A tap with many circles is still under fifteen seconds. The Card screen shows the
  time as a progress line.) The chip's policy for hello objects does not change: the phone
  slot may MAC with them, and nothing else.
- **What a thief gets.** A stolen phone already holds the chat keys and is a member until a
  bump. Stored hello words add the next eight hellos per seat. Unpairing on the badge and the
  bump that follows do not end them, since hello words do not depend on the chat generation.
  The remedy is for the other members to move their hello number with the badge's seat past
  the next eight, and to treat the thief's words as spent. The pocket lives in the same
  protected files as the chat keys.
- **Chat keys and bumps.** Unchanged from the badge and phone design: generation keys come
  from the root by tap, bumps the badge signed wait in the public objects until a phone posts
  them.
- **Pending at rest.** Nothing new is written to the chip for the pocket. The chip stays
  readable only by the badge and the paired phone.

## 8. Security of a tap-ready badge

Cutting the chip's power at rest was the original design's intent (the pin low at rest). It
only now works for a badge that is on: its chip is powered, and so deaf to a phone, outside a
window the owner opened. A badge that is off is not deaf. The phone's field starts an
unpowered chip: on 2026-10-31 an iPhone reached the applet with the Pico off. So a tap reaches
the chip whenever the badge is off or its window is open, and it is worth stating plainly what
such a tap gets.

- **A stranger's tap** still gets nothing it can use: no MAC, no signature, no read of an
  inbox or a public object. The refusals script proves this on the real applet.
- **Planting.** Any tap may make an object at an empty id, including one nobody can delete.
  The badge never adopts such objects, but each one burns the slot whose id it took. The
  badge should leave no id for a stranger to take. (Call: at first boot the badge makes a
  placeholder, owned by it, at every id of every circle slot it could ever use. On the bench
  SE051 that costs memory for sixteen slots' worth of small objects and lowers the circles
  that fit. The builder measures it, and if it costs more than one circle, the slot count
  drops to eight.)
- **In a safe, or just on.** A badge that is on keeps its chip powered and so deaf to taps
  until its owner presses. A badge in a drawer is off, and a badge that is off can be tapped,
  since the phone's field starts an unpowered chip. The planting surface is any moment the
  badge is off or its window is open. A power cut while the badge has an id empty for a
  moment, between deleting the placeholder there and writing what replaces it, leaves that id
  open until the next boot puts a placeholder back. On V1 the antenna switch closes the off
  badge's surface, since its coil is shorted unless the MCU opens it.
- **Face ID before every tap.** The phone's proof of presence comes before the sheet, for
  pairing and for every "Tap the card".

## 9. Hardware

- **The chip's supply is switched.** The V1 board gets a load switch or PNP transistor on the
  chip's supply, driven by the MCU, and the chip's enable pin stays wired. The bench Click
  board has the S8550 and three 330 Ω in series as the base resistor, with ENA SEL on EN.
- **An antenna switch in software, on V1.** A second gate beside the chip's power, and the
  only one for a badge that is off: a switch across the coil that shorts it. The coil must be
  shorted when the MCU is unpowered, since the phone's field starts an unpowered chip, so the
  switch is shorted by default and the MCU opens it for a tap window. (Call: a switch across
  the coil that conducts with no drive, such as a depletion-mode MOSFET. An RF switch in
  series works too if it is open with no drive, and the builder may prefer it if the short
  detunes the coil badly when open.) Nothing in the flows depends on it, and no step asks a
  person to flip a switch.
- **The V2 card** is RF powered during taps by nature. Its antenna must deliver field power
  for flash writes at the distances a phone is held. The Click board's small loop does, so a
  card-sized antenna will.
- **No radio.** Conner raised Bluetooth for a badge that is wireless to the phone. The badge
  is deliberately a board with no radio, and a badge that relays the chip over Bluetooth would
  be a powered badge, which is exactly the state in which the chip is deaf to a phone. The
  relay would have to go through the MCU, which puts the MCU between the phone and the chip
  all day. (Call: no radio. A tap-ready badge and a phone with a pocket make the badge a
  monthly visit, which removes most of what Bluetooth would have bought. For a laptop the
  existing USB pass-through is the link.)

## 10. What this changes in code

- **Badge crate.** The rest state cuts the chip's power on every idle screen, through
  `tap_begin` and `tap_end`, and every conversation ends in that state. A tap window on the
  home screen, opened by the left button and by the pairing page, that turns the antenna on
  for two minutes and shows the time left. A home screen, a single menu, a one-page circle,
  and notes that return home on their own. The per-circle "circles" page goes.
- **Firmware.** The rail pin is already there. The rest state at boot is power cut after the
  self-test is powered and deaf, as today. The bench window and the pin probes stay as bench
  tools.
- **Phone crate.** A pocket per card circle: hello words by seat and counter, filled by the
  tap in batches, used by `hello_words` and `next_hello` before any tap. One `tap_card`
  operation that fetches keys, posts bumps and tops up words, and reports what it did. A
  `card_status` that lists what a tap would do.
- **iOS.** The Card screen, the single "Tap the card" banner in chat, hello words from the
  pocket, the wording changes, and Face ID before every tap.
- **Chip.** Nothing. Placeholders for every slot's ids at first boot, under the policies the
  badge already uses.
- **Docs and tests.** The badge walk, the person's steps, the spec, and tests for the rest
  state (power cut on every idle screen, power on only inside a conversation), the pocket
  (words served without a tap, the tap tops up, a reset empties), and the menu depth.

## 11. Calls I made

- Taps reach the chip only inside a two minute tap window the owner opens with one press.
  Until V1 has its antenna switch in software, the gate is the chip's power: powered and deaf
  at rest, cut for the window. (Conner's calls: the owner's gate, and the firmware working
  around the bench rather than steps about a switch.)
- Hello words are fetched eight at a time per seat and kept on the phone.
- Placeholders for every circle slot's ids at first boot, with the slot count dropping to
  eight if sixteen cost more than a circle of memory.
- No radio. USB stays the laptop's link.
- Notes return home after five seconds or any press.
- One verb each: the app says "tap the card", the badge says "hold the phone to the chip".

**As built, 2026-11-01.** Where the build differs from the calls above:

- The gate is the chip's power. No board has an antenna switch in software yet, so the left
  button on home, the pairing page and the note after a pairing cut the chip's power, and
  every conversation of the badge's own gives it back. A badge that is off has no gate: the
  phone's field starts the chip, as an iPhone did with the Pico off on 2026-10-31. Home's status line says "closed to
  taps", "open for a tap, 2 min" or "open for a tap, 1 min". Home's title is "badge" over the
  circles' names. The menu lists the circles first, then "new circle", "pair a phone",
  "phone" while one is paired, and "back". A circle's page is a list of "words", "key as
  code", "grants", "forget this circle" and "back".
- Placeholders sit at the public id of every circle slot, and nowhere else. On the bench SE051
  a placeholder takes 160 bytes. Sixteen slots with all their ids, 144 objects, left room for
  two circles, and eight slots for three. With the public ids alone, the chip holds five
  circles of three seats where it held six, so the placeholders cost one circle. The hello ids
  of a free slot stay empty, a known limit: a tap while the badge is off, or inside an open
  window, can plant an object there and so burn that slot. A power cut while a keep or a
  forget has a public id empty for a moment leaves that id open the same way, until the next
  boot puts a placeholder back.
- A phone keeps one badge. A second badge's pairing is refused at its second tap with "Forget
  this card first."
- The Card screen lists a seat's hello words as low at three or fewer, not fewer than three.
- A tap's report counts the bumps it passed on, not the bumps it posted: a bump the relay
  already held, or one dropped as stale, counts too. The app says "passed on".
- A circle of three seats is 32 hello MACs at a tap, not 48: two other seats, eight numbers,
  and two speakers each. On the bench, through the laptop's wire, the second tap of a pairing
  kept three circles in 6.2 seconds: 83 commands, 32 of them hello MACs, 11 root MACs and 31
  reads. Over NFC a command took 37 to 55 ms, so a chip full of circles stays well under a
  tap's 60 seconds, and no limit on the MACs of one tap is needed yet.
- A seat whose hello object refuses the slot or is not there (`6985`, `6986` or `6A82` at the
  seat's first MAC in a tap) gets no words from the card, and the tap goes on. The phone marks
  the seat: its hello words say "This seat has no hello words from the card." and ask for no
  tap, and the Card screen does not count it as running low. A later tap that gets its words
  takes the mark off. A circle the badge kept before it made hello objects (before the evening
  of 2026-10-31) has none: the bench's two oldest circles hold placeholders at their hello ids.

## 12. Left to the builder

The home screen's layout on the 2.13 inch e-paper, how the menu scrolls past eight entries,
the exact Card screen layout, the progress line during a long tap, and the placeholder
memory measurement.

# Badge screens: design

*Written 2026-10-26 after the first ceremony on the Nano. Conner asked for the hardware-wallet way of steering with two buttons, and for the screens to be drawn once so every board shows the same thing. This reopens the display decision from the custodian design: the badge now renders into a framebuffer and a board only blits it.*

## Controls

Two buttons, three gestures, as on a Ledger Nano:

| Gesture | Meaning |
|---|---|
| Left (A) | previous page |
| Right (B) | next page |
| Both within 80 ms of each other | select, or allow |

On a prompt, Both allows and a single press of either button refuses. There are no modes. A board reports gestures through one `Board::press() -> Press` call with `Press::{None, Left, Right, Both}`, with debounce and the chord window handled on the board (a press is reported only after the state has been stable for 30 ms, and a Left or Right is held back for the chord window so that Both can be reported instead).

## The frame

`cahoots_hal::Frame` is a 200 by 200 one-bit image, row-major, 25 bytes per row, 1 = black. `Display` has one method, `draw(&mut self, frame: &Frame)`, replacing `show_words`, `show_qr` and `show_status`. A board's `draw` is a full refresh. The badge crate renders every screen with `embedded-graphics` (no_std) into a `Frame`; the host writes the frame to a PNG; the Nano copies it into the panel's buffer.

The ring driver's `run` helper no longer takes a `Display`; it takes `status: impl FnMut(&str)`.

## Screens

Every screen has the same skeleton: the circle name and the tier label in a header rule, the content, and arrow glyphs `<` and `>` at the left and right edges of the middle row when there is a page on that side. Text is the 10 by 20 font for the main content and 8 by 13 for secondary lines. Selection or emphasis is drawn inverted (white on a black box).

| Screen | Content | Left / Right | Both |
|---|---|---|---|
| Self-test | the lines from `boot`; shown for 2.5 s or until Both | | continue |
| Last panic | the recorded message | | continue |
| Pile | "stack the cards" and the seat once known | | start the count (top card) |
| Ceremony | "commits x/n", "reveals y/n" as a two-line counter | | |
| Words | the six words in two columns, "say them aloud" | | keep, during the keep prompt |
| Keep prompt | "keep this circle?" with Both = keep, either = discard | discard | keep |
| Menu, Words page | the six words | pages | redraw |
| Menu, Key page | "key as code", "Both to spend 32 bytes" | pages | spend and show the Code screen |
| Code | the 32 bytes as 4 rows of 8 hex pairs, inverted title "code" | back to the Key page | |
| Menu, Serve page | "serve a laptop", a "listening" line once a peer is present | pages | start serving |
| Consent prompt | "spend N bytes for <circle>?", "Both to allow" | refuse | allow |
| Status | one or two lines, for errors and transient notes | | |

Pages are a ring: Words, Key, Serve, then Words again. The page indicator "1/3" sits under the header.

## Host and simulator

`HostBoard` reads `buttons.txt` lines `L`, `R` and `B` (Both). `PngDisplay` writes the frame to `display.png` and the text the badge drew is no longer available as a string, so tests read a `screen.txt` that the badge's renderer also emits: the badge keeps the last screen's lines as text alongside the frame (`Screen { lines: Vec<String>, frame: Frame }`), and boards get the text through `Board::note_screen(&[String])` with a default no-op. The fake board records those lines in place of the old status strings.

## Out of scope

A QR encoder for the code screen (the hex rows stay), the driver fork that brings quick partial refreshes back, icons.

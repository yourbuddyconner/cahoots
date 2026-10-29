# Cahoots, the faces

A **cahoot** is the face a badge wears. It is a one-bit pixel portrait, 24 cells on a side, drawn from the badge's own random bytes, and it stays the same for as long as the badge does. It is what a badge shows when nothing else is going on, and it is how the people around a table come to know a badge at a glance.

![Twelve cahoots](diagrams/cahoots.svg)

## What one is made of

Every cahoot is put together from the same small set of parts, one from each layer, chosen by a hash of the badge's randomness:

| Layer | Choices |
|---|---|
| Head | square, round or slim, with plain or dotted skin |
| Hair | bald, short, mohawk, long or wild |
| Eyes | plain, shades, 3D glasses or a monocle |
| Mouth | plain, a smile, a moustache, a beard, a pipe or a cigarette |
| Accessory | none, an earring or a chain |
| Hat | none, a beanie, a cap, a fedora, a top hat, a bandana or a crown |

That is 15,120 different faces. A hat clips the hair under its brim, and every feature clears a ring of blank paper around itself, so a face reads cleanly on an e-paper panel the size of a postage stamp. The drawing uses two values only, ink and paper, because that is all the panel has.

The parts are weighted. A crown, a monocle or a pipe is rarer than a cap or a smile, so some faces are a little more to show off than others. The weights are in the generator; nobody at Cahoots picks a face for anyone.

## What it is for

- **On the badge.** The face is the badge's idle screen. Pick a badge up and you see whose it is before you see anything else.
- **In the room.** When circles are made at a meetup, the face is how you match the badge in someone's hand to the name they just told you.
- **In the app.** A contact card carries the face beside the name and the handles, and a circle's roster shows every member's face. The browser app draws the same face from the same bytes, so a member's face is the same on every screen that shows them.

A cahoot proves nothing. It is a picture, drawn from randomness that only the badge saw, and two badges could in principle draw the same one. Membership is the signed manifest and the seat keys; the face is how people recognise each other, the way a face is.

## Where it stands

The faces exist, and the generator that draws them is in the repository. The badge does not draw its own yet: that is the idle screen in the [roadmap](plan.md#hardware) (M4i), which comes after the chip work. Until then the face a badge will get is already fixed by its randomness; it just is not on the panel.

## Draw your own

The generator is `site/art/gen.py`. It writes the twelve above and a few other pieces of the site's art:

```sh
python3 site/art/gen.py
```

In Python, `gen.punk(seed)` returns one face as SVG for any bytes you give it, and `gen.punk_traits(seed)` tells you which parts it chose. The seed is hashed with SHA-256, so the same bytes always give the same face, and nothing about the bytes shows in the picture.

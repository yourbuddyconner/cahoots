# Cahoots launch video: directed brief (rev A)

GOAL: A viewer on X knows that Cahoots badges make a shared encryption secret when you touch them together, and opens cahoots.network.

OUTPUT: three.js scene in `video/src/scene.ts` rendered frame by frame through Playwright and Chrome, 1920 x 1080, H.264 High, yuv420p, CRF 16, AAC 192k, faststart. Output `video/out/cahoots-launch.mp4`.

TIME: 52.28 s x 30 fps = 1568 frames. Loop: no. Music "sunsides - chill trap promo" (152 BPM, steady; 147.7 in the intro), spliced in `video/splice.py`: 0 to 41.46 s (intro and first main section) then 135.02 s to the end (outro tail, faded over the last 2 s). Beat grid: detected beats (`video/src/beats.json`), snapped per beat. Chapters start on section changes and bars: 0, 6.6, 13.0, 22.5, 30.0, 36.0, 41.5.

LOOK: base #0a0a0b, text #f4f4f5, dim text #a1a1aa, accent amber #e5a33a, e-paper #e6e2d8 with ink #17191b. Headlines Inter Tight 600, 84 px, tracking -0.035em, left-aligned on a 12-column grid at x = 160 px. Part labels and use items JetBrains Mono 500, 36 px, uppercase, tracking 0.08em, amber. Address JetBrains Mono 400, 44 px. The badge is the site's procedural model (`site/badge.js`) under the site's room environment and the same amber rim light. The ticker of five-digit groups from the site runs along the bottom edge during the open and the close. Identity on screen: the badge's e-paper reads "sf-meetup", "TRNG · T4", and the six words sunset valve present raven curtain pudding (demo values from the site).

MOTION (tokens in `video/src/motion.ts`; no inline numbers in scenes):
- Easing: enter ease-out-expo (0.16, 1, 0.3, 1); move ease-in-out-quart (0.76, 0, 0.24, 1); exit ease-in-expo (0.7, 0, 0.84, 0).
- Exits 50% of entrances. Text 18 f reveal, 9 f exit. Panels 23 f, 12 f.
- Springs by damping ratio: badge drop ζ 0.8 at 1.1 Hz; snap ζ 0.6 at 2.2 Hz (one visible settle, the magnet); mark ζ 0.7 at 1.6 Hz. Text never overshoots.
- Stagger: words 3 f, items 2 f.
- One hero per shot. Secondaries +5 f after the hero at 0.6 of its travel. Ambient (the badge's slow turn) under 5% of hero travel and never on text being read.
- Holds ≥ 0.3 s/word + 0.5 s.
- Visual hits 1 frame before the beat; sound on the beat.

SHOTS (one hero each; frames from `timeline.ts`, the audit prints them):
- 00 open (0 to 6.6): the mark lands on the first beat (ζ 0.7), the wordmark reveals word-wise beside it, the ticker starts at the section change (3.4). Hands off by the wordmark exiting as the badge drops.
- 01 badge (6.6 to 13.0): the assembled badge drops from above and lands on the beat (ζ 0.8), then turns 180° at 10.2 to show the back. Headline "This badge makes an encryption secret." Hands off by the explode starting on the bar.
- 02 explode (13.0 to 22.5): the parts separate over two bars (ease-in-out-quart), one part label per two beats as its part clears the stack. Headline "Open hardware. Every part is public." Hands off by the parts assembling at 21.0 as the labels leave.
- 03 touch (22.5 to 30.0): a second badge slides in from the right and snaps onto the first (ζ 0.6), landing on the beat; an amber pulse crosses the pins, then returns. Headline "Touch two together. They make one secret." Hands off by the camera push.
- 04 words (30.0 to 36.0): the camera pushes to the two screens; both e-paper panels refresh (flip) from "ceremony · 2 of 2" to the same six words on the beat. Headline "Both screens show the same six words." Hands off by the badges receding.
- 05 uses (36.0 to 41.5): the pair pulls back and dims to a third; three uses enter one per bar beside the headline "Protect your VPN, your files, and your chat." Hands off by the headline leaving before the close enters.
- 06 close (41.5 to 52.28): "Get in cahoots." then the address; the pair settles to face the camera at 46.3; the scene dims at 49.4; fade to black over the last 20 frames.

BANNED: the skill's starting list, plus: lens flare or bloom on the polycarbonate; particles; any text over the e-paper; a scale bump of the whole frame on beats; the badge spinning while a headline is read; fade-and-slide-up on every line; headlines that are slogans (every headline above is a statement someone would say); the amber used for anything but the accent, the labels, and the pulse.

QA:
1. Numeric audit: holds, hero overlap, secondary delay/travel, beat alignment, duplicate beat frames, slots, chapter contiguity, contrast ≥ 4.5:1, flashes ≤ 3/s.
2. Contact sheet per chapter; stills at every hit frame and the frame after.
3. Notes with frame numbers, then fix, then re-render only what changed. Log every note in `docs/receipts/2026-10-21-launch.md`.
4. Sound: hits 3 to 6 dB over the bed (200 Hz to 6 kHz, first 60 ms); −14 LUFS; ≤ −1 dBTP and 0-sample sync on the decoded MP4.

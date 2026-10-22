# Cahoots launch video: directed brief (rev B)

GOAL: A viewer on X comes away with "I want one": a badge that makes its own random numbers, and one meeting with it gives a whole network its keys with no server and no CA. Then they open cahoots.network.

Rev B (2026-10-21): the six check words were taking a full chapter for a verification step. Cut to a beat inside the touch chapter. The payoff is now a 14 s network chapter.

Rev E (2026-10-21, late): three lines rewritten. "This one rolls its own" explained nothing; the explode headline is now "This card makes its own random numbers." with "You watch it happen." landing under it after the last part label. "They share a secret nobody else has" was jargon; the touch headline is now "Stack two. The numbers are made between them." "Sight unseen." was a fragment with nothing to hang on; the badge headline now carries a second sentence that lands under it four seconds later, "You weren't there.", and both leave together. A text cue can hold back its later words with `pause`.

Rev D (2026-10-21, late): the word reveals were breathing too long. Text reveal 12 f instead of 18, words 2 f apart instead of 3, exits 6 f. The time that freed took two more lines: "Sight unseen." after the badge headline, and "Nothing was sent. Nothing to decrypt." at the top of the close, before "Get in cahoots." Holds measured from the last word landing. The audit now lives in `video/audit.mts`.

Rev C (2026-10-21, evening): the badge became the 3 mm card that stacks (`docs/diy-badge.md`, "The manufactured badge"). Same timeline, same frames, same sound; the touch chapter is restaged as a pile: the second card comes down from above and lands on the first, the pulse runs along the pads at the near edge, then the top card lifts off and both screens show the words. Part labels follow the new parts. Rendering moved to an opaque canvas with ambient occlusion and the page colour drawn in-scene, because thin clear plastic does not render on a transparent canvas.

OUTPUT: three.js scene in `video/src/scene.ts` rendered frame by frame through Playwright and Chrome, 1920 x 1080, H.264 High, yuv420p, CRF 16, AAC 192k, faststart. Output `video/out/cahoots-launch.mp4`.

TIME: 52.28 s x 30 fps = 1568 frames. Loop: no. Music "sunsides - chill trap promo" (152 BPM, steady; 147.7 in the intro), spliced in `video/splice.py`: 0 to 41.46 s (intro and first main section) then 135.02 s to the end (outro tail, faded over the last 2 s). Beat grid: detected beats (`video/src/beats.json`), snapped per beat. Chapters start on section changes and bars: 0, 6.6, 14.5, 20.2, 27.1, 41.5.

LOOK: base #0a0a0b, text #f4f4f5, dim text #a1a1aa, accent amber #e5a33a, e-paper #e6e2d8 with ink #17191b. Headlines Inter Tight 600, 84 px, tracking -0.035em, left-aligned on a 12-column grid at x = 160 px. Part labels and use items JetBrains Mono 500, 36 px, uppercase, tracking 0.08em, amber. Address JetBrains Mono 400, 44 px. The badge is the site's procedural model (`site/badge.js`): the 3 mm clear card with its lanyard slot, e-paper, coin cell and the magnets and pads of the stacking link, under the site's studio environment, ambient occlusion and the same amber rim light. The ticker of five-digit groups from the site runs along the bottom edge during the open and the close. Identity on screen: the badge's e-paper reads "sf-meetup", "TRNG · T4", and the six words sunset valve present raven curtain pudding (demo values from the site).

MOTION (tokens in `video/src/motion.ts`; no inline numbers in scenes):
- Easing: enter ease-out-expo (0.16, 1, 0.3, 1); move ease-in-out-quart (0.76, 0, 0.24, 1); exit ease-in-expo (0.7, 0, 0.84, 0).
- Exits 50% of entrances. Text 12 f reveal, 6 f exit. Panels 23 f, 12 f.
- Springs by damping ratio: badge drop ζ 0.8 at 1.1 Hz; snap ζ 0.6 at 2.2 Hz (one visible settle, the magnet); mark ζ 0.7 at 1.6 Hz. Text never overshoots.
- Stagger: words 2 f, items 2 f.
- One hero per shot. Secondaries +5 f after the hero at 0.6 of its travel. Ambient (the badge's slow turn) under 5% of hero travel and never on text being read.
- Holds ≥ 0.3 s/word + 0.5 s, counted from the moment the last word has landed.
- Visual hits 1 frame before the beat; sound on the beat.

SHOTS (one hero each; frames from `timeline.ts`, the audit prints them):
- 00 open (0 to 6.6): the logo is on screen from frame 0 (the thumbnail), the ticker starts at the section change (3.4), the logo leaves as the badge drops.
- 01 badge (6.6 to 14.5): the assembled badge drops and lands on the beat (ζ 0.8), tilts to camera at 10.2. Headline "Every key you trust came from someone else's random numbers." from 7.0, with "You weren't there." landing under it at 11.4; both leave at 14.0. Hands off by the explode starting on the bar.
- 02 explode (14.5 to 20.2): parts separate over two bars, one label per beat: clear card, e-paper, Baochip-1x, magnets and pads, coin cell. Headline "This card makes its own random numbers." from 15.0, with "You watch it happen." landing under it at 17.4; both leave at 19.8. Hands off by the parts assembling at 19.4.
- 03 touch (20.2 to 27.1): a second card comes down and lands on the first (ζ 0.6) on the beat, the magnets taking it the last bit; an amber pulse runs along the pads at the near edge and returns; the camera pushes in while the top card lifts off beside the pile, and both screens refresh to the same six words on the beat. Headline "Stack two. The numbers are made between them." Hands off by the top card settling back onto the pile as it pulls back.
- 04 network (27.1 to 41.5): the payoff. The pile pulls back; an amber link grows from it to one of six devices on every beat (WireGuard peer, mesh radio, laptop, phone, server, second site), each device's ring lighting as its link lands; then keys run down every link at 35.4 and again at 38.6. Headlines "One meeting gives the whole network its keys." then "Keys rotate from the badge for years." over a second line that rolls like a cube on the bar at 37.0 from "No server, no CA." to "No man in the middle." Hands off by the last headline leaving before the close.
- 05 close (41.5 to 52.28): "Nothing was sent. Nothing to decrypt." to 45.0, then "Get in cahoots." at 46.0 and the address at 47.0; the pile settles to camera at 46.3; the scene dims at 49.4; fade to black over the last 20 frames.

BANNED: the skill's starting list, plus: lens flare or bloom on the polycarbonate; particles; any text over the e-paper; a scale bump of the whole frame on beats; the badge spinning while a headline is read; fade-and-slide-up on every line; headlines that are slogans (every headline above is a statement someone would say); the amber used for anything but the accent, the labels, and the pulse.

QA:
1. Numeric audit: holds, hero overlap, secondary delay/travel, beat alignment, duplicate beat frames, slots, chapter contiguity, contrast ≥ 4.5:1, flashes ≤ 3/s.
2. Contact sheet per chapter; stills at every hit frame and the frame after.
3. Notes with frame numbers, then fix, then re-render only what changed. Log every note in `docs/receipts/2026-10-21-launch.md`.
4. Sound: hits 3 to 6 dB over the bed (200 Hz to 6 kHz, first 60 ms); −14 LUFS; ≤ −1 dBTP and 0-sample sync on the decoded MP4.

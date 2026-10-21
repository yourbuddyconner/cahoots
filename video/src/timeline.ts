// The single source of frames for one piece. Scenes render it (`cue(id)`, `move(id)`), the audit checks it
// (<skill-dir>/scripts/audit.mts), and the sound synth scores it (<skill-dir>/scripts/sfx.mts).
// All frames are absolute (frame 0 = first frame of the video).
//
// Copy next to motion.ts and beats.json (from scripts/beats.py), then fill the PROJECT section at the bottom.
// With no music, write `[]` to beats.json: the grid falls back to a 0.5 s beat.
import beatsJson from "./beats.json";
import { DUR, FPS, SECONDARY } from "./motion";

// ───────────── grid (generic) ─────────────
export const SECONDS = 52.28; // the spliced track: intro + first main section + outro tail
export const TOTAL = Math.round(SECONDS * FPS);

/** Detected beats in output seconds. Extend past the last detection by n x period (never accumulate). */
const detected = beatsJson as number[];
const PERIOD = detected.length > 8 ? median(diff(detected.slice(-16))) : 0.5;
export const BEATS_S: number[] = [...detected];
for (let k = 1, last = detected[detected.length - 1] ?? 0; last + k * PERIOD < SECONDS; k++) BEATS_S.push(last + k * PERIOD);
export const BEAT_F = BEATS_S.map((s) => Math.round(s * FPS));

/** Visual hit frame for the beat nearest `sec`: one frame before the beat, so its sound lands on the beat. */
export const hit = (sec: number) => {
  let best = BEAT_F[0];
  for (const b of BEAT_F) if (Math.abs(b - sec * FPS) < Math.abs(best - sec * FPS)) best = b;
  return best - 1;
};
/** Visual hit frames of the next `n` beats strictly after frame `f`, taking every `every`-th beat. */
export const nextBeats = (f: number, n: number, every = 1) =>
  BEAT_F.filter((b) => b - 1 > f).filter((_, i) => i % every === 0).slice(0, n).map((b) => b - 1);
/** First visual hit frame at or after frame `f`. */
export const beatAfter = (f: number) => BEAT_F.find((b) => b - 1 >= f)! - 1;
export const s = (sec: number) => Math.round(sec * FPS);

// ───────────── registries (generic) ─────────────
export type HitKind = "key" | "enter" | "line" | "click" | "check" | "swap" | "chapter" | "flash" | "fly" | "slam" | "whoosh" | "tick";
export interface Hit { f: number; kind: HitKind; note: string; onBeat: boolean }
export interface TextCue { id: string; text: string; in: number; out: number; slot?: string }
export interface Move { id: string; from: number; dur: number; role: "hero" | "secondary"; travel: number; hero?: string }
export const HITS: Hit[] = [];
export const TEXTS: TextCue[] = [];
export const MOVES: Move[] = [];

/** Register a text state shown from `in` (reveal starts) to `out` (exit starts). Same `slot` = same screen position. */
export const text = (id: string, t: string, inF: number, out: number, slot?: string) => (TEXTS.push({ id, text: t, in: inF, out, slot }), inF);
/** Frame when a text that starts leaving at `out` has fully left (word exits stagger 2 f). */
export const goneAt = (t: string, out: number) => out + DUR.textOut + 2 * (t.trim().split(/\s+/).length - 1);
export const hero = (id: string, from: number, dur: number, travel = 1) => (MOVES.push({ id, from, dur, role: "hero", travel }), from);
export const second = (id: string, heroId: string, heroFrom: number, dur: number, travel = SECONDARY.travel) => {
  const from = heroFrom + SECONDARY.delay;
  MOVES.push({ id, from, dur, role: "secondary", travel, hero: heroId });
  return from;
};
/** Register an exit that hands off to a new object as a hero move, 60% done when `next` starts. Returns the exit start. */
export const exitBefore = (id: string, next: number, dur: number = DUR.cardOut) => hero(`${id}.out`, next - Math.ceil(0.6 * dur), dur);
/** Max seconds without a new move, hit, or text (audit rule 11). */
export const DEAD_AIR = 4.5;
export const mark = (f: number, kind: HitKind, note: string, onBeat = true) => (HITS.push({ f, kind, note, onBeat }), f);
const mark_ = mark;
export const cue = (id: string) => { const c = TEXTS.find((x) => x.id === id); if (!c) throw new Error(`no text cue ${id}`); return c; };
export const move = (id: string) => { const m = MOVES.find((x) => x.id === id); if (!m) throw new Error(`no move ${id}`); return m; };

/** Palette the audit checks for contrast: [name, hex, alpha] over `bg`. */
export const PALETTE = { bg: "#0a0a0b", text: [["text", "#f4f4f5", 1], ["dim", "#a1a1aa", 1], ["amber", "#e5a33a", 1]] as [string, string, number][] };

function diff(a: number[]) { return a.slice(1).map((x, i) => x - a[i]); }
function median(a: number[]) { const b = [...a].sort((x, y) => x - y); return b[Math.floor(b.length / 2)]; }

// ───────────── PROJECT: chapters, then one block per chapter ─────────────
const chStart = (sec: number) => (sec === 0 ? 0 : hit(sec) + 1);
export const CH: Record<string, readonly [number, number]> = {
  open: [0, chStart(6.6)],
  badge: [chStart(6.6), chStart(13.0)],
  explode: [chStart(13.0), chStart(22.5)],
  touch: [chStart(22.5), chStart(30.0)],
  words: [chStart(30.0), chStart(36.0)],
  uses: [chStart(36.0), chStart(41.5)],
  close: [chStart(41.5), TOTAL],
};

// OPEN: black. The mark lands, then the wordmark.
export const OPEN = (() => {
  const mark = hero("open.mark", hit(1.0), DUR.panelIn);
  mark_(mark, "tick", "mark lands");
  const word = text("open.word", "Cahoots", hit(1.8), hit(5.6), "head");
  mark_(word, "enter", "wordmark");
  const ticker = hero("open.ticker", hit(3.4), DUR.rowIn);
  mark_(ticker, "tick", "number groups start");
  const out = exitBefore("open", hit(6.6), DUR.panelOut);
  return { mark, word, ticker, out };
})();

// BADGE: the assembled badge drops in and turns.
export const BADGE = (() => {
  const drop = hero("badge.drop", hit(6.6), DUR.panelIn);
  mark_(drop, "slam", "badge lands");
  const head = text("badge.head", "This badge makes an encryption secret.", hit(7.4), hit(12.2), "head");
  const turn = hero("badge.turn", hit(10.2), DUR.lift);
  mark_(turn, "swap", "badge turns over");
  return { drop, head, turn };
})();

// EXPLODE: the parts separate over two bars; a label per part on every other beat.
export const PARTS = ["Clear case", "E-paper screen", "Baochip-1x", "Touch pins", "Battery and USB-C"];
export const EXPLODE = (() => {
  const apart = hero("explode.apart", hit(13.0), DUR.explode);
  mark_(apart, "whoosh", "parts separate");
  const head = text("explode.head", "Open hardware. Every part is public.", hit(13.8), hit(21.4), "head");
  const labels = nextBeats(hit(14.6) - 1, PARTS.length, 2);
  labels.forEach((f, i) => { text(`explode.label${i}`, PARTS[i], f, hit(21.4), `label${i}`); mark_(f, "tick", PARTS[i]); });
  const together = hero("explode.together", hit(21.0), DUR.panelIn);
  mark_(together, "swap", "parts assemble");
  return { apart, head, labels, together };
})();

// TOUCH: a second badge slides in and snaps on; a pulse crosses the pins.
export const TOUCH = (() => {
  const land = hit(23.9);
  const snap = hero("touch.snap", land - DUR.snap, DUR.snap);
  mark_(land, "slam", "badges snap together");
  const head = text("touch.head", "Touch two together. They make one secret.", hit(24.3), hit(29.5), "head");
  const pulse = hero("touch.pulse", hit(25.1), DUR.connector);
  mark_(pulse, "fly", "pulse over the pins");
  const pulse2 = hero("touch.pulse2", hit(26.7), DUR.connector);
  mark_(pulse2, "fly", "pulse back");
  return { snap, land, head, pulse, pulse2 };
})();

// WORDS: push in to the screens; both refresh to the same six words.
export const WORDS = (() => {
  const push = hero("words.push", hit(30.0), DUR.push);
  mark_(push, "swap", "camera push");
  const flip = hero("words.flip", hit(31.2), DUR.flip);
  mark_(flip, "check", "screens show the words");
  const head = text("words.head", "Both screens show the same six words.", hit(31.6), hit(35.6), "head");
  return { push, flip, head };
})();

// USES: the badges pull back; three uses, one per bar.
export const USES_ITEMS = ["WireGuard tunnels", "Encrypted files", "Group chat"];
export const USES = (() => {
  const recede = hero("uses.recede", hit(36.0), DUR.panelIn);
  mark_(recede, "whoosh", "badges recede");
  const head = text("uses.head", "Protect your VPN, your files, and your chat.", hit(36.4), hit(40.7), "head");
  const items = nextBeats(hit(37.2) - 1, USES_ITEMS.length, 2);
  items.forEach((f, i) => { text(`uses.item${i}`, USES_ITEMS[i], f, hit(40.7), `item${i}`); mark_(f, "tick", USES_ITEMS[i]); });
  return { recede, head, items };
})();

// CLOSE: the line, the address, a settle, then black.
export const CLOSE = (() => {
  const title = hero("close.title", hit(41.5), DUR.panelIn);
  mark_(title, "enter", "get in cahoots");
  const head = text("close.head", "Get in cahoots.", CH.close[0], TOTAL - DUR.fade, "head");
  const url = text("close.url", "cahoots.network", hit(43.1), TOTAL - DUR.fade, "url");
  mark_(hit(43.1), "tick", "address");
  const settle = hero("close.settle", hit(46.3), DUR.lift);
  mark_(settle, "tick", "badges settle");
  const dim = hero("close.dim", hit(49.4), DUR.lift);
  mark_(dim, "tick", "scene dims");
  const fade = hero("close.fade", TOTAL - DUR.fade, DUR.fade);
  return { title, head, url, settle, dim, fade };
})();

HITS.sort((a, b) => a.f - b.f);

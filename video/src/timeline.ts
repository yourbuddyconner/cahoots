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
/** A text state. A cube face enters or leaves by the roll (`enter`/`exit` "roll") instead of word by word. */
export interface TextCue { id: string; text: string; in: number; out: number; slot?: string; enter?: "roll"; exit?: "roll" }
export interface Move { id: string; from: number; dur: number; role: "hero" | "secondary"; travel: number; hero?: string }
export const HITS: Hit[] = [];
export const TEXTS: TextCue[] = [];
export const MOVES: Move[] = [];

/** Register a text state shown from `in` (reveal starts) to `out` (exit starts). Same `slot` = same screen position. */
export const text = (id: string, t: string, inF: number, out: number, slot?: string, opts?: { enter?: "roll"; exit?: "roll" }) => (TEXTS.push({ id, text: t, in: inF, out, slot, ...opts }), inF);
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
  badge: [chStart(6.6), chStart(14.5)],
  explode: [chStart(14.5), chStart(20.2)],
  touch: [chStart(20.2), chStart(27.1)],
  network: [chStart(27.1), chStart(41.5)],
  close: [chStart(41.5), TOTAL],
};

// OPEN: the logo is fully on screen from frame 0 (it is the thumbnail), holds, then leaves as the badge drops.
// The scene treats these two as pre-shown: their reveals ran before frame 0.
export const OPEN = (() => {
  const mark = hero("open.mark", 0, DUR.panelIn);
  const word = text("open.word", "Cahoots", 0, hit(5.6), "head");
  const ticker = hero("open.ticker", hit(3.4), DUR.rowIn);
  mark_(ticker, "tick", "number groups start");
  const out = exitBefore("open", hit(6.6), DUR.panelOut);
  return { mark, word, ticker, out };
})();

// BADGE: the problem, then the badge that answers it. The headline is a cube: it rolls to its second half.
export const BADGE = (() => {
  const drop = hero("badge.drop", hit(6.6), DUR.panelIn);
  mark_(drop, "slam", "badge lands");
  const flip = hero("badge.flip", hit(11.4), DUR.flip);
  const head = text("badge.cubeA", "Every key you trust came from someone else's random numbers.", hit(7.0), flip, "head", { exit: "roll" });
  const head2 = text("badge.cubeB", "You weren't there when they were generated.", flip, hit(14.6), "head", { enter: "roll" });
  const turn = hero("badge.turn", hit(10.2), DUR.lift);
  mark_(turn, "swap", "badge tilts to camera");
  return { drop, flip, head, head2, turn };
})();

// EXPLODE: the parts separate over two bars; a label per part on every beat; the headline rolls the other way to its second half.
export const PARTS = ["Clear card", "E-paper", "Baochip-1x", "Magnets and pads", "Coin cell"];
export const EXPLODE = (() => {
  const apart = hero("explode.apart", hit(14.5), DUR.explode);
  mark_(apart, "whoosh", "parts separate");
  const flip = hero("explode.flip", hit(18.8), DUR.flip);
  const head = text("explode.cubeA", "This card makes its own random numbers.", hit(15.4), flip, "head", { exit: "roll" });
  const head2 = text("explode.cubeB", "You watch it happen.", flip, hit(21.0), "head", { enter: "roll" });
  const labels = nextBeats(hit(15.4) - 1, PARTS.length, 1);
  labels.forEach((f, i) => { text(`explode.label${i}`, PARTS[i], f, hit(19.8), `label${i}`); mark_(f, "tick", PARTS[i]); });
  const together = hero("explode.together", hit(19.4), DUR.panelIn);
  mark_(together, "swap", "parts assemble");
  return { apart, flip, head, head2, labels, together };
})();

// TOUCH: a second card lands on the first, a pulse runs along the pads, the top card lifts off and both screens flip to the same six words.
export const TOUCH = (() => {
  const land = hit(21.4);
  const snap = hero("touch.snap", land - DUR.snap, DUR.snap);
  mark_(land, "slam", "the second card lands on the pile");
  const head = text("touch.head", "Stack two. The numbers are made between them.", hit(21.4), hit(26.0), "head");
  const pulse = hero("touch.pulse", hit(22.6), DUR.connector);
  mark_(pulse, "fly", "pulse along the pads");
  const pulse2 = hero("touch.pulse2", hit(23.4), DUR.connector);
  mark_(pulse2, "fly", "pulse back");
  const push = hero("touch.push", hit(24.2), DUR.push);
  const flip = hero("touch.flip", hit(25.4), DUR.flip);
  mark_(flip, "check", "both screens show the words");
  return { snap, land, head, pulse, pulse2, push, flip };
})();

// NETWORK: the payoff. Links grow from the pair to six devices, one per beat; then keys rotate down every link.
export const NODES = ["WireGuard peer", "Mesh radio", "Laptop", "Phone", "Server", "Second site"];
export const NETWORK = (() => {
  const recede = hero("net.recede", hit(27.1), DUR.panelIn);
  mark_(recede, "whoosh", "pair pulls back");
  const links = nextBeats(hit(28.6) - 1, NODES.length, 1);
  links.forEach((f, i) => {
    hero(`net.link${i}`, f, DUR.connector);
    mark_(f, "tick", `link to ${NODES[i]}`);
    text(`net.node${i}`, NODES[i], f + DUR.connector, hit(40.3), `node${i}`);
  });
  const head2 = text("net.head2", "One meeting gives the whole network its keys.", hit(27.9), hit(33.4), "head");
  const rotate = hero("net.rotate", hit(35.4), DUR.explode / 2);
  mark_(rotate, "fly", "keys rotate down the links");
  // The second line is a cube: "No server, no CA." rolls over to its generalization on the bar between the two key rotations.
  const head3 = text("net.head3", "Keys rotate from the badge for years.", hit(34.2), hit(40.3), "head");
  const cubeA = text("net.cubeA", "No server, no CA.", hit(34.2), hit(37.0), "cubeA", { exit: "roll" });
  const flip = hero("net.flip", hit(37.0), DUR.flip);
  mark_(flip, "swap", "cube rolls to the generalization");
  const cubeB = text("net.cubeB", "No man in the middle.", hit(37.0), hit(40.3), "cubeB", { enter: "roll" });
  const rotate2 = hero("net.rotate2", hit(38.6), DUR.explode / 2);
  mark_(rotate2, "fly", "keys rotate again");
  return { recede, links, head2, rotate, head3, cubeA, flip, cubeB, rotate2 };
})();

// CLOSE: what the whole thing means, then the line, the address, a settle, then black.
export const CLOSE = (() => {
  const title = hero("close.title", hit(41.5), DUR.panelIn);
  mark_(title, "enter", "the close begins");
  const pre = text("close.pre", "Nothing was sent. Nothing to decrypt.", CH.close[0], hit(45.0), "head");
  const head = text("close.head", "Get in cahoots.", hit(46.0), TOTAL - DUR.fade, "head");
  const url = text("close.url", "cahoots.network", hit(47.0), TOTAL - DUR.fade, "url");
  mark_(hit(47.0), "tick", "address");
  const settle = hero("close.settle", hit(46.3), DUR.lift);
  mark_(settle, "tick", "badges settle");
  const dim = hero("close.dim", hit(49.4), DUR.lift);
  mark_(dim, "tick", "scene dims");
  const fade = hero("close.fade", TOTAL - DUR.fade, DUR.fade);
  return { title, pre, head, url, settle, dim, fade };
})();

HITS.sort((a, b) => a.f - b.f);

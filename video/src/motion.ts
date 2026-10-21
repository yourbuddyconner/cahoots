// Motion tokens for one piece. Scenes import these; no inline easing or spring numbers.
// Pure functions of time (no engine import), so the audit and the sound synth can evaluate them in plain Node.
// This piece runs at 30 fps; every duration below is the skill's 60 fps value halved.

export const FPS = 30;

/** cubic-bezier(x1, y1, x2, y2) as a function of progress 0..1 (Newton + bisection solve on x). */
export function bezier(x1: number, y1: number, x2: number, y2: number) {
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
  const sx = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sy = (t: number) => ((ay * t + by) * t + cy) * t;
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  return (p: number) => {
    if (p <= 0) return 0;
    if (p >= 1) return 1;
    let t = p;
    for (let i = 0; i < 8; i++) { const e = sx(t) - p; const d = dx(t); if (Math.abs(e) < 1e-6) break; if (Math.abs(d) < 1e-6) break; t -= e / d; }
    if (t < 0 || t > 1 || Math.abs(sx(t) - p) > 1e-4) { let lo = 0, hi = 1; t = p; for (let i = 0; i < 30; i++) { if (sx(t) < p) lo = t; else hi = t; t = (lo + hi) / 2; } }
    return sy(t);
  };
}

export const EASE = {
  enter: bezier(0.16, 1, 0.3, 1), // ease-out-expo: text, panels arriving
  move: bezier(0.76, 0, 0.24, 1), // ease-in-out-quart: pointer, camera, swaps
  exit: bezier(0.7, 0, 0.84, 0), // ease-in-expo: anything leaving
};

/** Durations in frames at 60 fps. Exits are 50% of their entrance. */
export const DUR = {
  textIn: 18, // 600 ms
  textOut: 9,
  panelIn: 23, // 750 ms hero reveal
  panelOut: 12,
  pointer: 18,
  press: 3,
  chapterIn: 20,
  chapterHold: 35,
  chapterCollapse: 10,
  cardIn: 20, // card lands on SPRING.card
  cardOut: 11,
  rowIn: 12,
  connector: 15,
  pulse: 5,
  lift: 20,
  explode: 95, // two bars at 152 BPM: the parts separate
  snap: 40, // the second badge slides in and lands on SPRING.snap
  push: 30, // camera push to the screens
  flip: 12, // e-paper refresh
  fade: 20, // final fade to black
};

export const STAGGER = { word: 3, item: 2, row: 3, card: 9 };
/** Secondary moves start this many frames after the hero, at this fraction of its travel. */
export const SECONDARY = { delay: 5, travel: 0.6 };

/** Damped spring as a pure function of time (seconds) → progress 0..1. zeta 1 = no bounce. */
export function spring(t: number, zeta = 0.8, f = 1.4) {
  if (t <= 0) return 0;
  const w = 2 * Math.PI * f;
  if (zeta >= 1) return 1 - Math.exp(-w * t) * (1 + w * t);
  const wd = w * Math.sqrt(1 - zeta * zeta);
  return 1 - Math.exp(-zeta * w * t) * (Math.cos(wd * t) + ((zeta * w) / wd) * Math.sin(wd * t));
}
export const SPRING = { panel: [0.8, 1.4] as const, check: [0.5, 3] as const, title: [0.85, 1.2] as const, card: [0.75, 1.5] as const, chip: [0.55, 2.6] as const, lift: [0.8, 1.1] as const, badge: [0.8, 1.1] as const, snap: [0.6, 2.2] as const, mark: [0.7, 1.6] as const };

/** 3D staging for product objects. */
export const STAGE = { perspective: 2200, rotY: -8, rotX: 4, liftZ: 140, push: 0.02 };
/** Spring progress at frame `f` for an element that starts at frame `from`. */
export const springAt = (f: number, from: number, [zeta, hz]: readonly [number, number]) => spring((f - from) / FPS, zeta, hz);

/** Linear 0..1 progress between two frames, clamped. */
export const prog = (f: number, a: number, b: number) => (b <= a ? (f >= b ? 1 : 0) : Math.min(1, Math.max(0, (f - a) / (b - a))));

/** Minimum fully-visible hold for a text state, in frames: 0.3 s per word + 0.5 s. */
export const minHold = (text: string) => Math.ceil((0.3 * text.trim().split(/\s+/).length + 0.5) * FPS);

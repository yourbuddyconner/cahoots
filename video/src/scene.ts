// The launch video as a pure function of the frame number. The driver (render.mjs) calls
// window.renderFrame(f) and screenshots the page. Every number comes from motion.ts or timeline.ts.
import * as THREE from "three";
import { makeBadge } from "../../site/badge.js";
import { addGround, addLights, enableShadows, setupEnvironment } from "../../site/rig.js";
import { DUR, EASE, FPS, SPRING, STAGGER, prog, springAt } from "./motion";
import { BADGE, CH, CLOSE, EXPLODE, OPEN, PARTS, TEXTS, TOTAL, TOUCH, USES, USES_ITEMS, WORDS, cue, move } from "./timeline";

declare global {
  interface Window { renderFrame: (f: number) => Promise<void>; ready: boolean; TOTAL: number }
}

const W = 1920, H = 1080;
const canvas = document.getElementById("gl") as HTMLCanvasElement;
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, preserveDrawingBuffer: true });
renderer.setPixelRatio(1);
renderer.setSize(W, H, false);
renderer.toneMapping = THREE.ACESFilmicToneMapping;
const EXPOSURE = 0.8;
renderer.toneMappingExposure = EXPOSURE;
renderer.setClearColor(0x000000, 0);

const scene = new THREE.Scene();
enableShadows(renderer);
const envReady = setupEnvironment(renderer, scene, "../../site/assets/hdr/studio.hdr", 0.9);
const camera = new THREE.PerspectiveCamera(30, W / H, 0.1, 100);
addLights(scene);
const ground = addGround(scene, 0, 0.5);

// Two badges. B is the one that slides in and snaps on from the right (+x), pins to pads.
const A = makeBadge(renderer);
const B = makeBadge(renderer);
const stage = new THREE.Group();
stage.add(A.group, B.group);
scene.add(stage);
const GAP = A.W + 0.0; // B sits edge to edge with A when snapped

// E-paper states. The site texture shows the words; the "waiting" state is drawn here.
const wordsTex = A.materials.paperMat.map!;
wordsTex.rotation = 0; // the badges lie with the lanyard away from the camera, so the words read upright
B.materials.paperMat.map!.rotation = 0;
function paperTexture(draw: (ctx: CanvasRenderingContext2D, w: number, h: number) => void) {
  const c = document.createElement("canvas");
  c.width = c.height = 512;
  draw(c.getContext("2d")!, 512, 512);
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.center.set(0.5, 0.5);
  t.rotation = 0;
  return t;
}
const waitingTex = paperTexture((ctx, w, h) => {
  ctx.fillStyle = "#e6e2d8";
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "#17191b";
  ctx.font = `500 ${w * 0.052}px "JetBrains Mono", monospace`;
  ctx.fillText("sf-meetup", w * 0.07, h * 0.13);
  ctx.textAlign = "right";
  ctx.font = `400 ${w * 0.042}px "JetBrains Mono", monospace`;
  ctx.fillText("2 of 2", w * 0.93, h * 0.13);
  ctx.textAlign = "left";
  ctx.fillRect(w * 0.07, h * 0.18, w * 0.86, 2);
  ctx.font = `500 ${w * 0.07}px "JetBrains Mono", monospace`;
  ctx.fillText("touching…", w * 0.07, h * 0.5);
  ctx.font = `400 ${w * 0.042}px "JetBrains Mono", monospace`;
  ctx.fillText("hold the badges together", w * 0.07, h * 0.92);
});
const blankTex = paperTexture((ctx, w, h) => {
  ctx.fillStyle = "#17191b";
  ctx.fillRect(0, 0, w, h);
});
// Each badge gets its own paper material so the two can flip independently.
const paperA = A.materials.paperMat;
const paperB = B.materials.paperMat;

// The amber pulse that crosses the pins.
const pulse = new THREE.Mesh(
  new THREE.SphereGeometry(0.06, 16, 12),
  new THREE.MeshBasicMaterial({ color: 0xe5a33a, transparent: true, opacity: 0 }),
);
stage.add(pulse);

// ───────────── DOM overlay ─────────────
const overlay = document.getElementById("overlay")!;
const els = new Map<string, HTMLElement>();
function el(id: string, cls: string, textContent: string) {
  let e = els.get(id);
  if (!e) {
    e = document.createElement("div");
    e.className = cls;
    e.dataset.id = id;
    for (const w of textContent.split(/\s+/)) {
      const s = document.createElement("span");
      s.className = "w";
      s.textContent = w;
      e.appendChild(s);
    }
    overlay.appendChild(e);
    els.set(id, e);
  }
  return e;
}

/** Word-wise reveal: 0 before `in`, 1 once shown, back to 0 after `out`. Text never moves while read. */
function textState(f: number, id: string, cls: string, preShown = false) {
  const c = cue(id);
  // A pre-shown text finished its reveal before frame 0, so frame 0 is a finished frame (the thumbnail).
  const shift = preShown ? DUR.textIn + STAGGER.word * c.text.split(/\s+/).length : 0;
  f += shift;
  const e = el(id, cls, c.text);
  const words = e.querySelectorAll<HTMLElement>(".w");
  const visible = f >= c.in && f - shift < c.out + DUR.textOut + STAGGER.word * words.length;
  e.style.display = visible ? "" : "none";
  if (!visible) return;
  words.forEach((w, i) => {
    const pin = EASE.enter(prog(f, c.in + i * STAGGER.word, c.in + i * STAGGER.word + DUR.textIn));
    const pout = EASE.exit(prog(f - shift, c.out + i * 2, c.out + i * 2 + DUR.textOut));
    const o = Math.min(pin, 1 - pout);
    w.style.opacity = String(o);
    w.style.transform = `translateY(${(1 - pin) * 0.35}em)`;
  });
}

const ticker = document.getElementById("ticker")!;
const tickerText = (() => {
  let s = 1;
  const r = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const g: string[] = [];
  for (let i = 0; i < 80; i++) g.push(String(Math.floor(r() * 100000)).padStart(5, "0"));
  return g.join("   ");
})();
ticker.textContent = `${tickerText}   ${tickerText}`;
const fadeEl = document.getElementById("fade")!;
const markEl = document.getElementById("mark")!;

// ───────────── helpers ─────────────
const between = (f: number, a: number, b: number) => f >= a && f < b;
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
/** Progress of a registered move at frame f, with the given easing. */
const mv = (f: number, id: string, ease = EASE.move) => {
  const m = move(id);
  return ease(prog(f, m.from, m.from + m.dur));
};

// Camera rigs, as (position, lookAt) pairs the chapters blend between.
const RIG = {
  hero: { p: new THREE.Vector3(0, 5.2, 9.6), l: new THREE.Vector3(-1.6, 0.2, 0) },
  explode: { p: new THREE.Vector3(0, 4.2, 16.5), l: new THREE.Vector3(-1.7, 0.9, 0) },
  pair: { p: new THREE.Vector3(1.4, 5.6, 10.4), l: new THREE.Vector3(1.4, -0.6, 0) },
  screens: { p: new THREE.Vector3(1.4, 7.4, 6.2), l: new THREE.Vector3(1.4, -1.3, 0.5) },
  far: { p: new THREE.Vector3(5.2, 5.0, 13.0), l: new THREE.Vector3(1.4, 0.4, 0) },
  close: { p: new THREE.Vector3(1.4, 7.0, 9.0), l: new THREE.Vector3(1.4, -0.7, 0) },
};
function setCam(a: { p: THREE.Vector3; l: THREE.Vector3 }, b: { p: THREE.Vector3; l: THREE.Vector3 }, t: number) {
  camera.position.lerpVectors(a.p, b.p, t);
  camera.lookAt(new THREE.Vector3().lerpVectors(a.l, b.l, t));
}

// ───────────── the frame ─────────────
async function renderFrame(f: number) {
  const sec = f / FPS;
  // Defaults every frame, so a chapter only has to set what differs.
  A.group.visible = true;
  B.group.visible = false;
  A.group.position.set(0, 0.55, 0);
  A.group.rotation.set(0.1, 0, 0);
  B.group.position.set(GAP, 0.55, 0);
  B.group.rotation.set(0.1, 0, 0);
  A.explode(0);
  B.explode(0);
  paperA.map = wordsTex;
  paperB.map = wordsTex;
  pulse.material.opacity = 0;
  stage.visible = true;
  markEl.style.opacity = "0";
  ticker.parentElement!.style.opacity = "0";
  fadeEl.style.opacity = "0";
  for (const e of els.values()) e.style.display = "none";
  const ambient = 0.05; // ambient turn, as a fraction of a radian per second: under 5% of any hero travel

  // OPEN
  if (between(f, CH.open[0], CH.badge[0] + DUR.panelOut)) {
    stage.visible = false;
    // The mark leaves with the wordmark, not later with the chapter.
    const wordOut = cue("open.word").out;
    const out = EASE.exit(prog(f, wordOut, wordOut + DUR.textOut));
    markEl.style.opacity = String(1 - out);
    markEl.style.transform = "translate(-50%, -50%)";
    textState(f, "open.word", "wordmark", true);
    const tk = mv(f, "open.ticker", EASE.enter);
    ticker.parentElement!.style.opacity = String(tk * (1 - out));
  }

  // BADGE
  if (between(f, CH.badge[0] - 1, CH.explode[0])) {
    const drop = springAt(f, BADGE.drop, SPRING.badge);
    A.group.position.y = lerp(4.5, 0.55, Math.min(1, drop));
    const turn = mv(f, "badge.turn");
    A.group.rotation.y = (-0.5 + ambient * (sec - CH.badge[0] / FPS)) * (1 - turn);
    A.group.rotation.x = 0.1 + turn * 0.3;
    setCam(RIG.hero, RIG.hero, 0);
    textState(f, "badge.head", "head");
  }

  // EXPLODE
  if (between(f, CH.explode[0], CH.touch[0])) {
    const apart = mv(f, "explode.apart");
    const together = mv(f, "explode.together");
    const e = apart * (1 - together);
    A.explode(e);
    A.group.rotation.y = -0.45 * e;
    A.group.rotation.x = 0.1 + 0.3 * (1 - together) * (1 - apart);
    A.group.position.y = 0.55 - 0.5 * apart * (1 - together);
    setCam(RIG.hero, RIG.explode, e);
    textState(f, "explode.head", "head");
    PARTS.forEach((_, i) => textState(f, `explode.label${i}`, `label l${i}`));
  }

  // TOUCH
  if (between(f, CH.touch[0], CH.words[0])) {
    B.group.visible = true;
    const t0 = TOUCH.snap;
    const slide = EASE.move(prog(f, t0, t0 + DUR.snap * 0.6));
    const land = springAt(f, TOUCH.land, SPRING.snap);
    const x = f < TOUCH.land ? lerp(GAP + 6.5, GAP + 0.9, slide) : lerp(GAP + 0.9, GAP, Math.min(1.08, land));
    B.group.position.x = x;
    A.group.rotation.y = 0;
    B.group.rotation.y = 0;
    const camT = EASE.move(prog(f, CH.touch[0], CH.touch[0] + DUR.panelIn));
    setCam(RIG.explode, RIG.pair, camT);
    for (const id of ["touch.pulse", "touch.pulse2"]) {
      const m = move(id);
      const p = EASE.move(prog(f, m.from, m.from + m.dur));
      if (f >= m.from && f < m.from + m.dur + 4) {
        const dir = id === "touch.pulse" ? 1 : -1;
        pulse.position.set(lerp(A.W / 2 - 0.6, GAP + 0.6, dir > 0 ? p : 1 - p), 0.55 + 0.12, -0.05);
        pulse.material.opacity = Math.sin(Math.PI * Math.min(1, p)) * 0.95;
      }
    }
    textState(f, "touch.head", "head");
  }

  // WORDS
  if (between(f, CH.words[0], CH.uses[0])) {
    B.group.visible = true;
    A.group.rotation.y = 0;
    B.group.rotation.y = 0;
    const push = mv(f, "words.push");
    setCam(RIG.pair, RIG.screens, push);
    const flip = prog(f, WORDS.flip, WORDS.flip + DUR.flip);
    const tex = flip <= 0 ? waitingTex : flip < 0.5 ? blankTex : wordsTex;
    paperA.map = tex;
    paperB.map = tex;
    textState(f, "words.head", "head");
  }

  // USES
  if (between(f, CH.uses[0], CH.close[0])) {
    B.group.visible = true;
    A.group.rotation.y = 0;
    B.group.rotation.y = 0;
    const r = mv(f, "uses.recede");
    setCam(RIG.screens, RIG.far, r);
    textState(f, "uses.head", "head");
    USES_ITEMS.forEach((_, i) => textState(f, `uses.item${i}`, `item i${i}`));
  }

  // CLOSE
  if (f >= CH.close[0]) {
    B.group.visible = true;
    const title = mv(f, "close.title");
    const settle = mv(f, "close.settle");
    setCam(RIG.far, RIG.close, Math.max(title, settle));
    const spin = ambient * (sec - CH.close[0] / FPS);
    A.group.rotation.y = spin * (1 - settle);
    B.group.rotation.y = spin * (1 - settle);
    const dim = mv(f, "close.dim");
    renderer.toneMappingExposure = EXPOSURE * (1 - 0.8 * dim);
    textState(f, "close.head", "head");
    textState(f, "close.url", "url");
    ticker.parentElement!.style.opacity = String(title * (1 - dim));
  } else {
    renderer.toneMappingExposure = EXPOSURE;
  }
  ticker.style.transform = `translateX(${-(sec * 60) % (ticker.scrollWidth / 2)}px)`;

  const fade = mv(f, "close.fade", EASE.exit);
  fadeEl.style.opacity = String(fade);

  // The ground sits just under the lowest visible part, so shadows land where the badge rests.
  stage.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(stage);
  ground.position.y = box.min.y - 0.01;
  renderer.render(scene, camera);
  await new Promise((r) => requestAnimationFrame(() => r(null)));
}

window.renderFrame = renderFrame;
window.TOTAL = TOTAL;
Promise.all([document.fonts.ready, envReady]).then(() => { window.ready = true; });

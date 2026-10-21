// The launch video as a pure function of the frame number. The driver (render.mjs) calls
// window.renderFrame(f) and screenshots the page. Every number comes from motion.ts or timeline.ts.
import * as THREE from "three";
import { makeBadge } from "../../site/badge.js";
import { addGround, addLights, enableShadows, setupEnvironment } from "../../site/rig.js";
import { DUR, EASE, FPS, SPRING, STAGGER, prog, springAt } from "./motion";
import { BADGE, CH, EXPLODE, NETWORK, NODES, OPEN, PARTS, TOTAL, TOUCH, cue, move } from "./timeline";

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

// The network: six devices on an arc behind the pair, a link to each, and key pulses that run down the links.
const NET_CENTER = new THREE.Vector3(1.4, 0.3, 0);
const NET_ANCHOR = new THREE.Vector3(1.4, 0.95, -0.3);
const NET_R = 6.5;
const amberMat = new THREE.MeshBasicMaterial({ color: 0xe5a33a, transparent: true, opacity: 0.9 });
const puckMat = new THREE.MeshStandardMaterial({ color: 0x1a1b1e, roughness: 0.5, metalness: 0.3 });
const net = new THREE.Group();
scene.add(net);
const nodes = NODES.map((_, i) => {
  const a = THREE.MathUtils.degToRad(-72 + (144 * i) / (NODES.length - 1));
  const pos = new THREE.Vector3(NET_CENTER.x + NET_R * Math.sin(a), NET_CENTER.y, NET_CENTER.z - NET_R * Math.cos(a));
  const puck = new THREE.Mesh(new THREE.CylinderGeometry(0.42, 0.42, 0.14, 40), puckMat);
  puck.position.copy(pos);
  puck.castShadow = puck.receiveShadow = true;
  const ring = new THREE.Mesh(new THREE.TorusGeometry(0.34, 0.03, 12, 48), amberMat.clone());
  ring.rotation.x = Math.PI / 2;
  ring.position.copy(pos).add(new THREE.Vector3(0, 0.08, 0));
  const dir = pos.clone().sub(NET_ANCHOR);
  const len = dir.length();
  const link = new THREE.Mesh(new THREE.CylinderGeometry(0.022, 0.022, 1, 8), amberMat.clone());
  link.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir.clone().normalize());
  const key = new THREE.Mesh(new THREE.SphereGeometry(0.12, 12, 10), amberMat.clone());
  net.add(puck, ring, link, key);
  return { pos, puck, ring, link, key, dir: dir.normalize(), len };
});
/** Draw link i grown to `p` of its length and its device lit by `lit`. */
function setLink(i: number, p: number, lit: number) {
  const n = nodes[i];
  n.link.visible = p > 0;
  n.link.scale.y = Math.max(0.001, n.len * p);
  n.link.position.copy(NET_ANCHOR).addScaledVector(n.dir, (n.len * p) / 2);
  (n.ring.material as THREE.MeshBasicMaterial).opacity = 0.15 + 0.85 * lit;
  n.ring.visible = true;
  n.puck.visible = true;
}
/** A key travelling down link i: `p` 0 at the pair, 1 at the device. Hidden outside 0..1. */
function setKey(i: number, p: number) {
  const n = nodes[i];
  n.key.visible = p > 0 && p < 1;
  n.key.position.copy(NET_ANCHOR).addScaledVector(n.dir, n.len * p);
}
const ndc = new THREE.Vector3();
function placeLabel(id: string, pos: THREE.Vector3) {
  const e = els.get(id);
  if (!e || e.style.display === "none") return;
  ndc.copy(pos).project(camera);
  e.style.left = `${((ndc.x + 1) / 2) * W}px`;
  e.style.top = `${((1 - ndc.y) / 2) * H - 64}px`;
}

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

/** The network's second headline: a fixed first line and a second line that rolls like a cube between two faces. */
function cubeHead(f: number) {
  const line = cue("net.head3"), a = cue("net.cubeA"), b = cue("net.cubeB");
  let e = els.get("net.cube");
  if (!e) {
    e = document.createElement("div");
    e.className = "head cubehead";
    const l1 = document.createElement("div");
    l1.className = "line";
    for (const w of line.text.split(/\s+/)) { const s = document.createElement("span"); s.className = "w"; s.textContent = w; l1.appendChild(s); }
    const cube = document.createElement("div");
    cube.className = "cube";
    for (const [cls, txt] of [["a", a.text], ["b", b.text]] as const) {
      const face = document.createElement("div");
      face.className = `face ${cls}`;
      for (const w of txt.split(/\s+/)) { const s = document.createElement("span"); s.className = "w"; s.textContent = w; face.appendChild(s); }
      cube.appendChild(face);
    }
    const wrap = document.createElement("div");
    wrap.className = "cubewrap";
    wrap.appendChild(cube);
    e.append(l1, wrap);
    overlay.appendChild(e);
    els.set("net.cube", e);
  }
  const visible = f >= line.in && f < line.out + DUR.textOut + STAGGER.word * 8;
  e.style.display = visible ? "" : "none";
  if (!visible) return;
  const reveal = (el: HTMLElement, inF: number) => {
    el.querySelectorAll<HTMLElement>(".w").forEach((w, i) => {
      const pin = EASE.enter(prog(f, inF + i * STAGGER.word, inF + i * STAGGER.word + DUR.textIn));
      const pout = EASE.exit(prog(f, line.out + i * 2, line.out + i * 2 + DUR.textOut));
      w.style.opacity = String(Math.min(pin, 1 - pout));
      w.style.transform = `translateY(${(1 - pin) * 0.35}em)`;
    });
  };
  reveal(e.querySelector(".line")!, line.in);
  reveal(e.querySelector(".face.a")!, a.in);
  // Face b is revealed by the roll itself, so its words are fully opaque once the roll starts.
  reveal(e.querySelector(".face.b")!, b.in - DUR.textIn - STAGGER.word * 5);
  const roll = mv(f, "net.flip");
  (e.querySelector(".cube") as HTMLElement).style.transform = `rotateX(${-90 * roll}deg)`;
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
const RIG: Record<string, { p: THREE.Vector3; l: THREE.Vector3 }> = {
  hero: { p: new THREE.Vector3(0, 5.2, 9.6), l: new THREE.Vector3(-2.0, 0.1, 0) },
  explode: { p: new THREE.Vector3(0, 4.2, 16.5), l: new THREE.Vector3(-1.7, 0.9, 0) },
  pair: { p: new THREE.Vector3(1.4, 5.6, 10.4), l: new THREE.Vector3(1.4, -0.6, 0) },
  screens: { p: new THREE.Vector3(1.4, 7.4, 6.2), l: new THREE.Vector3(1.4, -1.3, 0.5) },
  net: { p: new THREE.Vector3(1.0, 8.6, 12.8), l: new THREE.Vector3(0.9, 0.0, -2.4) },
  close: { p: new THREE.Vector3(1.4, 7.0, 9.0), l: new THREE.Vector3(1.4, -0.7, 0) },
};
// Where the touch chapter leaves the camera: part way from the pair rig to the screens rig.
RIG.touchEnd = { p: new THREE.Vector3().lerpVectors(RIG.pair.p, RIG.screens.p, 0.7), l: new THREE.Vector3().lerpVectors(RIG.pair.l, RIG.screens.l, 0.7) };
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
  net.visible = false;
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
  if (between(f, CH.touch[0], CH.network[0])) {
    B.group.visible = true;
    const t0 = TOUCH.snap;
    const slide = EASE.move(prog(f, t0, t0 + DUR.snap * 0.6));
    const land = springAt(f, TOUCH.land, SPRING.snap);
    const x = f < TOUCH.land ? lerp(GAP + 6.5, GAP + 0.9, slide) : lerp(GAP + 0.9, GAP, Math.min(1.08, land));
    B.group.position.x = x;
    A.group.rotation.y = 0;
    B.group.rotation.y = 0;
    const camT = EASE.move(prog(f, CH.touch[0], CH.touch[0] + DUR.panelIn));
    const push = mv(f, "touch.push");
    if (push > 0) setCam(RIG.pair, RIG.touchEnd, push);
    else setCam(RIG.explode, RIG.pair, camT);
    for (const id of ["touch.pulse", "touch.pulse2"]) {
      const m = move(id);
      const p = EASE.move(prog(f, m.from, m.from + m.dur));
      if (f >= m.from && f < m.from + m.dur + 4) {
        const dir = id === "touch.pulse" ? 1 : -1;
        pulse.position.set(lerp(A.W / 2 - 0.6, GAP + 0.6, dir > 0 ? p : 1 - p), 0.55 + 0.12, -0.05);
        pulse.material.opacity = Math.sin(Math.PI * Math.min(1, p)) * 0.95;
      }
    }
    // Both screens wait, then refresh to the same six words on the beat.
    const flip = prog(f, TOUCH.flip, TOUCH.flip + DUR.flip);
    const tex = flip <= 0 ? waitingTex : flip < 0.5 ? blankTex : wordsTex;
    paperA.map = tex;
    paperB.map = tex;
    textState(f, "touch.head", "head");
  }

  // NETWORK: the pair pulls back, links grow to six devices one per beat, then keys run down every link.
  if (between(f, CH.network[0], CH.close[0])) {
    B.group.visible = true;
    A.group.rotation.y = 0;
    B.group.rotation.y = 0;
    net.visible = true;
    const r = mv(f, "net.recede");
    setCam(RIG.touchEnd, RIG.net, r);
    NODES.forEach((_, i) => {
      const grow = EASE.enter(prog(f, NETWORK.links[i], NETWORK.links[i] + DUR.connector));
      // The ring lights when the link lands, with one soft settle.
      const lit = Math.min(1, springAt(f, NETWORK.links[i] + DUR.connector, SPRING.chip));
      setLink(i, grow, Math.max(0, lit));
      let keyP = -1;
      for (const id of ["net.rotate", "net.rotate2"]) {
        const m = move(id);
        const from = m.from + i * STAGGER.item;
        if (f >= from && f < from + m.dur) keyP = EASE.move(prog(f, from, from + m.dur));
      }
      setKey(i, keyP);
      textState(f, `net.node${i}`, "node");
      placeLabel(`net.node${i}`, nodes[i].pos);
    });
    textState(f, "net.head2", "head");
    cubeHead(f);
  }

  // CLOSE
  if (f >= CH.close[0]) {
    B.group.visible = true;
    const title = mv(f, "close.title");
    const settle = mv(f, "close.settle");
    setCam(RIG.net, RIG.close, Math.max(title, settle));
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

// Cahoots landing page: the exploded badge and the Numbers Station ticker.
// The badge is the reference hardware design, built from primitives so nothing is fetched: a
// credit-card footprint 2.9 mm thick, clear polycarbonate, a 1.54" e-paper, the Baochip-1x on a
// black board, a coin cell, two dome buttons, and the stacking link: four magnets, four gold pads
// on the face, four spring fingers underneath. See badge.js.
import * as THREE from "three";
import { makeBadge } from "./badge.js";
import { addGround, addLights, enableShadows, makeBackdrop, makeComposer, setupEnvironment } from "./rig.js";

// ---------- Numbers Station ticker: real random five-digit groups ----------
function groups(n) {
  const out = [];
  const buf = new Uint32Array(n);
  crypto.getRandomValues(buf);
  for (let i = 0; i < n; i++) out.push(String(buf[i] % 100000).padStart(5, "0"));
  return out;
}
const ticker = document.getElementById("ticker");
if (ticker) {
  const g = groups(120).join("  ");
  ticker.textContent = g + "    " + g;
}
const sample = document.getElementById("station-sample");
if (sample) {
  const lines = [];
  for (let r = 0; r < 6; r++) lines.push(groups(8).join(" "));
  sample.textContent = lines.join("\n");
}

// ---------- Exploded badge ----------
const canvas = document.getElementById("badge");
const parts = [...document.querySelectorAll("#parts li")];
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

function setActivePart(t) {
  const idx = Math.min(parts.length - 1, Math.floor(t * parts.length * 1.15));
  parts.forEach((li, i) => li.classList.toggle("on", reduced ? true : i === idx));
}

function hasWebGL() {
  try {
    return !!document.createElement("canvas").getContext("webgl2");
  } catch {
    return false;
  }
}

async function start() {
  // The screen and the board carry text drawn into textures: wait for the page's mono font.
  try {
    await Promise.race([document.fonts.load('500 20px "JetBrains Mono"'), new Promise((r) => setTimeout(r, 1500))]);
  } catch {}

  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: false, powerPreference: "high-performance" });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;
  renderer.setClearColor(0x0a0a0b, 1);
  enableShadows(renderer);

  const scene = new THREE.Scene();
  setupEnvironment(renderer, scene, null, 1.0);
  const camera = new THREE.PerspectiveCamera(20, 1, 0.5, 200);
  addLights(scene);
  const backdrop = makeBackdrop(renderer);
  scene.add(backdrop);

  const badge = makeBadge(renderer);
  const { group, stack } = badge;
  scene.add(group);
  const ground = addGround(scene, 0, 0.4);

  const section = document.getElementById("explode");
  let target = 0, t = reduced ? 0.7 : 0;
  let mouseX = 0, mouseY = 0;
  addEventListener("pointermove", (e) => {
    mouseX = (e.clientX / innerWidth - 0.5) * 2;
    mouseY = (e.clientY / innerHeight - 0.5) * 2;
  }, { passive: true });

  function progress() {
    const rect = section.getBoundingClientRect();
    const total = rect.height - innerHeight;
    return Math.min(1, Math.max(0, -rect.top / total));
  }
  const ease = (x) => 1 - Math.pow(1 - x, 3);

  let composer = null, size = { w: 0, h: 0 }, wide = true;
  function resize() {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    if (w === size.w && h === size.h) return;
    size = { w, h };
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    composer?.composer.dispose?.();
    composer = makeComposer(renderer, scene, camera, w, h, { ao: w > 600 });
    wide = w > 860;
  }
  // The backdrop sits far behind the subject, square to the camera, and covers the view at any aspect.
  const dir = new THREE.Vector3();
  function placeBackdrop() {
    const dist = 60;
    dir.copy(camera.position).negate().normalize();
    backdrop.position.copy(camera.position).addScaledVector(dir, dist);
    backdrop.lookAt(camera.position);
    const side = 2 * (dist + 12) * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * Math.max(camera.aspect, 1) * 1.25;
    backdrop.scale.set(side, side, 1);
  }

  const timer = new THREE.Timer();
  timer.connect(document);
  // A slow turntable sway around a three-quarter view, so the screen stays readable.
  const sway = (time) => 0.42 + 0.28 * Math.sin(time * 0.22);

  function animate() {
    timer.update();
    resize();
    if (!reduced) {
      target = progress();
      t += (target - t) * Math.min(1, timer.getDelta() * 5);
    }
    const e = ease(t);
    badge.explode(e);
    // Camera: a three-quarter view from above, pulling back a little as the parts spread out.
    const dolly = 1 + 0.2 * e;
    camera.position.set(0, (wide ? 6.0 : 10.0) * dolly, (wide ? 11.8 : 22.0) * dolly);
    camera.lookAt(0, 0, 0);
    placeBackdrop();
    // Keep the stack centred and clear of the part list at the bottom: the card is 2.9 mm thick
    // at rest and about 45 mm tall exploded.
    group.position.y = -badge.MM * (1.45 + e * 5.0) + 0.1 + e * 0.15;
    group.rotation.y = (reduced ? 0.42 : sway(timer.getElapsed())) + mouseX * 0.2;
    group.rotation.x = mouseY * 0.06;
    ground.position.y = group.position.y + stack[0].mesh.position.y - 0.004;
    setActivePart(t);
    composer.composer.render();
  }
  renderer.setAnimationLoop(animate);
}

if (!canvas || !hasWebGL()) {
  document.getElementById("nogl")?.removeAttribute("hidden");
  parts.forEach((li) => li.classList.add("on"));
} else {
  start();
}

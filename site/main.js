// Cahoots landing page: the exploded badge and the Numbers Station ticker.
// The badge is the reference hardware design, built from primitives so nothing is fetched:
// an ID-1 card footprint (54 × 86 mm, scaled 1 mm = 0.05 units), two polycarbonate shells
// held by four screws, a 1.54" e-paper on a ribbon, the Baochip-1x on a black ENIG board,
// a four-pin magnetic touch link on each short edge, a 400 mAh LiPo, USB-C, one button.
import * as THREE from "three";
import { makeBadge } from "./badge.js";
import { addLights, enableShadows, setupEnvironment } from "./rig.js";

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

if (!canvas || !hasWebGL()) {
  document.getElementById("nogl")?.removeAttribute("hidden");
  parts.forEach((li) => li.classList.add("on"));
} else {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.setClearColor(0x000000, 0);

  const scene = new THREE.Scene();
  enableShadows(renderer);
  setupEnvironment(renderer, scene, "assets/hdr/studio.hdr", 0.9);
  const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
  addLights(scene);

  const { group, stack } = makeBadge(renderer);
  scene.add(group);

  group.rotation.x = 0.0;

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

  function resize() {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    const pr = renderer.getPixelRatio();
    if (canvas.width !== Math.floor(w * pr) || canvas.height !== Math.floor(h * pr)) {
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    }
    const wide = w > 860;
    group.position.x = 0;
    camera.position.set(0, 5.4, wide ? 9.8 : 13.5);
  }

  const timer = new THREE.Timer();
  timer.connect(document);
  let spin = 0.6;

  function animate() {
    timer.update();
    resize();
    if (!reduced) {
      target = progress();
      t += (target - t) * Math.min(1, timer.getDelta() * 5);
      spin += timer.getDelta() * 0.12;
    }
    const e = ease(t);
    for (const p of stack) p.mesh.position.y = p.rest + (p.out - p.rest) * e;
    // Keep the exploded stack centred: it spans about -1.9 to +3.2 units when fully apart.
    group.position.y = 0.55 - e * 0.5;
    group.rotation.y = spin + mouseX * 0.2;
    group.rotation.x = 0.1 + mouseY * 0.08;
    camera.lookAt(0, group.position.y + 0.9 * e + 0.1, 0);
    setActivePart(t);
    renderer.render(scene, camera);
  }
  renderer.setAnimationLoop(animate);
}

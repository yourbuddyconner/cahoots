// Cahoots landing page: the exploded badge and the Numbers Station ticker.
// The badge is the reference hardware design, built from primitives so nothing is fetched:
// an ID-1 card footprint (54 × 86 mm, scaled 1 mm = 0.05 units), two polycarbonate shells
// held by four screws, a 1.54" e-paper on a ribbon, the Baochip-1x on a black ENIG board,
// a four-pin magnetic touch link on each short edge, a 400 mAh LiPo, USB-C, one button.
import * as THREE from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";

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
  const pmrem = new THREE.PMREMGenerator(renderer);
  const room = new RoomEnvironment();
  scene.environment = pmrem.fromScene(room, 0.04).texture;
  scene.environmentIntensity = 0.55;
  room.dispose();
  pmrem.dispose();

  const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
  const key = new THREE.DirectionalLight(0xfff4e6, 2.2);
  key.position.set(4, 7, 5);
  scene.add(key);
  const rim = new THREE.DirectionalLight(0xc8783c, 1.2);
  rim.position.set(-6, 3, -5);
  scene.add(rim);
  scene.add(new THREE.HemisphereLight(0xe8e4da, 0x1b2228, 0.5));

  // 1 mm = 0.05 units. Card: 54 × 86 mm. Y is the stacking axis.
  const MM = 0.05;
  const W = 54 * MM, L = 86 * MM, CORNER = 3.2 * MM;

  function rrect(w, h, r, holes = []) {
    const s = new THREE.Shape();
    s.moveTo(-w / 2 + r, -h / 2);
    s.lineTo(w / 2 - r, -h / 2);
    s.quadraticCurveTo(w / 2, -h / 2, w / 2, -h / 2 + r);
    s.lineTo(w / 2, h / 2 - r);
    s.quadraticCurveTo(w / 2, h / 2, w / 2 - r, h / 2);
    s.lineTo(-w / 2 + r, h / 2);
    s.quadraticCurveTo(-w / 2, h / 2, -w / 2, h / 2 - r);
    s.lineTo(-w / 2, -h / 2 + r);
    s.quadraticCurveTo(-w / 2, -h / 2, -w / 2 + r, -h / 2);
    for (const hole of holes) s.holes.push(hole);
    return s;
  }
  // A rounded slot, used for the lanyard hole.
  function slot(cx, cy, w, h) {
    const p = new THREE.Path();
    const r = h / 2;
    p.moveTo(cx - w / 2 + r, cy - r);
    p.lineTo(cx + w / 2 - r, cy - r);
    p.absarc(cx + w / 2 - r, cy, r, -Math.PI / 2, Math.PI / 2, false);
    p.lineTo(cx - w / 2 + r, cy + r);
    p.absarc(cx - w / 2 + r, cy, r, Math.PI / 2, (3 * Math.PI) / 2, false);
    return p;
  }
  // Extrude a shape lying flat (in XZ), thickness along +Y, base at y = 0.
  function plate(shape, depth, material, bevel = 0) {
    const geo = new THREE.ExtrudeGeometry(shape, {
      depth,
      bevelEnabled: bevel > 0,
      bevelThickness: bevel,
      bevelSize: bevel,
      bevelSegments: 2,
    });
    geo.rotateX(-Math.PI / 2); // extrusion now runs from y = 0 to y = depth
    return new THREE.Mesh(geo, material);
  }
  function box(w, h, d, material) {
    const m = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), material);
    m.geometry.translate(0, h / 2, 0);
    return m;
  }

  function canvasTexture(draw, w = 512, h = 512) {
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    draw(c.getContext("2d"), w, h);
    const t = new THREE.CanvasTexture(c);
    t.colorSpace = THREE.SRGBColorSpace;
    t.anisotropy = Math.min(8, renderer.capabilities.getMaxAnisotropy());
    return t;
  }

  // E-paper: what the screen shows after a ceremony.
  const epaperTex = canvasTexture((ctx, w, h) => {
    ctx.fillStyle = "#e6e2d8";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "#17191b";
    ctx.font = `500 ${w * 0.052}px "IBM Plex Mono", monospace`;
    ctx.fillText("sf-meetup", w * 0.07, h * 0.13);
    ctx.font = `400 ${w * 0.042}px "IBM Plex Mono", monospace`;
    ctx.textAlign = "right";
    ctx.fillText("TRNG · T4", w * 0.93, h * 0.13);
    ctx.textAlign = "left";
    ctx.fillRect(w * 0.07, h * 0.18, w * 0.86, 2);
    ctx.font = `500 ${w * 0.085}px "IBM Plex Mono", monospace`;
    const words = ["sunset", "valve", "present", "raven", "curtain", "pudding"];
    words.forEach((word, i) => {
      ctx.fillText(word, w * 0.07 + (i % 2) * w * 0.46, h * 0.34 + Math.floor(i / 2) * h * 0.19);
    });
    ctx.font = `400 ${w * 0.042}px "IBM Plex Mono", monospace`;
    ctx.fillText("read aloud · press to confirm", w * 0.07, h * 0.92);
  });

  // Board: black solder mask, gold pads and traces, silkscreen.
  const pcbTex = canvasTexture(
    (ctx, w, h) => {
      ctx.fillStyle = "#15171a";
      ctx.fillRect(0, 0, w, h);
      ctx.strokeStyle = "#c9a24a";
      ctx.lineWidth = 2.2;
      ctx.lineCap = "round";
      for (let i = 0; i < 34; i++) {
        const y = h * 0.1 + i * h * 0.022;
        const kink = w * (0.42 + (i % 6) * 0.035);
        ctx.beginPath();
        ctx.moveTo(w * 0.1, y);
        ctx.lineTo(kink, y);
        ctx.lineTo(kink + w * 0.04, y + h * 0.022);
        ctx.lineTo(w * 0.8, y + h * 0.022);
        ctx.stroke();
      }
      ctx.fillStyle = "#c9a24a";
      for (let i = 0; i < 48; i++) {
        ctx.beginPath();
        ctx.arc(w * 0.12 + (i % 16) * w * 0.05, h * 0.9 + Math.floor(i / 16) * h * 0.03, 3, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.fillStyle = "#e8e4da";
      ctx.font = `500 ${w * 0.028}px "IBM Plex Mono", monospace`;
      ctx.fillText("CAHOOTS BADGE  r1  ·  T4  ·  OPEN HARDWARE", w * 0.1, h * 0.06);
      ctx.fillText("TOUCH LINK  GND TX RX DET", w * 0.56, h * 0.5);
      ctx.fillText("NO RADIO", w * 0.1, h * 0.84);
    },
    1024,
    1024,
  );

  const chipTex = canvasTexture((ctx, w, h) => {
    ctx.fillStyle = "#0b0d0f";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "#9a9ea3";
    ctx.font = `500 ${w * 0.12}px "IBM Plex Mono", monospace`;
    ctx.fillText("BAOCHIP", w * 0.12, h * 0.42);
    ctx.fillText("1X", w * 0.12, h * 0.58);
    ctx.font = `400 ${w * 0.07}px "IBM Plex Mono", monospace`;
    ctx.fillText("RV32 · TRNG · RRAM", w * 0.12, h * 0.8);
    ctx.beginPath();
    ctx.arc(w * 0.14, h * 0.14, w * 0.035, 0, Math.PI * 2);
    ctx.fill();
  });

  const cellTex = canvasTexture((ctx, w, h) => {
    ctx.fillStyle = "#b9bcc0";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "#2a2d31";
    ctx.font = `500 ${w * 0.06}px "IBM Plex Mono", monospace`;
    ctx.fillText("LiPo 3.7 V 400 mAh", w * 0.08, h * 0.4);
    ctx.font = `400 ${w * 0.045}px "IBM Plex Mono", monospace`;
    ctx.fillText("protected · 503040", w * 0.08, h * 0.6);
  });

  const polycarbonate = new THREE.MeshPhysicalMaterial({
    color: 0xf2efe8,
    metalness: 0,
    roughness: 0.12,
    transmission: 0.92,
    thickness: 0.25,
    ior: 1.58,
    clearcoat: 1,
    clearcoatRoughness: 0.08,
    transparent: true,
    opacity: 1,
  });
  epaperTex.center.set(0.5, 0.5);
  epaperTex.rotation = -Math.PI / 2;
  const paperMat = new THREE.MeshStandardMaterial({ map: epaperTex, roughness: 0.98, metalness: 0 });
  const bezelMat = new THREE.MeshStandardMaterial({ color: 0x101214, roughness: 0.55 });
  const pcbMat = new THREE.MeshStandardMaterial({ map: pcbTex, roughness: 0.62, metalness: 0.15 });
  const pcbEdgeMat = new THREE.MeshStandardMaterial({ color: 0x3a3f33, roughness: 0.9 });
  const chipMat = new THREE.MeshStandardMaterial({ map: chipTex, roughness: 0.3, metalness: 0.1 });
  const chipSideMat = new THREE.MeshStandardMaterial({ color: 0x0b0d0f, roughness: 0.4 });
  const gold = new THREE.MeshStandardMaterial({ color: 0xd4af37, roughness: 0.25, metalness: 1 });
  const steel = new THREE.MeshStandardMaterial({ color: 0xaeb3b8, roughness: 0.35, metalness: 1 });
  const dark = new THREE.MeshStandardMaterial({ color: 0x24282c, roughness: 0.6, metalness: 0.4 });
  const cellMat = new THREE.MeshStandardMaterial({ map: cellTex, roughness: 0.45, metalness: 0.5 });
  const ribbonMat = new THREE.MeshStandardMaterial({ color: 0xb8742a, roughness: 0.5, metalness: 0.2 });
  const buttonMat = new THREE.MeshStandardMaterial({ color: 0xc8783c, roughness: 0.4 });

  const group = new THREE.Group();
  scene.add(group);
  const stack = [];
  function add(mesh, rest, explodedOffset, step) {
    mesh.position.y = rest;
    stack.push({ mesh, rest, out: rest + explodedOffset });
    mesh.userData.step = step;
    group.add(mesh);
    return mesh;
  }

  // Bottom shell: a tray with a floor and a lip. Step 0.
  const shellT = 1.6 * MM;
  const lanyard = () => slot(0, -(L / 2 - 6 * MM), 12 * MM, 3 * MM);
  const bottomShell = new THREE.Group();
  bottomShell.add(plate(rrect(W, L, CORNER, [lanyard()]), shellT, polycarbonate, 0.3 * MM));
  const lipOuter = rrect(W, L, CORNER, [rrect(W - 2 * shellT, L - 2 * shellT, CORNER - shellT)]);
  const lip = plate(lipOuter, 7 * MM, polycarbonate);
  lip.position.y = shellT;
  bottomShell.add(lip);
  add(bottomShell, -5.0 * MM, -34 * MM, 0);

  // Battery. Step 4.
  const cell = box(30 * MM, 4 * MM, 40 * MM, cellMat);
  cell.position.x = 0;
  cell.position.z = 6 * MM;
  add(cell, -3.2 * MM, -18 * MM, 4);

  // PCB with everything soldered to it. Step 2.
  const boardShape = rrect(W - 4 * MM, L - 4 * MM, CORNER - 1 * MM, [slot(0, -(L / 2 - 8 * MM), 12 * MM, 3 * MM)]);
  const pcb = plate(boardShape, 1.0 * MM, [pcbMat, pcbEdgeMat]);
  add(pcb, 1.0 * MM, 0, 2);
  // Chip: 7 × 7 mm.
  const chip = new THREE.Mesh(new THREE.BoxGeometry(7 * MM, 0.9 * MM, 7 * MM), [
    chipSideMat, chipSideMat, chipMat, chipSideMat, chipSideMat, chipSideMat,
  ]);
  chip.geometry.translate(0, 0.45 * MM, 0);
  chip.position.set(8 * MM, 0, 12 * MM);
  add(chip, 2.0 * MM, 14 * MM, 2);
  // Decoupling and the power path, small dark parts.
  for (const [x, z, w, d] of [[-9, -6, 3, 2], [-9, -1, 3, 2], [-4, -14, 2, 1.5], [12, -18, 4, 3]]) {
    const part = box(w * MM, 0.8 * MM, d * MM, dark);
    part.position.set(x * MM, 0, z * MM);
    add(part, 2.0 * MM, 0, 2);
  }
  // USB-C at the bottom edge. Step 4.
  const usb = box(9 * MM, 3.2 * MM, 7.5 * MM, steel);
  usb.position.set(0, 0, L / 2 - 6 * MM);
  add(usb, 2.0 * MM, -6 * MM, 4);
  // Confirm button, right of the display area.
  const button = new THREE.Mesh(new THREE.CylinderGeometry(2.5 * MM, 2.5 * MM, 2 * MM, 24), buttonMat);
  button.geometry.translate(0, 1 * MM, 0);
  button.position.set(-16 * MM, 0, 20 * MM);
  add(button, 2.0 * MM, 0, 2);

  // Touch link: four pogo pins on the right edge, four pads and two magnets on the left. Step 3.
  const link = new THREE.Group();
  for (let i = 0; i < 4; i++) {
    const z = (-6 + i * 4) * MM;
    const pin = new THREE.Mesh(new THREE.CylinderGeometry(0.75 * MM, 0.75 * MM, 5 * MM, 12), gold);
    pin.rotation.z = Math.PI / 2;
    pin.position.set(W / 2 - 3.5 * MM, 1.6 * MM, z);
    link.add(pin);
    const tip = new THREE.Mesh(new THREE.SphereGeometry(0.75 * MM, 12, 8), gold);
    tip.position.set(W / 2 - 1 * MM, 1.6 * MM, z);
    link.add(tip);
    const pad = box(3 * MM, 0.2 * MM, 2.2 * MM, gold);
    pad.position.set(-W / 2 + 3.5 * MM, 0, z);
    link.add(pad);
  }
  for (const z of [-11, 7]) {
    const magnet = new THREE.Mesh(new THREE.CylinderGeometry(2 * MM, 2 * MM, 1.5 * MM, 20), steel);
    magnet.geometry.translate(0, 0.75 * MM, 0);
    magnet.position.set(-W / 2 + 4 * MM, 0, z * MM);
    link.add(magnet);
    const magnetR = magnet.clone();
    magnetR.position.x = W / 2 - 4 * MM;
    link.add(magnetR);
  }
  add(link, 2.0 * MM, 24 * MM, 3);

  // E-paper: 1.54", 200 × 200, bezel 42 × 44 mm, active 37 × 37 mm, with its ribbon. Step 1.
  const display = new THREE.Group();
  const bezel = plate(rrect(42 * MM, 44 * MM, 1 * MM), 1.0 * MM, bezelMat);
  display.add(bezel);
  const panel = box(37 * MM, 0.3 * MM, 37 * MM, paperMat);
  panel.position.y = 1.0 * MM;
  display.add(panel);
  const ribbon = box(14 * MM, 0.15 * MM, 10 * MM, ribbonMat);
  ribbon.position.set(0, 0.2 * MM, 24 * MM);
  display.add(ribbon);
  display.position.z = -8 * MM;
  add(display, 3.2 * MM, 36 * MM, 1);

  // Top shell with a window over the display, plus four screws. Step 0.
  const topShell = new THREE.Group();
  topShell.add(plate(rrect(W, L, CORNER, [lanyard()]), shellT, polycarbonate, 0.3 * MM));
  for (const [sx, sz] of [[-1, -1], [1, -1], [-1, 1], [1, 1]]) {
    const screw = new THREE.Mesh(new THREE.CylinderGeometry(1.1 * MM, 1.1 * MM, 1.2 * MM, 16), steel);
    screw.geometry.translate(0, 0.6 * MM, 0);
    screw.position.set(sx * (W / 2 - 3.5 * MM), shellT, sz * (L / 2 - 14 * MM));
    topShell.add(screw);
    const recess = new THREE.Mesh(new THREE.CylinderGeometry(0.55 * MM, 0.55 * MM, 0.5 * MM, 6), dark);
    recess.position.set(screw.position.x, shellT + 1.0 * MM, screw.position.z);
    topShell.add(recess);
  }
  add(topShell, 5.5 * MM, 58 * MM, 0);

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

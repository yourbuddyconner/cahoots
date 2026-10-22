// The badge model, shared by the landing page and the launch video.
//
// The manufactured card: 54 × 86 mm and 2.9 mm thick, a clear polycarbonate tray with a clear top
// sheet. Nothing stacks inside except on the 0.4 mm board: a 1.54" e-paper, the chip, a CR2016
// coin cell dropped into a board cutout, two dome buttons under the sheet. Cards stack: four
// magnets in the floor line them up, four gold pads on the face meet four spring fingers under
// the card above, and the pile talks through them. A slot at the far end takes a lanyard.
//
// 1 mm = 0.05 units. Y is up, the card's underside is y = 0, +z is the contacts end.
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";
import * as BufferGeometryUtils from "three/addons/utils/BufferGeometryUtils.js";

export const MM = 0.05;
const u = (mm) => mm * MM;

// ---------- Outlines: closed polylines in the XZ plane, in mm, as a function of an offset ----------
function arc(cx, cz, r, a0, a1, n, out) {
  for (let i = 0; i <= n; i++) {
    const a = a0 + ((a1 - a0) * i) / n;
    out.push([cx + r * Math.cos(a), cz + r * Math.sin(a)]);
  }
  return out;
}
export const rrect = (w, h, r, seg = 10) => (off = 0) => {
  const W = w + 2 * off, H = h + 2 * off, R = Math.max(0.02, r + off);
  const p = [];
  arc(W / 2 - R, H / 2 - R, R, 0, Math.PI / 2, seg, p);
  arc(-W / 2 + R, H / 2 - R, R, Math.PI / 2, Math.PI, seg, p);
  arc(-W / 2 + R, -H / 2 + R, R, Math.PI, 1.5 * Math.PI, seg, p);
  arc(W / 2 - R, -H / 2 + R, R, 1.5 * Math.PI, 2 * Math.PI, seg, p);
  return p;
};
export const circle = (r, seg = 56) => (off = 0) => {
  const p = [];
  arc(0, 0, r + off, 0, 2 * Math.PI, seg, p);
  p.pop();
  return p;
};
// A slot: w long along x, h wide along z, round ends.
export const stadium = (w, h) => (off = 0) => {
  const R = h / 2 + off, c = w / 2 - h / 2;
  const p = [];
  arc(c, 0, R, -Math.PI / 2, Math.PI / 2, 12, p);
  arc(-c, 0, R, Math.PI / 2, 1.5 * Math.PI, 12, p);
  return p;
};
export const at = (fn, dx, dz) => (off = 0) => fn(off).map(([x, z]) => [x + dx, z + dz]);

// ---------- Slabs: an outline swept with a side profile, so edges get real fillets ----------
// Profile points: [offset from the outline (mm, + outward), height (mm), normal offset, normal y].
function profile(t, rTop, rBot, k) {
  const p = [];
  if (rTop > 0) {
    for (let i = 0; i <= k; i++) {
      const a = Math.PI / 2 - ((Math.PI / 2) * i) / k;
      p.push([-rTop + rTop * Math.cos(a), t - rTop + rTop * Math.sin(a), Math.cos(a), Math.sin(a)]);
    }
  } else p.push([0, t, 0, 1], [0, t, 1, 0]);
  if (rBot > 0) {
    for (let i = 0; i <= k; i++) {
      const a = -((Math.PI / 2) * i) / k;
      p.push([-rBot + rBot * Math.cos(a), rBot + rBot * Math.sin(a), Math.cos(a), Math.sin(a)]);
    }
  } else p.push([0, 0, 1, 0], [0, 0, 0, -1]);
  return p;
}

function sweep(outline, prof, inward) {
  const base = outline(0), n = base.length;
  let area = 0;
  for (let i = 0; i < n; i++) {
    const [x0, z0] = base[i], [x1, z1] = base[(i + 1) % n];
    area += x0 * z1 - x1 * z0;
  }
  const sgn = (area > 0 ? 1 : -1) * (inward ? -1 : 1);
  const N = [], len = [0];
  for (let i = 0; i < n; i++) {
    const [px, pz] = base[(i + n - 1) % n], [cx, cz] = base[i], [qx, qz] = base[(i + 1) % n];
    const l1 = Math.hypot(cx - px, cz - pz), l2 = Math.hypot(qx - cx, qz - cz);
    const t1x = (cx - px) / l1, t1z = (cz - pz) / l1, t2x = (qx - cx) / l2, t2z = (qz - cz) / l2;
    const n1x = t1z * sgn, n1z = -t1x * sgn, n2x = t2z * sgn, n2z = -t2x * sgn;
    let mx = n1x + n2x, mz = n1z + n2z;
    const ml = Math.hypot(mx, mz);
    mx /= ml; mz /= ml;
    const d = mx * n1x + mz * n1z; // miter: keeps the offset distance right at corners
    N.push([mx / d, mz / d, mx, mz]);
    len.push(len[i] + l2);
  }
  const m = prof.length, cols = n + 1;
  const pos = [], nor = [], uv = [], idx = [];
  for (let j = 0; j < m; j++) {
    const [o, y, no, ny] = prof[j];
    for (let c = 0; c < cols; c++) {
      const i = c % n;
      const [x, z] = base[i], [ox, oz, nx, nz] = N[i];
      pos.push(u(x + ox * o), u(y), u(z + oz * o));
      nor.push(nx * no, ny, nz * no);
      uv.push(len[c] / len[n], j / (m - 1));
    }
  }
  for (let j = 0; j < m - 1; j++) {
    for (let c = 0; c < n; c++) {
      const a = j * cols + c, b = a + 1, d = a + cols, e = d + 1;
      idx.push(a, d, b, b, d, e);
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  g.setAttribute("normal", new THREE.Float32BufferAttribute(nor, 3));
  g.setAttribute("uv", new THREE.Float32BufferAttribute(uv, 2));
  g.setIndex(idx);
  // Face the triangles the way the normals point.
  const P = g.attributes.position, Nn = g.attributes.normal, I = g.index.array;
  const A = new THREE.Vector3(), B = new THREE.Vector3(), C = new THREE.Vector3(), X = new THREE.Vector3();
  for (let t = 0; t < I.length; t += 3) {
    A.fromBufferAttribute(P, I[t]); B.fromBufferAttribute(P, I[t + 1]); C.fromBufferAttribute(P, I[t + 2]);
    X.subVectors(B, A).cross(C.sub(A));
    if (X.lengthSq() < 1e-14) continue;
    if (X.dot(A.fromBufferAttribute(Nn, I[t])) < 0) {
      for (let s = 0; s < I.length; s += 3) { const tmp = I[s + 1]; I[s + 1] = I[s + 2]; I[s + 2] = tmp; }
    }
    break;
  }
  return g;
}

function cap(outline, holes, r, y, up) {
  const f = up ? -1 : 1;
  const s = new THREE.Shape();
  outline(-r).forEach(([x, z], i) => (i ? s.lineTo(u(x), f * u(z)) : s.moveTo(u(x), f * u(z))));
  for (const h of holes) {
    const path = new THREE.Path();
    h(r).forEach(([x, z], i) => (i ? path.lineTo(u(x), f * u(z)) : path.moveTo(u(x), f * u(z))));
    s.holes.push(path);
  }
  const g = new THREE.ShapeGeometry(s);
  g.rotateX(up ? -Math.PI / 2 : Math.PI / 2);
  g.translate(0, u(y), 0);
  return g;
}

/** A slab of thickness t (mm) with filleted top and bottom edges. Groups: 0 sides, 1 top, 2 bottom. */
export function slab({ outline, holes = [], t, rTop = 0, rBot = 0, k = 6, top = true, bottom = true }) {
  const prof = profile(t, rTop, rBot, k);
  const sides = [sweep(outline, prof, false), ...holes.map((h) => sweep(h, prof, true))];
  const parts = [sides.length > 1 ? BufferGeometryUtils.mergeGeometries(sides) : sides[0]];
  if (top) parts.push(cap(outline, holes, rTop, t, true));
  if (bottom) parts.push(cap(outline, holes, rBot, 0, false));
  return BufferGeometryUtils.mergeGeometries(parts, true);
}

/** A spherical cap h tall with a rim of radius r, rim at y = 0. */
function dome(r, h, seg = 48) {
  const R = (r * r + h * h) / (2 * h);
  const g = new THREE.SphereGeometry(u(R), seg, 10, 0, Math.PI * 2, 0, Math.asin(r / R));
  g.translate(0, -u(R - h), 0);
  return g;
}

function rbox(w, h, d, r, mat, seg = 3) {
  const m = new THREE.Mesh(new RoundedBoxGeometry(u(w), u(h), u(d), seg, u(r)), mat);
  m.geometry.translate(0, u(h) / 2, 0);
  return m;
}

// ---------- Textures ----------
function canvasTexture(renderer, w, h, draw, srgb = true) {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  draw(c.getContext("2d"), w, h);
  const t = new THREE.CanvasTexture(c);
  if (srgb) t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = Math.min(16, renderer.capabilities.getMaxAnisotropy());
  return t;
}
// Planar mapping for a cap: uv = (x, −z) in scene units, so a w × h mm image covers the part.
function planar(tex, w, h, cx = 0, cz = 0) {
  tex.repeat.set(1 / u(w), 1 / u(h));
  tex.offset.set(0.5 - cx / w, 0.5 + cz / h);
  tex.wrapS = tex.wrapT = THREE.ClampToEdgeWrapping;
  return tex;
}
const MONO = '"JetBrains Mono", "IBM Plex Mono", ui-monospace, monospace';
let seed = 11;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);

// The e-paper: the whole glass (32 × 37.5 mm), with the 200 × 200 active area drawn at native
// resolution and scaled up without smoothing, so it has the pixel grid of the real thing.
function epaperTexture(renderer, words) {
  const S = 64, GW = 32, GH = 37.5, A = 27.6, a0 = 2.2;
  return canvasTexture(renderer, GW * S, GH * S, (ctx, w, h) => {
    ctx.fillStyle = "#c9c7c0";
    ctx.fillRect(0, 0, w, h);
    // Driver ledge: the chip-on-glass strip and the bond area.
    ctx.fillStyle = "#73767a";
    ctx.fillRect(0, (a0 + A + 1.0) * S, w, h);
    ctx.fillStyle = "#1a1c1e";
    ctx.fillRect(3 * S, (a0 + A + 3.2) * S, w - 6 * S, 1.3 * S);
    ctx.strokeStyle = "#b08d3c";
    ctx.lineWidth = 2;
    for (let i = 0; i < 36; i++) {
      const x = (3.6 + i * 0.7) * S;
      ctx.beginPath(); ctx.moveTo(x, (a0 + A + 4.5) * S); ctx.lineTo(x, (GH - 0.6) * S); ctx.stroke();
    }
    // Active area, drawn at 200 × 200.
    const e = document.createElement("canvas");
    e.width = e.height = 200;
    const g = e.getContext("2d");
    g.fillStyle = "#e9e6de";
    g.fillRect(0, 0, 200, 200);
    g.fillStyle = "#1b1d1f";
    g.font = `500 11px ${MONO}`;
    g.fillText("sf-meetup", 8, 19);
    g.textAlign = "right";
    g.font = `400 9px ${MONO}`;
    g.fillText("TRNG · T4", 192, 19);
    g.textAlign = "left";
    g.fillRect(8, 26, 184, 1);
    g.font = `600 19px ${MONO}`;
    words.forEach((word, i) => g.fillText(word, 8 + (i % 2) * 96, 62 + Math.floor(i / 2) * 36));
    g.font = `400 9px ${MONO}`;
    g.fillText("read aloud · B to confirm", 8, 188);
    g.fillRect(8, 170, 184, 1);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(e, a0 * S, a0 * S, A * S, A * S);
    ctx.imageSmoothingEnabled = true;
    // The thin black frame every e-paper has at the edge of its active area.
    ctx.strokeStyle = "#2a2c2e";
    ctx.lineWidth = 0.18 * S;
    ctx.strokeRect(a0 * S, a0 * S, A * S, A * S);
  });
}

// The board: black solder mask, gold where copper is exposed, white silkscreen. Drawn twice, once
// in color and once as a roughness/metalness map (green = roughness, blue = metalness).
const BOARD = { w: 51.8, h: 76, cz: 4 }; // centred at x = 0, z = 4 on the card
function boardArt(ctx, w, h, pal, chip, parts, domes, pads, magnets) {
  const S = w / BOARD.w;
  const X = (x) => (x + BOARD.w / 2) * S, Z = (z) => (z - BOARD.cz + BOARD.h / 2) * S;
  ctx.fillStyle = pal.mask;
  ctx.fillRect(0, 0, w, h);
  if (pal.speckle) {
    for (let i = 0; i < 9000; i++) {
      ctx.fillStyle = rnd() < 0.5 ? "rgba(255,255,255,0.025)" : "rgba(0,0,0,0.05)";
      ctx.fillRect(rnd() * w, rnd() * h, 3, 3);
    }
  }
  // Traces: from the chip out to every part, with the 45° kinks of a real layout.
  ctx.strokeStyle = pal.trace;
  ctx.lineWidth = 0.2 * S;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const route = (x0, z0, x1, z1) => {
    const dx = x1 - x0, dz = z1 - z0, k = Math.min(Math.abs(dx), Math.abs(dz));
    ctx.beginPath();
    ctx.moveTo(X(x0), Z(z0));
    ctx.lineTo(X(x0 + Math.sign(dx) * k), Z(z0 + Math.sign(dz) * k));
    ctx.lineTo(X(x1), Z(z1));
    ctx.stroke();
  };
  for (const p of parts) route(chip.x + (p.x > chip.x ? 3.5 : -3.5), chip.z + (rnd() - 0.5) * 5, p.x, p.z);
  for (const d of domes) route(chip.x + 3.5, chip.z - 1, d.x, d.z);
  for (const p of pads) route(chip.x + (p.x > chip.x ? 3.5 : -3.5), chip.z + 2.5, p.x, p.z);
  for (let i = 0; i < 14; i++) route(chip.x - 3.5, chip.z - 3 + i * 0.5, -24 + rnd() * 3, -30 + i * 2.6);
  // Vias.
  for (let i = 0; i < 90; i++) {
    const x = -24 + rnd() * 48, z = -32 + rnd() * 72;
    if (Math.hypot(x, z - 27) < 11.5) continue;
    ctx.fillStyle = pal.gold;
    ctx.beginPath(); ctx.arc(X(x), Z(z), 0.3 * S, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = pal.goldCore;
    ctx.beginPath(); ctx.arc(X(x), Z(z), 0.12 * S, 0, Math.PI * 2); ctx.fill();
  }
  // Exposed copper: the chip's QFN footprint, passive pads, dome pads, the contact vias.
  ctx.fillStyle = pal.gold;
  for (let i = 0; i < 12; i++) {
    const o = -2.75 + i * 0.5;
    ctx.fillRect(X(chip.x + o - 0.14), Z(chip.z - 3.9), 0.28 * S, 0.8 * S);
    ctx.fillRect(X(chip.x + o - 0.14), Z(chip.z + 3.1), 0.28 * S, 0.8 * S);
    ctx.fillRect(X(chip.x - 3.9), Z(chip.z + o - 0.14), 0.8 * S, 0.28 * S);
    ctx.fillRect(X(chip.x + 3.1), Z(chip.z + o - 0.14), 0.8 * S, 0.28 * S);
  }
  ctx.fillRect(X(chip.x - 2.2), Z(chip.z - 2.2), 4.4 * S, 4.4 * S);
  for (const p of parts) {
    const along = p.rot ? [0, 1] : [1, 0];
    for (const s of [-1, 1]) {
      const cx = p.x + s * along[0] * (p.w / 2 + 0.1), cz = p.z + s * along[1] * (p.w / 2 + 0.1);
      const pw = p.rot ? p.d + 0.2 : 0.6, pd = p.rot ? 0.6 : p.d + 0.2;
      ctx.fillRect(X(cx - pw / 2), Z(cz - pd / 2), pw * S, pd * S);
    }
  }
  for (const d of domes) {
    ctx.beginPath(); ctx.arc(X(d.x), Z(d.z), 1.0 * S, 0, Math.PI * 2); ctx.fill();
    ctx.lineWidth = 0.5 * S;
    ctx.strokeStyle = pal.gold;
    ctx.beginPath(); ctx.arc(X(d.x), Z(d.z), 2.3 * S, 0, Math.PI * 2); ctx.stroke();
  }
  for (const p of pads) { ctx.beginPath(); ctx.arc(X(p.x), Z(p.z), 0.9 * S, 0, Math.PI * 2); ctx.fill(); }
  // Silkscreen.
  ctx.fillStyle = pal.silk;
  ctx.font = `500 ${1.1 * S}px ${MONO}`;
  ctx.fillText("CAHOOTS BADGE  r2", X(-24), Z(-31.2));
  ctx.fillText("OPEN HARDWARE · NO RADIO", X(-24), Z(-29.4));
  ctx.font = `500 ${0.9 * S}px ${MONO}`;
  ctx.fillText("U1 BAOCHIP-1X", X(chip.x - 4.5), Z(chip.z + 5.6));
  ctx.fillText("B1  CR2016  3 V", X(-8), Z(40.2));
  ctx.fillText("J1  1  2  3  4", X(-12.5), Z(38.6));
  ctx.fillText("A", X(domes[0].x - 0.5), Z(domes[0].z + 4.4));
  ctx.fillText("B", X(domes[1].x - 0.5), Z(domes[1].z + 4.4));
  ctx.fillText("+", X(10.4), Z(18.2));
  ctx.lineWidth = 0.15 * S;
  ctx.strokeStyle = pal.silk;
  ctx.strokeRect(X(chip.x - 3.9), Z(chip.z - 3.9), 7.8 * S, 7.8 * S);
  for (const d of domes) { ctx.beginPath(); ctx.arc(X(d.x), Z(d.z), 2.9 * S, 0, Math.PI * 2); ctx.stroke(); }
  for (const m of magnets) { ctx.beginPath(); ctx.arc(X(m.x), Z(m.z), 2.1 * S, 0, Math.PI * 2); ctx.stroke(); }
  // The mark: three rings.
  for (const r of [0.6, 1.2, 1.8]) { ctx.beginPath(); ctx.arc(X(22), Z(-30.3), r * S, 0, Math.PI * 2); ctx.stroke(); }
}

export function makeBadge(renderer, { words = ["sunset", "valve", "present", "raven", "curtain", "pudding"] } = {}) {
  const W = 54, L = 86, R = 3.2, T = 2.9;
  const FLOOR = 0.8, WALL = 0.6, SHEET = 0.3, CAVITY = T - FLOOR - SHEET; // 1.8
  const BOARD_Y = FLOOR, BOARD_T = 0.4, TOP = BOARD_Y + BOARD_T; // parts sit at y = 1.2
  const chip = { x: -13, z: 12 };
  const domes = [{ x: 13, z: 13 }, { x: 21, z: 13 }];
  const pads = [-9, -3, 3, 9].map((x) => ({ x, z: 40 }));
  const magnets = [[-21, -39], [21, -39], [-21, 39], [21, 39]].map(([x, z]) => ({ x, z }));
  const cell = { x: 0, z: 27, r: 10, t: 1.6 };
  const parts = [
    [-18.5, 8.5], [-18.5, 10], [-18.5, 11.5], [-7.5, 9], [-7.5, 10.5], [-7.5, 12], [-13, 17.4, 1], [-11.2, 17.4, 1],
    [-9.4, 17.4, 1], [-16.6, 17.4, 1], [6, 8.6, 1], [8, 8.6, 1], [10, 8.6, 1], [-20, 30], [-20, 32], [-14.5, 36.5, 1],
    [-14.5, 38.2, 1], [15.5, 30], [15.5, 32], [15.5, 34], [21, 24], [21, 26], [-21.5, 21], [-21.5, 23], [5, 15.2, 1], [3, 15.2, 1],
  ].map(([x, z, rot]) => ({ x, z, w: 1.0, d: 0.5, rot: !!rot }));

  // ----- textures -----
  const epaperTex = planar(epaperTexture(renderer, words), 32, 37.5);
  const pal = { mask: "#15171b", trace: "#1f2329", gold: "#d9b25a", goldCore: "#3b2f12", silk: "#e3e0d6", speckle: true };
  const orm = { mask: "rgb(255,135,0)", trace: "rgb(255,120,0)", gold: "rgb(255,70,255)", goldCore: "rgb(255,120,255)", silk: "rgb(255,215,0)" };
  const BW = 1536, BH = Math.round((BOARD.h / BOARD.w) * 1536);
  const boardTex = planar(canvasTexture(renderer, BW, BH, (ctx, w, h) => boardArt(ctx, w, h, pal, chip, parts, domes, pads, magnets)), BOARD.w, BOARD.h, 0, BOARD.cz);
  const boardOrm = planar(canvasTexture(renderer, BW, BH, (ctx, w, h) => boardArt(ctx, w, h, orm, chip, parts, domes, pads, magnets), false), BOARD.w, BOARD.h, 0, BOARD.cz);
  const chipTex = canvasTexture(renderer, 512, 512, (ctx, w, h) => {
    ctx.fillStyle = "#0d0f11";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "#9da1a6";
    ctx.font = `500 ${w * 0.13}px ${MONO}`;
    ctx.fillText("BAOCHIP", w * 0.12, h * 0.4);
    ctx.fillText("1X", w * 0.12, h * 0.56);
    ctx.font = `400 ${w * 0.065}px ${MONO}`;
    ctx.fillText("RV32 · TRNG · RRAM", w * 0.12, h * 0.74);
    ctx.fillText("2627  TW", w * 0.12, h * 0.84);
    ctx.beginPath(); ctx.arc(w * 0.13, h * 0.14, w * 0.035, 0, Math.PI * 2); ctx.fill();
  });
  const cellTex = canvasTexture(renderer, 512, 512, (ctx, w, h) => {
    ctx.fillStyle = "#b4b6b9";
    ctx.fillRect(0, 0, w, h);
    // Brushed rings.
    for (let r = 8; r < w / 2; r += 2) {
      ctx.strokeStyle = rnd() < 0.5 ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.06)";
      ctx.beginPath(); ctx.arc(w / 2, h / 2, r, 0, Math.PI * 2); ctx.stroke();
    }
    ctx.strokeStyle = "rgba(0,0,0,0.25)";
    ctx.lineWidth = 3;
    ctx.beginPath(); ctx.arc(w / 2, h / 2, w * 0.44, 0, Math.PI * 2); ctx.stroke();
    ctx.fillStyle = "#4f5256";
    ctx.textAlign = "center";
    ctx.font = `600 ${w * 0.16}px ${MONO}`;
    ctx.fillText("+", w / 2, h * 0.42);
    ctx.font = `500 ${w * 0.075}px ${MONO}`;
    ctx.fillText("CR2016", w / 2, h * 0.6);
    ctx.font = `400 ${w * 0.055}px ${MONO}`;
    ctx.fillText("LITHIUM 3V", w / 2, h * 0.7);
  });
  // Second-surface print under the clear sheet: button rings, pad frames, the name.
  const printTex = canvasTexture(renderer, 1080, 1720, (ctx, w, h) => {
    const S = w / W, X = (x) => (x + W / 2) * S, Z = (z) => (z + L / 2) * S;
    ctx.clearRect(0, 0, w, h);
    ctx.strokeStyle = "#d8d5cc";
    ctx.fillStyle = "#d8d5cc";
    ctx.lineWidth = 0.22 * S;
    ctx.textAlign = "center";
    ctx.font = `500 ${1.9 * S}px ${MONO}`;
    for (const [d, label] of [[domes[0], "A"], [domes[1], "B"]]) {
      ctx.beginPath(); ctx.arc(X(d.x), Z(d.z), 3.0 * S, 0, Math.PI * 2); ctx.stroke();
      ctx.fillText(label, X(d.x), Z(d.z + 0.7));
    }
    ctx.lineWidth = 0.15 * S;
    ctx.textAlign = "left";
    ctx.font = `500 ${1.3 * S}px ${MONO}`;
    ctx.fillText("cahoots", X(-24.5), Z(41.3));
    ctx.textAlign = "right";
    ctx.font = `400 ${1.1 * S}px ${MONO}`;
    ctx.fillText("open hardware · no radio", X(24.5), Z(41.3));
  });
  // Smudges and fine scratches: a roughness map tiled over the clear plastic.
  const smudge = canvasTexture(renderer, 512, 512, (ctx, w, h) => {
    ctx.fillStyle = "rgb(14,14,14)";
    ctx.fillRect(0, 0, w, h);
    for (let i = 0; i < 26; i++) {
      const x = rnd() * w, y = rnd() * h, r = 30 + rnd() * 120;
      const g = ctx.createRadialGradient(x, y, 0, x, y, r);
      g.addColorStop(0, `rgba(70,70,70,${0.25 + rnd() * 0.35})`);
      g.addColorStop(1, "rgba(70,70,70,0)");
      ctx.fillStyle = g;
      ctx.beginPath(); ctx.ellipse(x, y, r, r * (0.5 + rnd() * 0.5), rnd() * Math.PI, 0, Math.PI * 2); ctx.fill();
    }
    ctx.strokeStyle = "rgba(90,90,90,0.5)";
    ctx.lineWidth = 1;
    for (let i = 0; i < 40; i++) {
      const x = rnd() * w, y = rnd() * h, a = rnd() * Math.PI, l = 20 + rnd() * 90;
      ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(x + Math.cos(a) * l, y + Math.sin(a) * l); ctx.stroke();
    }
  }, false);
  smudge.wrapS = smudge.wrapT = THREE.RepeatWrapping;
  smudge.repeat.set(0.45, 0.45);

  // ----- materials -----
  const polycarbonate = new THREE.MeshPhysicalMaterial({
    color: 0xffffff,
    metalness: 0,
    roughness: 1,
    roughnessMap: smudge,
    transmission: 1,
    thickness: u(0.6),
    attenuationColor: new THREE.Color(0xf4ecdc),
    attenuationDistance: u(60),
    ior: 1.585,
    specularIntensity: 1,
    envMapIntensity: 1.5,
    clearcoat: 0.6,
    clearcoatRoughness: 0.04,
    transparent: true,
  });
  const paperMat = new THREE.MeshPhysicalMaterial({ map: epaperTex, roughness: 0.95, metalness: 0, clearcoat: 1, clearcoatRoughness: 0.08 });
  const glassEdge = new THREE.MeshStandardMaterial({ color: 0x2e3033, roughness: 0.25, metalness: 0 });
  const boardMat = new THREE.MeshStandardMaterial({ map: boardTex, roughnessMap: boardOrm, metalnessMap: boardOrm, roughness: 1, metalness: 1 });
  const boardBack = new THREE.MeshStandardMaterial({ color: 0x15171b, roughness: 0.55 });
  const boardEdge = new THREE.MeshStandardMaterial({ color: 0x3c4136, roughness: 0.95 });
  const chipTop = new THREE.MeshStandardMaterial({ map: chipTex, roughness: 0.38 });
  const chipSide = new THREE.MeshStandardMaterial({ color: 0x0d0f11, roughness: 0.45 });
  const gold = new THREE.MeshStandardMaterial({ color: 0xe6c067, roughness: 0.22, metalness: 1 });
  const nickel = new THREE.MeshStandardMaterial({ color: 0xc6c8cb, roughness: 0.3, metalness: 1 });
  const steel = new THREE.MeshStandardMaterial({ color: 0xb9bcc0, roughness: 0.28, metalness: 1 });
  const cellTop = new THREE.MeshStandardMaterial({ map: cellTex, roughness: 0.42, metalness: 1 });
  const tin = new THREE.MeshStandardMaterial({ color: 0xcfd2d5, roughness: 0.4, metalness: 1 });
  const ceramic = new THREE.MeshStandardMaterial({ color: 0xa79c86, roughness: 0.6 });
  const resistor = new THREE.MeshStandardMaterial({ color: 0x1f2124, roughness: 0.5 });
  const plastic = new THREE.MeshStandardMaterial({ color: 0x2a2c2f, roughness: 0.55 });
  const ribbonMat = new THREE.MeshStandardMaterial({ color: 0xc08a3a, roughness: 0.45, metalness: 0.15, side: THREE.DoubleSide });
  const printMat = new THREE.MeshStandardMaterial({ map: printTex, transparent: true, alphaTest: 0.4, roughness: 0.7, side: THREE.DoubleSide });

  const group = new THREE.Group();
  const stack = [];
  function add(mesh, rest, out, step) {
    mesh.traverse((m) => {
      if (!m.isMesh) return;
      m.receiveShadow = true;
      m.castShadow = m.material !== polycarbonate && m.material !== printMat;
    });
    mesh.position.y = u(rest);
    mesh.userData.step = step;
    stack.push({ mesh, rest: u(rest), out: u(rest + out) });
    group.add(mesh);
    return mesh;
  }
  const outline = rrect(W, L, R);
  const slot = at(stadium(12, 3), 0, -(L / 2 - 5));
  const magnetHoles = magnets.map((m) => at(circle(1.5), m.x, m.z));
  const fingerHoles = pads.map((p) => at(rrect(2.6, 2.6, 0.6), p.x, p.z));
  const padHoles = pads.map((p) => at(rrect(3.04, 2.24, 0.42), p.x, p.z));

  // ----- Tray: floor, wall, the boss around the lanyard slot, and the four magnets. Step 0. -----
  const tray = new THREE.Group();
  tray.add(new THREE.Mesh(slab({ outline, holes: [slot, ...magnetHoles, ...fingerHoles], t: FLOOR, rBot: 0.5 }), polycarbonate));
  const wall = new THREE.Mesh(slab({ outline, holes: [rrect(W - 2 * WALL, L - 2 * WALL, R - WALL)], t: CAVITY, rTop: 0.2, bottom: false }), polycarbonate);
  wall.position.y = u(FLOOR);
  tray.add(wall);
  const boss = new THREE.Mesh(slab({ outline: at(stadium(13.2, 4.2), 0, -(L / 2 - 5)), holes: [slot], t: CAVITY, bottom: false }), polycarbonate);
  boss.position.y = u(FLOOR);
  tray.add(boss);
  for (const m of magnets) {
    const mag = new THREE.Mesh(slab({ outline: circle(1.5), t: 1.0, rTop: 0.12, rBot: 0.12 }), nickel);
    mag.position.set(u(m.x), 0, u(m.z));
    tray.add(mag);
  }
  add(tray, 0, -16, 0);

  // ----- Coin cell, in its cutout, on the floor. Step 4. -----
  const coin = new THREE.Mesh(slab({ outline: circle(cell.r), t: cell.t, rTop: 0.25, rBot: 0.25 }), [nickel, cellTop, nickel]);
  coin.position.set(u(cell.x), 0, u(cell.z));
  cellTex.center.set(0.5, 0.5);
  planar(cellTex, 2 * cell.r, 2 * cell.r, cell.x, cell.z);
  add(coin, FLOOR, -7, 4);

  // ----- Board with everything soldered to it. Step 2. -----
  const board = new THREE.Group();
  const boardOutline = at(rrect(BOARD.w, BOARD.h, 2.4), 0, BOARD.cz);
  const boardHoles = [at(circle(cell.r + 0.3), cell.x, cell.z), ...magnets.filter((m) => m.z > 0).map((m) => at(circle(1.7), m.x, m.z))];
  board.add(new THREE.Mesh(slab({ outline: boardOutline, holes: boardHoles, t: BOARD_T, rTop: 0.05, rBot: 0.05, k: 2 }), [boardEdge, boardMat, boardBack]));
  const onBoard = (mesh, x, z, y = BOARD_T) => { mesh.position.set(u(x), u(y), u(z)); board.add(mesh); return mesh; };
  const chipMesh = rbox(7, 0.9, 7, 0.12, [chipSide, chipSide, chipTop, chipSide, chipSide, chipSide]);
  onBoard(chipMesh, chip.x, chip.z);
  for (const p of parts) {
    const body = rbox(p.w, 0.5, p.d, 0.06, rnd() < 0.45 ? resistor : ceramic, 2);
    const part = new THREE.Group();
    part.add(body);
    for (const s of [-1, 1]) {
      const end = rbox(0.26, 0.52, p.d + 0.02, 0.05, tin, 2);
      end.position.x = s * (p.w / 2 - 0.08) * MM;
      part.add(end);
    }
    if (p.rot) part.rotation.y = Math.PI / 2;
    onBoard(part, p.x, p.z);
  }
  onBoard(rbox(3.2, 0.8, 2.5, 0.15, steel), -5, 15.5); // crystal
  onBoard(rbox(2.0, 1.0, 2.0, 0.1, plastic), -19.5, 14.2); // inductor
  onBoard(rbox(2.9, 1.0, 1.4, 0.1, chipSide), -3.5, 20.2); // regulator
  onBoard(rbox(7.0, 1.2, 3.5, 0.12, plastic), 0, 10.2); // display connector
  for (const d of domes) {
    const m = new THREE.Mesh(dome(2.5, 0.35), steel);
    onBoard(m, d.x, d.z);
  }
  // Spring fingers: through the floor, a gold dome under the card.
  for (const p of pads) {
    const f = new THREE.Group();
    const stem = new THREE.Mesh(slab({ outline: circle(1.0), t: BOARD_Y, rTop: 0, rBot: 0.1 }), gold);
    stem.position.y = -u(BOARD_Y);
    f.add(stem);
    const tip = new THREE.Mesh(dome(1.0, 0.4), gold);
    tip.rotation.x = Math.PI;
    tip.position.y = -u(BOARD_Y);
    f.add(tip);
    f.userData.step = 3;
    onBoard(f, p.x, p.z, 0);
  }
  // The clip that holds the cell until the sheet goes on.
  const clip = new THREE.Group();
  const strap = rbox(26, 0.2, 1.6, 0.08, steel, 2);
  strap.position.y = u(cell.t - BOARD_T - 0.2);
  clip.add(strap);
  for (const s of [-1, 1]) {
    const foot = rbox(2.5, cell.t - BOARD_T, 1.6, 0.08, steel, 2);
    foot.position.x = u(s * 12.5);
    clip.add(foot);
  }
  onBoard(clip, cell.x, cell.z);
  add(board, BOARD_Y, 0, 2);

  // ----- Pads on the face, flush with the sheet. Step 3. -----
  const padGroup = new THREE.Group();
  for (const p of pads) {
    const pad = new THREE.Mesh(slab({ outline: rrect(3.0, 2.2, 0.4), t: SHEET, rTop: 0.06 }), gold);
    pad.position.set(u(p.x), 0, u(p.z));
    padGroup.add(pad);
  }
  add(padGroup, FLOOR + CAVITY, 7, 3);

  // ----- E-paper: the glass on the board, the ribbon to its connector. Step 1. -----
  const display = new THREE.Group();
  const glass = new THREE.Mesh(slab({ outline: at(rrect(32, 37.5, 0.5), 0, -11), t: 1.1, rTop: 0.08, rBot: 0.08, k: 2 }), [glassEdge, paperMat, glassEdge]);
  planar(epaperTex, 32, 37.5, 0, -11);
  display.add(glass);
  const ribbon = rbox(10, 0.12, 5.5, 0.05, ribbonMat, 1);
  ribbon.position.set(0, u(1.0), u(9));
  display.add(ribbon);
  add(display, TOP, 14, 1);

  // ----- Top sheet and its print. Step 0. -----
  const sheet = new THREE.Group();
  sheet.add(new THREE.Mesh(slab({ outline, holes: [slot, ...padHoles], t: SHEET, rTop: 0.15 }), polycarbonate));
  const print = new THREE.Mesh(new THREE.PlaneGeometry(u(W), u(L)), printMat);
  print.rotation.x = -Math.PI / 2;
  print.position.y = -u(0.01);
  sheet.add(print);
  add(sheet, FLOOR + CAVITY, 26, 0);

  /** Set how far apart the parts are: 0 assembled, 1 fully exploded. */
  function explode(e) {
    for (const p of stack) p.mesh.position.y = p.rest + (p.out - p.rest) * e;
  }
  return {
    group, stack, explode,
    materials: { polycarbonate, paperMat, gold },
    MM, W: u(W), L: u(L), T: u(T),
    /** Card-to-card pitch in a pile: the thickness plus the standoff the magnets leave. */
    PITCH: u(T + 0.3),
  };
}

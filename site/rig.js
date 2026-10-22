// Rendering rig shared by the landing page and the launch video: a procedural studio for
// reflections, one shadow-casting key light with a warm rim, a shadow-catching ground, a backdrop
// that comes out of tone mapping as exactly the page colour, and a composer with ambient occlusion.
import * as THREE from "three";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { GTAOPass } from "three/addons/postprocessing/GTAOPass.js";
import { SMAAPass } from "three/addons/postprocessing/SMAAPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";

/** A small studio: a big softbox up and to the left, a tall amber strip to the right and behind,
 *  a cool fill on the left, dark walls. Prefiltered into an environment map; no file to fetch. */
export function makeStudio(renderer) {
  const room = new THREE.Scene();
  room.add(new THREE.Mesh(new THREE.BoxGeometry(40, 40, 40), new THREE.MeshBasicMaterial({ color: 0x33363b, side: THREE.BackSide })));
  const panel = (w, h, color, intensity, x, y, z) => {
    const m = new THREE.Mesh(new THREE.PlaneGeometry(w, h), new THREE.MeshBasicMaterial({ color: new THREE.Color(color).multiplyScalar(intensity), side: THREE.DoubleSide }));
    m.position.set(x, y, z);
    m.lookAt(0, 0, 0);
    room.add(m);
  };
  panel(12, 7, 0xfff3e2, 9, -7, 12, 7); // key softbox
  panel(1.2, 16, 0xffb45a, 7, 12, 4, -7); // amber strip, the rim
  panel(6, 6, 0xd9e1ec, 1.6, -12, 2, -5); // cool fill
  panel(16, 16, 0x8a8f96, 0.35, 0, -14, 0); // a pale floor, for the underside
  panel(14, 14, 0x9aa0a8, 0.22, 2, 16, -2); // a dim ceiling, so metal has something to reflect
  const pmrem = new THREE.PMREMGenerator(renderer);
  const env = pmrem.fromScene(room, 0.03).texture;
  pmrem.dispose();
  room.traverse((o) => { o.geometry?.dispose(); o.material?.dispose(); });
  return env;
}

/** Sets the studio as the scene's environment. The url is accepted for the old call sites and ignored. */
export async function setupEnvironment(renderer, scene, _url, intensity = 1.0) {
  scene.environment = makeStudio(renderer);
  scene.environmentIntensity = intensity;
  return scene.environment;
}

export function addLights(scene) {
  const key = new THREE.DirectionalLight(0xfff1e0, 2.0);
  key.position.set(-5, 9, 6);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.camera.near = 1;
  key.shadow.camera.far = 40;
  key.shadow.camera.left = key.shadow.camera.bottom = -6;
  key.shadow.camera.right = key.shadow.camera.top = 6;
  key.shadow.bias = -0.0003;
  key.shadow.normalBias = 0.012;
  key.shadow.radius = 3;
  scene.add(key);
  const rim = new THREE.DirectionalLight(0xe5a33a, 0.7);
  rim.position.set(6, 3, -6);
  scene.add(rim);
  const fill = new THREE.HemisphereLight(0xcfd3d8, 0x101214, 0.12);
  scene.add(fill);
  return { key, rim, fill };
}

/** A large invisible floor that receives the key light's shadow. */
export function addGround(scene, y, opacity = 0.45) {
  const ground = new THREE.Mesh(
    new THREE.PlaneGeometry(80, 80),
    new THREE.ShadowMaterial({ color: 0x000000, opacity, transparent: true }),
  );
  ground.rotation.x = -Math.PI / 2;
  ground.position.y = y;
  ground.receiveShadow = true;
  scene.add(ground);
  return ground;
}

export function enableShadows(renderer) {
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFShadowMap;
}

// ACES filmic as three.js applies it, run backwards: a colour that will come out of the
// OutputPass as the sRGB colour asked for. Used so the backdrop matches the page exactly.
const ACES_IN = new THREE.Matrix3().set(0.59719, 0.35458, 0.04823, 0.076, 0.90834, 0.01566, 0.0284, 0.13383, 0.83777);
const ACES_OUT = new THREE.Matrix3().set(1.60475, -0.53108, -0.07367, -0.10208, 1.10813, -0.00605, -0.00327, -0.07276, 1.07602);
const IN_INV = ACES_IN.clone().invert();
const OUT_INV = ACES_OUT.clone().invert();
function unfit(y) {
  const A = 1 - 0.983729 * y, B = 0.0245786 - 0.432951 * y, C = -0.000090537 - 0.238081 * y;
  return (-B + Math.sqrt(Math.max(0, B * B - 4 * A * C))) / (2 * A);
}
export function preToneMap(color, exposure = 1) {
  const v = new THREE.Vector3(color.r, color.g, color.b).applyMatrix3(OUT_INV);
  v.set(unfit(v.x), unfit(v.y), unfit(v.z)).applyMatrix3(IN_INV).multiplyScalar(0.6 / exposure);
  return new THREE.Color(v.x, v.y, v.z);
}

/** A plane to put far behind the subject: the page colour with the hero's amber glow. */
export function makeBackdrop(renderer, { bg = 0x0a0a0b, glow = 0xe5a33a, strength = 0.14, size = 256 } = {}) {
  const exposure = renderer.toneMappingExposure;
  const bgS = new THREE.Color(bg).convertLinearToSRGB();
  const glowS = new THREE.Color(glow).convertLinearToSRGB();
  const data = new Float32Array(size * size * 4);
  const target = new THREE.Color();
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = (x + 0.5) / size - 0.5, dy = (y + 0.5) / size - 0.5;
      const r = Math.min(1, Math.hypot(dx / 0.5, dy / 0.34));
      const a = strength * Math.pow(1 - r, 1.7);
      target.setRGB(bgS.r + (glowS.r - bgS.r) * a, bgS.g + (glowS.g - bgS.g) * a, bgS.b + (glowS.b - bgS.b) * a, THREE.SRGBColorSpace);
      const lin = preToneMap(target, exposure);
      const i = (y * size + x) * 4;
      data[i] = lin.r; data[i + 1] = lin.g; data[i + 2] = lin.b; data[i + 3] = 1;
    }
  }
  const tex = new THREE.DataTexture(data, size, size, THREE.RGBAFormat, THREE.FloatType);
  tex.colorSpace = THREE.LinearSRGBColorSpace;
  tex.magFilter = tex.minFilter = THREE.LinearFilter;
  tex.needsUpdate = true;
  return new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.MeshBasicMaterial({ map: tex }));
}

/** Scene render with MSAA, ambient occlusion, SMAA, then tone mapping. Call composer.setSize on resize. */
export function makeComposer(renderer, scene, camera, w, h, { ao = true } = {}) {
  const target = new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType, samples: 4 });
  const composer = new EffectComposer(renderer, target);
  composer.addPass(new RenderPass(scene, camera));
  let gtao = null;
  if (ao) {
    gtao = new GTAOPass(scene, camera, w, h, undefined,
      { radius: 0.3, distanceExponent: 1, thickness: 1, scale: 1.0, samples: 16, distanceFallOff: 1, screenSpaceRadius: false },
      { lumaPhi: 10, depthPhi: 2, normalPhi: 3, radius: 4, radiusExponent: 1, rings: 2, samples: 16 });
    gtao.output = GTAOPass.OUTPUT.Default;
    composer.addPass(gtao);
  }
  composer.addPass(new SMAAPass());
  composer.addPass(new OutputPass());
  composer.setSize(w, h);
  return { composer, gtao };
}

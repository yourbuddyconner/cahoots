// Light rig shared by the landing page and the launch video: a real studio HDRI for reflections,
// one shadow-casting key light, a warm rim, and a ground that only catches shadows.
import * as THREE from "three";
import { RGBELoader } from "three/addons/loaders/RGBELoader.js";

/** Loads the studio HDRI into the scene's environment. Resolves when reflections are ready. */
export async function setupEnvironment(renderer, scene, url, intensity = 1.0) {
  const hdr = await new RGBELoader().loadAsync(url);
  hdr.mapping = THREE.EquirectangularReflectionMapping;
  const pmrem = new THREE.PMREMGenerator(renderer);
  pmrem.compileEquirectangularShader();
  scene.environment = pmrem.fromEquirectangular(hdr).texture;
  scene.environmentIntensity = intensity;
  // Turn the room so the big softbox sits up and to the left of the camera, where the key light is.
  scene.environmentRotation = new THREE.Euler(0, Math.PI * 0.35, 0);
  pmrem.dispose();
  return hdr;
}

export function addLights(scene) {
  const key = new THREE.DirectionalLight(0xfff1e0, 2.6);
  key.position.set(-3.5, 8, 4.5);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.camera.near = 1;
  key.shadow.camera.far = 30;
  key.shadow.camera.left = key.shadow.camera.bottom = -8;
  key.shadow.camera.right = key.shadow.camera.top = 8;
  key.shadow.bias = -0.0004;
  key.shadow.normalBias = 0.02;
  key.shadow.radius = 6;
  scene.add(key);
  const rim = new THREE.DirectionalLight(0xe5a33a, 1.1);
  rim.position.set(6, 3, -6);
  scene.add(rim);
  const fill = new THREE.HemisphereLight(0xcfd3d8, 0x101214, 0.25);
  scene.add(fill);
  return { key, rim, fill };
}

/** A large invisible floor that receives the key light's shadow. */
export function addGround(scene, y, opacity = 0.55) {
  const ground = new THREE.Mesh(
    new THREE.PlaneGeometry(60, 60),
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
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
}

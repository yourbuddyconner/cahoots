// Geometry comes from the V1 KiCad assembly, not hand-built primitives.
import * as THREE from './vendor/three/three.module.js';
import { GLTFLoader } from './vendor/three/GLTFLoader.js';
import { batchAssembly } from './badge-geometry.js';
import { RoomEnvironment } from './vendor/three/RoomEnvironment.js';

export async function initBadge(stage) {
  const canvas = stage.querySelector('canvas');
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const toggle = stage.querySelector('.motion-toggle');
  let paused = motion.matches;
  let renderer;
  try {
    // Load and validate the assembly before hiding the static fallback.
    const gltf = await new GLTFLoader().loadAsync(new URL('./assets/badge-v1.glb', import.meta.url).href);
    const model = gltf.scene;
    const bounds = new THREE.Box3().setFromObject(model);
    if (bounds.isEmpty()) throw new Error('Empty PCB assembly');
    const size = bounds.getSize(new THREE.Vector3());
    const center = bounds.getCenter(new THREE.Vector3());
    model.position.sub(center);

    renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.35;
    renderer.setClearColor(0xffffff, 0);
    const scene = new THREE.Scene();
    const pmrem = new THREE.PMREMGenerator(renderer);
    const room = new RoomEnvironment();
    const environment = pmrem.fromScene(room, 0.04);
    scene.environment = environment.texture;
    scene.environmentIntensity = 1.1;
    room.dispose();
    pmrem.dispose();
    scene.add(new THREE.HemisphereLight(0xffffff, 0x8c91a3, 2));
    const key = new THREE.DirectionalLight(0xffffff, 3);
    key.position.set(3, 5, 6);
    scene.add(key);
    const fill = new THREE.DirectionalLight(0xe4eaff, 2);
    fill.position.set(-4, 1, -3);
    scene.add(fill);

    const group = new THREE.Group();
    // KiCad exports in metres, with the board face normal along +Y.
    group.scale.setScalar(3.7 / Math.max(size.x, size.y, size.z));
    group.add(model);
    scene.add(group);

    // Use source node indices: GLTFLoader renames duplicate CASE1 nodes.
    // The two enclosure models are attached to CASE1 in tray/lid order.
    const layers = [];
    let enclosure = 0;
    const clearMaterials = new Map();
    for (const [index, source] of gltf.parser.json.nodes.entries()) {
      if (!['CASE1', 'DS1'].includes(source.name)) continue;
      const node = await gltf.parser.getDependency('node', index);
      const offset = source.name === 'DS1' ? 0.009 : enclosure++ === 0 ? -0.012 : 0.018;
      layers.push({ node, rest: node.position.y, offset });
      if (source.name === 'CASE1') {
        // Restore the clear enclosure finish from case/*.wrl; STEP loses alpha.
        node.traverse(child => {
          if (!child.isMesh) return;
          const materials = Array.isArray(child.material) ? child.material : [child.material];
          child.material = materials.map(material => {
            if (clearMaterials.has(material)) return clearMaterials.get(material);
            const copy = material.clone();
            copy.color.set('#e0e7f0');
            copy.transparent = true;
            copy.opacity = 0.28;
            copy.depthWrite = false;
            copy.roughness = 0.25;
            clearMaterials.set(material, copy);
            return copy;
          });
          if (child.material.length === 1) child.material = child.material[0];
        });
      }
    }

    batchAssembly(model, layers.map(layer => layer.node));

    const camera = new THREE.PerspectiveCamera(33, 1, 0.1, 100);
    camera.position.set(0, 0.3, 9);
    camera.lookAt(0, 0, 0);
    let elapsed = 0, lastTime = null, visible = true, dirty = true;
    let renderedWidth = 0, renderedHeight = 0;
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; dirty = true; });
    observer.observe(stage);
    function updateToggle() {
      toggle.textContent = paused ? 'Play motion' : 'Pause motion';
      toggle.setAttribute('aria-pressed', String(paused));
      dirty = true;
    }
    toggle.addEventListener('click', () => { paused = !paused; updateToggle(); });
    motion.addEventListener('change', () => { paused = motion.matches; updateToggle(); });
    new ResizeObserver(() => { dirty = true; }).observe(canvas);
    updateToggle();

    function animate(time = 0) {
      const delta = lastTime === null ? 0 : Math.min((time - lastTime) / 1000, 0.05);
      lastTime = time;
      if (!visible || document.hidden) return;
      if (paused && !dirty) return;
      if (!paused) elapsed += delta;
      const width = canvas.clientWidth, height = canvas.clientHeight;
      if (!width || !height) return;
      if (width !== renderedWidth || height !== renderedHeight) {
        renderer.setSize(width, height, false);
        camera.aspect = width / height;
        camera.updateProjectionMatrix();
        renderedWidth = width;
        renderedHeight = height;
      }
      // Start on the display face, then slowly show the components on the back.
      group.rotation.set(Math.PI / 2, 0.35 + elapsed * 0.16, -0.14, 'YXZ');
      const separation = (1 - Math.cos(elapsed * 0.35)) / 2;
      for (const layer of layers) layer.node.position.y = layer.rest + layer.offset * separation;
      renderer.render(scene, camera);
      dirty = false;
    }
    canvas.hidden = false;
    animate();
    stage.querySelector('.badge-fallback').hidden = true;
    toggle.hidden = false;
    renderer.setAnimationLoop(animate);
  } catch (error) {
    renderer?.dispose();
    throw error; // The loader keeps the real V1 board image as the fallback.
  }
}

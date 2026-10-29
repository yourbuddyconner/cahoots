import * as THREE from './vendor/three/three.module.js';
import { mergeGeometries } from './vendor/three/BufferGeometryUtils.js';

// CAD exports contain thousands of small faces. Batch by material and moving
// assembly part, retaining the exact exported vertices and component positions.
export function batchAssembly(model, movingParts) {
  model.updateMatrixWorld(true);
  const targets = new Set([model, ...movingParts]);
  const batches = new Map();
  const meshes = [];
  model.traverse(mesh => { if (mesh.isMesh) meshes.push(mesh); });
  const sourceGeometries = new Set();
  for (const mesh of meshes) {
    if (Array.isArray(mesh.material)) throw new Error('Expected one material per CAD surface');
    let target = mesh.parent;
    while (!targets.has(target)) target = target.parent;
    let materials = batches.get(target);
    if (!materials) batches.set(target, materials = new Map());
    let geometries = materials.get(mesh.material);
    if (!geometries) materials.set(mesh.material, geometries = []);
    const transform = new THREE.Matrix4().copy(target.matrixWorld).invert().multiply(mesh.matrixWorld);
    const geometry = mesh.geometry.index ? mesh.geometry.toNonIndexed() : mesh.geometry.clone();
    for (const name of Object.keys(geometry.attributes)) {
      if (!['position', 'normal'].includes(name)) geometry.deleteAttribute(name);
    }
    if (!geometry.getAttribute('normal')) geometry.computeVertexNormals();
    geometry.applyMatrix4(transform);
    geometries.push(geometry);
    sourceGeometries.add(mesh.geometry);
    mesh.removeFromParent();
  }
  for (const [target, materials] of batches) {
    for (const [material, geometries] of materials) {
      const merged = mergeGeometries(geometries);
      if (!merged) throw new Error('Unable to combine CAD surfaces');
      target.add(new THREE.Mesh(merged, material));
      for (const geometry of geometries) geometry.dispose();
    }
  }
  for (const geometry of sourceGeometries) geometry.dispose();
}

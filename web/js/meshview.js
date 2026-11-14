/* Photographed 3D city: Vexcel's textured mesh (3D Tiles, through our server's /api/mesh proxy), seen from the spot.
 * Where Vexcel has a mesh (LA, Houston for our key) this is the 3D view; elsewhere the app uses realview.js.
 * Real walls, real textures. The capture's own light is baked in, so the time of day is shown by grading the photo
 * (warmer and darker toward sunset, night after) under a physical sky with the sun in place.
 * Same interface as realview.js: create(...) -> {mount, setSun, setYaw, setLook, dispose, yaw, attribution}.
 */
import * as THREE from 'three';
import { Sky } from 'three/examples/jsm/objects/Sky.js';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { DRACOLoader } from 'three/examples/jsm/loaders/DRACOLoader.js';
import { KTX2Loader } from 'three/examples/jsm/loaders/KTX2Loader.js';
import { TilesRenderer, WGS84_ELLIPSOID } from '3d-tiles-renderer';

const LIBS = 'https://cdn.jsdelivr.net/npm/three@0.160.0/examples/jsm/libs/';
const rad = Math.PI / 180;
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const lerp = (a, b, t) => a + (b - a) * t;

/* How the photo is graded for the sun's elevation: [r, g, b] multipliers. */
function grade(el) {
  if (el > 12) return [1, 1, 1];
  if (el > 0) { const t = el / 12; return [lerp(1.05, 1, t), lerp(.82, 1, t), lerp(.62, 1, t)]; }      // golden
  if (el > -6) { const t = (el + 6) / 6; return [lerp(.32, 1.0, t), lerp(.3, .78, t), lerp(.42, .6, t)]; } // dusk
  return [.18, .19, .3];                                                                                  // night
}

export async function create({ host, base = '', lat, lon, yaw = 270, tilesetUrl, attribution = '', onStatus = () => {} }) {
  onStatus('Loading the Vexcel 3D city…');
  const renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance', logarithmicDepthBuffer: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio || 1, 2));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = .5;
  const scene = new THREE.Scene();
  const cam = new THREE.PerspectiveCamera(60, 1, 0.5, 80000);

  // the mesh, in the spot's local frame: x east, y up, z south, origin on the ellipsoid under the spot
  const tiles = new TilesRenderer(base + tilesetUrl);
  tiles.errorTarget = 6; // finer than the default: we stand in the scene, not above it
  tiles.downloadQueue.maxJobs = 12;
  tiles.parseQueue.maxJobs = 4;
  // standing in the city needs many fine tiles at once (Vexcel goes to zoom 20, ~0.75 m geometric error)
  tiles.lruCache.minSize = 1500; tiles.lruCache.maxSize = 3000;
  const draco = new DRACOLoader().setDecoderPath(LIBS + 'draco/gltf/');
  const ktx2 = new KTX2Loader().setTranscoderPath(LIBS + 'basis/').detectSupport(renderer);
  const gltf = new GLTFLoader(tiles.manager).setDRACOLoader(draco).setKTX2Loader(ktx2);
  tiles.manager.addHandler(/\.(gltf|glb)$/i, gltf);
  const enu = WGS84_ELLIPSOID.getEastNorthUpFrame(lat * rad, lon * rad, new THREE.Matrix4());
  tiles.group.matrixAutoUpdate = false;
  tiles.group.matrix.copy(new THREE.Matrix4().makeRotationX(-Math.PI / 2).multiply(enu.invert()));
  tiles.group.matrixWorldNeedsUpdate = true;
  scene.add(tiles.group);

  // photo look: unlit (the capture already has its light), graded for the time of day
  const tint = new THREE.Color(1, 1, 1);
  const mats = new Set();
  tiles.addEventListener('load-model', ({ scene: s }) => {
    s.traverse(o => {
      if (!o.isMesh) return;
      const old = o.material;
      const m = new THREE.MeshBasicMaterial({ map: old.map, color: tint.clone() });
      old.dispose(); o.material = m; mats.add(m);
    });
  });
  tiles.addEventListener('dispose-model', ({ scene: s }) => s.traverse(o => { if (o.isMesh) mats.delete(o.material); }));

  const sky = new Sky(); sky.scale.setScalar(60000); scene.add(sky);
  const su = sky.material.uniforms;
  su.turbidity.value = 4; su.rayleigh.value = 2.2; su.mieCoefficient.value = .005; su.mieDirectionalG.value = .82;
  scene.fog = new THREE.FogExp2('#c4cfda', .00012);

  tiles.setCamera(cam);
  tiles.setResolutionFromRenderer(cam, renderer);

  // stand on whatever is under the spot (roof or street): ray down through the mesh, eye 1.7 m above it
  let eyeFound = false, groundY = null;
  cam.position.set(0, 250, 0);
  const ray = new THREE.Raycaster(new THREE.Vector3(0, 3000, 0), new THREE.Vector3(0, -1, 0));
  const findGround = () => {
    const hit = ray.intersectObject(tiles.group, true)[0];
    if (hit) { groundY = hit.point.y; cam.position.y = groundY + 1.7; eyeFound = true; }
  };

  let look = { yaw, pitch: 3, fov: 60 };
  const aim = () => {
    const a = look.yaw * rad, p = look.pitch * rad;
    if (cam.fov !== look.fov) { cam.fov = look.fov; cam.updateProjectionMatrix(); }
    cam.lookAt(cam.position.x + Math.sin(a) * Math.cos(p), cam.position.y + Math.sin(p), cam.position.z - Math.cos(a) * Math.cos(p));
  };

  // drag to look, pinch / wheel to zoom
  const el = renderer.domElement;
  el.style.touchAction = 'none'; el.style.cursor = 'grab'; el.style.display = 'block';
  const pts = new Map(); let pinch = 0;
  el.onpointerdown = e => { pts.set(e.pointerId, [e.clientX, e.clientY]); el.setPointerCapture(e.pointerId); el.style.cursor = 'grabbing'; };
  el.onpointerup = el.onpointercancel = e => { pts.delete(e.pointerId); pinch = 0; el.style.cursor = 'grab'; };
  el.onpointermove = e => {
    const prev = pts.get(e.pointerId); if (!prev) return;
    pts.set(e.pointerId, [e.clientX, e.clientY]);
    if (pts.size === 2) {
      const [a, b] = [...pts.values()], d = Math.hypot(a[0] - b[0], a[1] - b[1]);
      if (pinch) look.fov = clamp(look.fov * pinch / d, 15, 80);
      pinch = d;
    } else {
      const k = look.fov / 60 * .22;
      look.yaw -= (e.clientX - prev[0]) * k; look.pitch = clamp(look.pitch + (e.clientY - prev[1]) * k, -45, 60);
    }
  };
  el.onwheel = e => { e.preventDefault(); look.fov = clamp(look.fov * (e.deltaY > 0 ? 1.08 : 1 / 1.08), 15, 80); };

  // render loop: 3D Tiles streams in finer tiles as you look around
  let alive = true, frames = 0;
  const loop = () => {
    if (!alive) return;
    requestAnimationFrame(loop);
    if (!hostEl || !hostEl.isConnected) return;
    if (!eyeFound || frames++ % 30 === 0) findGround(); // refine as sharper tiles arrive
    aim();
    cam.updateMatrixWorld();
    tiles.update();
    renderer.render(scene, cam);
  };

  let hostEl = null;
  const fit = () => {
    if (!hostEl) return;
    const w = hostEl.clientWidth || 600, h = hostEl.clientHeight || 320;
    renderer.setSize(w, h, false); el.style.width = '100%'; el.style.height = '100%';
    cam.aspect = w / Math.max(h, 1); cam.updateProjectionMatrix();
    tiles.setResolutionFromRenderer(cam, renderer);
  };
  const ro = new ResizeObserver(fit);

  const view = {
    kind: 'mesh',
    attribution: attribution || '3D mesh © Vexcel Imaging US, Inc.',
    canvas: el,
    get yaw() { return look.yaw; },
    get groundY() { return groundY; },
    tiles, // for tests
    _cam: cam,
    mount(h) {
      if (hostEl) ro.unobserve(hostEl);
      hostEl = h; h.innerHTML = ''; h.style.position = 'relative'; h.appendChild(el);
      const note = document.createElement('div');
      note.textContent = view.attribution;
      note.style.cssText = 'position:absolute;left:8px;bottom:6px;font:11px sans-serif;color:rgba(255,255,255,.85);text-shadow:0 1px 3px #000;pointer-events:none';
      h.appendChild(note); ro.observe(h); fit();
    },
    setYaw(y) { look.yaw = y; },
    setLook(o) { Object.assign(look, o); },
    setSun(az, elev) {
      const dir = new THREE.Vector3(Math.sin(az * rad) * Math.cos(elev * rad), Math.sin(elev * rad), -Math.cos(az * rad) * Math.cos(elev * rad));
      const low = clamp(1 - elev / 12, 0, 1);
      su.sunPosition.value.copy(dir);
      su.turbidity.value = lerp(4, 7, low); su.rayleigh.value = lerp(2.2, 3, low);
      renderer.toneMappingExposure = elev > 0 ? lerp(.5, .42, clamp(elev / 20, 0, 1)) : lerp(.55, .9, clamp(-elev / 6, 0, 1));
      const g = grade(elev); tint.setRGB(g[0], g[1], g[2]);
      for (const m of mats) m.color.copy(tint);
      scene.fog.color.setRGB(lerp(.77, .25, clamp(-elev / 6, 0, 1)) * g[0], lerp(.81, .25, clamp(-elev / 6, 0, 1)) * g[1], lerp(.85, .35, clamp(-elev / 6, 0, 1)) * g[2]);
    },
    dispose() {
      alive = false; ro.disconnect();
      tiles.dispose(); draco.dispose(); ktx2.dispose();
      sky.geometry.dispose(); sky.material.dispose();
      renderer.dispose(); el.remove();
    },
  };
  view.mount(host);
  loop();
  onStatus('');
  return view;
}

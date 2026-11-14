import { interpSun, sunVector } from './geo.js';

let T = null;

export function disposeScene() {
  if (!T) return;
  T.r?.dispose();
  T = null;
}

export function buildScene({ host, sceneGrid, sunPath, horizon, spot, timeMs, yaw }) {
  if (!host || !window.THREE) {
    host.innerHTML = '<p class="msg">3D preview needs WebGL.</p>';
    return;
  }
  const THREE = window.THREE;
  try {
    if (!T) {
      T = {
        r: new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' }),
        cam: new THREE.PerspectiveCamera(62, 1, 0.5, 4000),
      };
      T.r.setPixelRatio(Math.min(devicePixelRatio || 1, 1.5));
      T.r.shadowMap.enabled = true;
    }
  } catch {
    host.innerHTML = '<p class="msg">WebGL is not available in this browser.</p>';
    return;
  }
  const w = host.clientWidth || 640, h = host.clientHeight || 360;
  T.r.setSize(w, h, false);
  T.cam.aspect = w / Math.max(h, 1);
  T.cam.updateProjectionMatrix();
  host.innerHTML = '';
  host.appendChild(T.r.domElement);

  const s = new THREE.Scene();
  s.fog = new THREE.FogExp2('#4a2c28', 0.0018);
  const world = new THREE.Group();
  s.add(world);

  const sc = sceneGrid;
  if (sc?.heights && sc.n) {
    const n = sc.n;
    const L = (n - 1) * (sc.cell_m || 1);
    const g = new THREE.PlaneGeometry(L, L, n - 1, n - 1);
    g.rotateX(-Math.PI / 2);
    const P = g.attributes.position;
    const col = [];
    let base = Infinity;
    for (let i = 0; i < sc.heights.length; i++) {
      const y = sc.heights[i];
      if (Number.isFinite(y)) base = Math.min(base, y);
    }
    if (!Number.isFinite(base)) base = 0;
    for (let i = 0; i < P.count; i++) {
      const y = Number.isFinite(sc.heights[i]) ? sc.heights[i] - base : 0;
      P.setY(i, y);
      const k = Math.min(1, y / 80);
      col.push(0.22 + 0.35 * k, 0.18 + 0.22 * k, 0.16 + 0.18 * k);
    }
    g.setAttribute('color', new THREE.Float32BufferAttribute(col, 3));
    g.computeVertexNormals();
    const mesh = new THREE.Mesh(g, new THREE.MeshLambertMaterial({ vertexColors: true }));
    mesh.receiveShadow = true;
    mesh.castShadow = true;
    world.add(mesh);
    T.base = base;
    T.eyeY = (Number.isFinite(sc.heights[Math.floor(n * n / 2)]) ? sc.heights[Math.floor(n * n / 2)] : (spot.height_m || 20)) - base + 2;
  } else {
    T.eyeY = (spot.height_m || 12) + 2;
    T.base = 0;
  }

  if (horizon?.length) {
    const pts = horizon.map((p) => {
      const v = sunVector(p.az_deg, p.elev_deg, 420);
      return new THREE.Vector3(v.x, v.y + T.eyeY, v.z);
    });
    pts.push(pts[0].clone());
    world.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), new THREE.LineBasicMaterial({ color: 0xffd9a0, transparent: true, opacity: 0.55 })));
  }

  const pathPts = (sunPath || []).filter((p) => p.elev_deg > -2).map((p) => {
    const v = sunVector(p.az_deg, p.elev_deg, 900);
    return new THREE.Vector3(v.x, v.y, v.z);
  });
  if (pathPts.length) {
    world.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pathPts), new THREE.LineBasicMaterial({ color: 0xfff4d2 })));
  }

  T.sun = new THREE.Mesh(new THREE.SphereGeometry(28, 16, 16), new THREE.MeshBasicMaterial({ color: 0xfff2c4 }));
  s.add(T.sun);
  T.dl = new THREE.DirectionalLight(0xffd9a0, 1.2);
  T.dl.castShadow = true;
  s.add(T.dl, new THREE.AmbientLight(0x6a5a70, 0.55));
  T.s = s;
  T.yaw = yaw ?? 270;
  T.path = sunPath;
  T.cam.position.set(0, T.eyeY, 0);
  T.host = host;

  const el = T.r.domElement;
  el.style.touchAction = 'none';
  el.style.cursor = 'grab';
  let drag = null;
  el.onpointerdown = (e) => { drag = e.clientX; el.setPointerCapture(e.pointerId); el.style.cursor = 'grabbing'; };
  el.onpointerup = () => { drag = null; el.style.cursor = 'grab'; };
  el.onpointermove = (e) => {
    if (drag == null) return;
    T.yaw -= (e.clientX - drag) * 0.25;
    drag = e.clientX;
    renderAt(timeMs);
  };
  renderAt(timeMs);
}

export function renderAt(timeMs) {
  if (!T?.s || !T.path) return;
  const THREE = window.THREE;
  const p = interpSun(T.path, timeMs);
  const v = sunVector(p.az, Math.max(p.el, -5), 900);
  T.sun.position.set(v.x, v.y, v.z);
  T.sun.visible = p.el > -3;
  T.dl.position.set(v.x, v.y, v.z);
  T.dl.intensity = Math.max(0.12, Math.min(1.4, p.el / 12 + 0.35));
  const t = Math.max(0, Math.min(1, (p.el + 4) / 22));
  const sky = new THREE.Color().setHSL(0.07 + t * 0.45, 0.45, 0.16 + t * 0.28);
  T.s.background = sky;
  if (T.s.fog) T.s.fog.color.copy(sky);
  const look = sunVector(T.yaw, 4, 1);
  T.cam.lookAt(T.cam.position.clone().add(new THREE.Vector3(look.x, look.y, look.z)));
  T.r.render(T.s, T.cam);
}

export function currentYaw() {
  return T?.yaw ?? 270;
}

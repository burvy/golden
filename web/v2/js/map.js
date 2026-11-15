// The map: Vexcel tiles from golden's server (cached downtown LA), the candidates as a soft heat,
// points a must dropped greyed out, the objects the goals talk about, and the top spots as numbered pins.
import { CENTER, GRID, STEP_M, OBJECTS, SUN_AZ, toLatLon } from './mock.js';

const Z = 16, TILE = 256, M_PX_Z = 156543.03392 * Math.cos(CENTER.lat * Math.PI / 180) / 2 ** Z; // metres per tile px
const worldPx = (lat, lon) => {
  const s = Math.sin(lat * Math.PI / 180), n = TILE * 2 ** Z;
  return { x: (lon + 180) / 360 * n, y: (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * n };
};
const tiles = {};

export function createMap(canvas, { base, onPick }) {
  const ctx = canvas.getContext('2d');
  const view = { cx: 60, cy: -20, spanM: GRID * STEP_M * 1.04 };
  let data = null, tilesOk = !!base;
  const off = document.createElement('canvas');

  function px(x, y) { // metres from CENTER -> css px
    const W = canvas.clientWidth, H = canvas.clientHeight, mpp = view.spanM / Math.min(W, H * 1.35);
    return { x: W / 2 + (x - view.cx) / mpp, y: H / 2 - (y - view.cy) / mpp, mpp };
  }
  function tile(z, x, y) {
    const k = `${z}/${x}/${y}`;
    if (!tiles[k]) { const im = new Image(); im.onload = () => draw(); im.onerror = () => { im.bad = true; }; im.src = `${base}/api/tiles/vexcel/${k}`; tiles[k] = im; }
    return tiles[k];
  }
  function drawTiles(W, H) {
    const { mpp } = px(0, 0), scale = M_PX_Z / mpp; // css px per tile px
    const c = toLatLon(view.cx, view.cy), w = worldPx(c.lat, c.lon);
    const x0 = w.x - W / 2 / scale, y0 = w.y - H / 2 / scale;
    let any = false;
    for (let tx = Math.floor(x0 / TILE); tx <= Math.floor((x0 + W / scale) / TILE); tx++)
      for (let ty = Math.floor(y0 / TILE); ty <= Math.floor((y0 + H / scale) / TILE); ty++) {
        const im = tile(Z, tx, ty);
        if (im.complete && im.naturalWidth) { any = true; ctx.drawImage(im, (tx * TILE - x0) * scale, (ty * TILE - y0) * scale, TILE * scale + 0.5, TILE * scale + 0.5); }
      }
    return any;
  }
  function drawBlocks(cands) { // no tiles: a quiet drawn city from the roof mask, so the mock still reads as a place
    ctx.fillStyle = '#e9f1f8'; ctx.fillRect(0, 0, canvas.clientWidth, canvas.clientHeight);
    const s = px(STEP_M, 0).x - px(0, 0).x;
    ctx.fillStyle = '#c9dbea';
    for (const c of cands) if (c.roof) { const p = px(c.x, c.y); ctx.fillRect(p.x - s / 2, p.y - s / 2, s + 0.5, s + 0.5); }
  }

  function draw() {
    const dpr = devicePixelRatio || 1, W = canvas.clientWidth, H = canvas.clientHeight;
    if (canvas.width !== Math.round(W * dpr)) { canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr); }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const cands = data?.candidates || [];
    if (!(tilesOk && drawTiles(W, H))) drawBlocks(cands);
    if (!data) return;
    const { run, layer, color, sel, picks } = data;
    const s = px(STEP_M, 0).x - px(0, 0).x;

    // dropped by a must: a cool veil, so you can see what each gate threw away
    ctx.fillStyle = 'rgba(12,42,67,.34)';
    cands.forEach((c, i) => { if (run.droppedBy[i] && (layer === 'all' || run.droppedBy[i] === layer || layer === 'dropped')) { const p = px(c.x, c.y); ctx.fillRect(p.x - s / 2, p.y - s / 2, s + 0.5, s + 0.5); } });

    // heat, blurred on an offscreen canvas: the combined score (sunset ramp) or one goal (its colour)
    off.width = canvas.width; off.height = canvas.height;
    const o = off.getContext('2d'); o.setTransform(dpr, 0, 0, dpr, 0, 0);
    const vals = layer === 'all' ? run.total : run.layers[layer];
    const alive = layer === 'all' ? new Set(run.alive) : null;
    let max = 0; cands.forEach((_, i) => { if (!alive || alive.has(i)) max = Math.max(max, vals[i]); });
    cands.forEach((c, i) => {
      if (alive && !alive.has(i)) return;
      const v = vals[i] / (max || 1); if (v < 0.08) return;
      const p = px(c.x, c.y);
      o.fillStyle = layer === 'all' ? ramp(v) : hexA(color(layer), 0.15 + 0.75 * v);
      o.fillRect(p.x - s / 2, p.y - s / 2, s + 0.5, s + 0.5);
    });
    ctx.save(); ctx.filter = `blur(${Math.max(2, s * 0.35)}px)`; ctx.globalAlpha = 0.85; ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(off, 0, 0); ctx.restore();

    // the selected spot: where the sun goes down, and the line to the tower
    if (sel != null) {
      const c = cands[sel], p = px(c.x, c.y), a = SUN_AZ * Math.PI / 180, L = Math.max(W, H);
      ctx.save(); ctx.setLineDash([6, 6]); ctx.lineWidth = 2; ctx.strokeStyle = '#f9a03f';
      ctx.beginPath(); ctx.moveTo(p.x, p.y); ctx.lineTo(p.x + Math.sin(a) * L, p.y - Math.cos(a) * L); ctx.stroke(); ctx.restore();
      const t = px(OBJECTS[0].x, OBJECTS[0].y);
      ctx.save(); ctx.lineWidth = 1.5; ctx.strokeStyle = 'rgba(255,255,255,.85)'; ctx.beginPath(); ctx.moveTo(p.x, p.y); ctx.lineTo(t.x, t.y); ctx.stroke(); ctx.restore();
    }
    // the objects
    for (const ob of OBJECTS) {
      const p = px(ob.x, ob.y);
      ctx.save(); ctx.fillStyle = '#fff'; ctx.strokeStyle = '#0c2a43'; ctx.lineWidth = 2;
      ctx.beginPath(); ob.kind === 'fountain' ? ctx.arc(p.x, p.y, 6, 0, 7) : ctx.rect(p.x - 6, p.y - 6, 12, 12); ctx.fill(); ctx.stroke();
      ctx.font = '600 12.5px "Instrument Sans",sans-serif'; ctx.lineWidth = 3.5; ctx.strokeStyle = 'rgba(12,42,67,.85)'; ctx.fillStyle = '#fff';
      ctx.strokeText(ob.name, p.x + 10, p.y + 4); ctx.fillText(ob.name, p.x + 10, p.y + 4); ctx.restore();
    }
    // the top spots
    picks.forEach((i, n) => {
      const p = px(cands[i].x, cands[i].y), on = i === sel, r = on ? 15 : 12;
      ctx.save(); ctx.shadowColor = 'rgba(12,42,67,.45)'; ctx.shadowBlur = 6;
      ctx.fillStyle = on ? '#f26b1d' : '#fff'; ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, 7); ctx.fill(); ctx.shadowBlur = 0;
      ctx.lineWidth = 2.5; ctx.strokeStyle = on ? '#fff' : '#f26b1d'; ctx.stroke();
      ctx.fillStyle = '#0c2a43'; ctx.font = `800 ${on ? 15 : 13}px "Bricolage Grotesque",sans-serif`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
      ctx.fillText(n + 1, p.x, p.y + 1); ctx.restore();
    });
  }

  canvas.addEventListener('click', e => {
    if (!data) return;
    const r = canvas.getBoundingClientRect(), mx = e.clientX - r.left, my = e.clientY - r.top;
    let best = null, bd = 26;
    data.picks.forEach((i, n) => { const p = px(data.candidates[i].x, data.candidates[i].y), d = Math.hypot(p.x - mx, p.y - my); if (d < bd) { bd = d; best = n; } });
    if (best != null) onPick(best);
  });
  addEventListener('resize', () => draw());
  return {
    set(d) { data = d; draw(); },
    attribution: () => (tilesOk ? 'Imagery © Vexcel Imaging' : 'Drawn preview, no imagery (server not reachable)'),
    noTiles() { tilesOk = false; draw(); },
    draw,
  };
}

// combined score: pale sky -> golden -> sunset orange -> deep sunset
function ramp(v) {
  const stops = [[0, [61, 155, 224, 0.0]], [0.3, [249, 200, 120, 0.55]], [0.65, [249, 160, 63, 0.8]], [1, [214, 72, 12, 0.95]]];
  let i = 1; while (i < stops.length - 1 && v > stops[i][0]) i++;
  const [a0, c0] = stops[i - 1], [a1, c1] = stops[i], t = (v - a0) / (a1 - a0);
  const c = c0.map((x, k) => x + (c1[k] - x) * t);
  return `rgba(${c[0] | 0},${c[1] | 0},${c[2] | 0},${c[3].toFixed(2)})`;
}
export const RAMP_CSS = 'linear-gradient(90deg,rgba(61,155,224,.15),rgba(249,200,120,.8),#f9a03f,#d6480c)';
function hexA(hex, a) { const n = parseInt(hex.slice(1), 16); return `rgba(${n >> 16},${(n >> 8) & 255},${n & 255},${a})`; }

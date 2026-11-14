import { blockTime, horizonFn, offsetISO, rng, sunAt, tzOffsetHours } from './geo.js';
import { normalizeSearch } from './adapters.js';

function sunPath(date, lat, lon, tz) {
  const off = tzOffsetHours(tz);
  const o = [];
  for (let m = 0; m < 1440; m += 5) {
    const p = sunAt(date, m, lat, lon, off);
    if (p.el > -6) o.push({ t: offsetISO(date, m, off), az_deg: +p.az.toFixed(2), elev_deg: +p.el.toFixed(2) });
  }
  return o;
}

function makeSpots(lat, lon) {
  const r = rng(Math.round(Math.abs(lat * 1000) + Math.abs(lon * 1000)));
  const names = [
    'DEMO rooftop A',
    'DEMO garage deck B',
    'DEMO street overlook C',
    'DEMO terrace D',
    'DEMO bridge walk E',
    'DEMO parking roof F',
  ];
  return names.map((id, i) => {
    const dlat = (r() - 0.45) * 0.012;
    const dlon = (r() - 0.45) * 0.012;
    const h = 8 + Math.round(r() * 48);
    const towers = [];
    for (let k = 0; k < 10; k++) towers.push({ c: r() * 360, w: 0.8 + r() * 2.2, e: 2 + r() * (8 + h / 10) });
    const gapAz = 250 + r() * 50;
    towers.push({ c: gapAz - 3, w: 2, e: 14 + h / 12, g: 1 }, { c: gapAz + 3, w: 2, e: 12 + h / 14, g: 1 });
    return {
      id,
      lat: +(lat + dlat).toFixed(5),
      lon: +(lon + dlon).toFixed(5),
      h,
      towers,
      gap: [gapAz, 3.2],
    };
  });
}

function prof(s, az) {
  let e = 0.6;
  for (const t of s.towers) if (Math.abs(az - t.c) <= t.w && t.e > e) e = t.e;
  return e;
}

function horizon(s) {
  return Array.from({ length: 360 }, (_, a) => ({ az_deg: a, elev_deg: +prof(s, a).toFixed(2) }));
}

function nearest(spots, lat, lon) {
  return spots.reduce((m, s) => (Math.hypot(s.lat - lat, s.lon - lon) < Math.hypot(m.lat - lat, m.lon - lon) ? s : m));
}

export function createMock({ lat, lon, tz, date, text }) {
  const spotsRaw = makeSpots(lat, lon);
  const rise = /sunrise|dawn|rising/i.test(text || '');
  const q = (text || '').toLowerCase();

  function search() {
    const res = spotsRaw.map((s) => {
      const p = sunPath(date, s.lat, s.lon, tz);
      const c = blockTime(p, horizonFn(horizon(s)), rise);
      let sc = 0.35;
      const why = ['DEMO geometry only'];
      if (/roof/.test(q)) { sc += 0.12; why.push('labelled rooftop in demo set'); }
      const gap = s.gap && c && Math.abs(c.az - s.gap[0]) < s.gap[1];
      if (gap) { sc += /between|gap/.test(q) ? 0.28 : 0.1; why.push('sun sets in a synthetic tower gap'); }
      sc += Math.min(0.08, s.h / 640);
      const until = c ? c.ms : Date.parse(p[p.length - 1]?.t || date);
      return {
        id: s.id,
        lat: s.lat,
        lon: s.lon,
        ground_m: 80 + s.h,
        eye_m: 81.7 + s.h,
        standing: /roof/i.test(s.id) ? 'roof' : 'ground',
        score: +Math.max(0.05, Math.min(0.99, sc)).toFixed(2),
        sun: { date, flat_sunset: p[p.length - 1]?.t || null, hidden_at: new Date(until).toISOString(), hidden_az_deg: c?.az ?? null, hidden_elev_deg: c?.el ?? null, golden_min_visible: 20 },
        why,
      };
    }).sort((a, b) => b.score - a.score).slice(0, 6);
    return normalizeSearch({
      brief: { text, area: { lat, lon, radius_m: 400 }, date, tz, event: rise ? 'sunrise' : 'sunset', standing: 'any', framing: 'clear', max_spots: 10 },
      spots: res,
      heat: { center_lat: lat, center_lon: lon, cell_m: 20, width: 1, height: 1, values: [0.5] },
      searched: 6, took_ms: 0,
    }, { demo: true });
  }

  function agent() {
    const result = search();
    return {
      title: 'DEMO location scouting call sheet',
      brief: { text, area: { lat, lon, radius_m: 400 }, date, tz, event: rise ? 'sunrise' : 'sunset', standing: 'any', framing: 'clear', max_spots: 10 },
      summary: 'Synthetic location recommendations; not verified filming sites.',
      timeline: [],
      spots: result.spots.slice(0, 3).map((spot, i) => ({ role: i ? 'backup' : 'primary', spot: { id: spot.id, lat: spot.lat, lon: spot.lon, rank: i + 1, score: spot.score, standing: spot.standing, ground_m: spot.height_m, eye_m: spot.height_m + 1.7, sun: { hidden_at: spot.sun_visible_until }, why: [spot.why], building_id: null }, arrive_by: spot.sun_visible_until, notes: ['DEMO only'], photo_url: null })),
      access: [], caveats: ['DEMO geometry, no verified roof access'], model: 'mock',
    };
  }

  function sun(p) {
    return { lat: +p.lat, lon: +p.lon, date: p.date, tz, sunrise: null, sunset: null, golden_hour: { morning: null, evening: null }, points: sunPath(p.date, +p.lat, +p.lon, tz) };
  }

  function hz(p) {
    const sp = nearest(spotsRaw, +p.lat, +p.lon);
    return { observer: { lat: +p.lat, lon: +p.lon, ground_m: 80 + sp.h, eye_m: 81.7 + sp.h }, radius_m: 1500, step_deg: 1, points: horizon(sp), sun: { date, flat_sunset: null, hidden_at: null, hidden_az_deg: null, hidden_elev_deg: null, golden_min_visible: 0 } };
  }

  function scene(p) {
    const s = nearest(spotsRaw, +p.lat, +p.lon);
    const cell = 4;
    const n = Math.min(81, Math.floor((2 * (+p.radius_m || 200)) / cell) + 1);
    const c = (n - 1) / 2;
    const hs = new Array(n * n).fill(2);
    for (const t of s.towers) {
      const d = 40 + (t.c * 11) % 90;
      const bx = Math.sin((t.c * Math.PI) / 180) * d / cell + c;
      const bz = -Math.cos((t.c * Math.PI) / 180) * d / cell + c;
      const hw = (d * Math.tan((t.w * Math.PI) / 180) + 4) / cell;
      const H = s.h + Math.tan((t.e * Math.PI) / 180) * d;
      for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) {
        if (Math.abs(j - bx) < hw && Math.abs(i - bz) < hw) hs[i * n + j] = Math.max(hs[i * n + j], +H.toFixed(1));
      }
    }
    hs[Math.round(c) * n + Math.round(c)] = s.h;
    return { n, cell_m: cell, heights: hs, attribution: 'DEMO height grid — not survey data' };
  }

  function buildings() {
    return { buildings: [
      { id: 'demo:1', name: 'DEMO building (not a real listing)', height_m: 42, floors: 12, contacts: [], matterport: [] },
      { id: 'demo:2', name: 'DEMO neighbouring tower', height_m: 88, floors: 22, contacts: [], matterport: [] },
    ] };
  }

  function photoDataUrl(az) {
    const c = document.createElement('canvas');
    c.width = 720; c.height = 420;
    const x = c.getContext('2d');
    const g = x.createLinearGradient(0, 0, 0, 420);
    g.addColorStop(0, '#2a1830');
    g.addColorStop(0.45, '#c45a2a');
    g.addColorStop(1, '#f0c27a');
    x.fillStyle = g; x.fillRect(0, 0, 720, 420);
    x.fillStyle = '#16141c';
    const r = rng(Math.round(az * 9));
    for (let i = 0; i < 18; i++) {
      const w = 18 + r() * 46, h = 50 + r() * 220;
      x.fillRect(i * 42, 420 - h, w, h);
    }
    x.fillStyle = '#ffe7c2';
    x.font = '600 16px sans-serif';
    x.fillText(`DEMO aerial — looking ${Math.round(az)}°  ·  not real imagery`, 18, 28);
    return c.toDataURL();
  }

  return { search, agent, sun, hz, scene, buildings, photoDataUrl };
}

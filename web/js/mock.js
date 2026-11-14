import { blockTime, horizonFn, offsetISO, rng, sunAt, tzOffsetHours } from './geo.js';
import { classifyScene, decodeSceneRing, normalizeSearch, pickSceneRing } from './adapters.js';

/**
 * Built-in DEMO server. Every method returns the same raw JSON shape the real
 * server sends (docs/contracts.md); adapters.js normalizes it for the UI, so the
 * demo exercises exactly the code path the live data will take.
 */

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
  return names.map((id) => {
    const dlat = (r() - 0.45) * 0.012;
    const dlon = (r() - 0.45) * 0.012;
    const h = 8 + Math.round(r() * 48);
    const towers = [];
    for (let k = 0; k < 10; k++) towers.push({ c: r() * 360, w: 0.8 + r() * 2.2, e: 2 + r() * (8 + h / 10) });
    const gapAz = 250 + r() * 50;
    towers.push({ c: gapAz - 3, w: 2, e: 14 + h / 12, g: 1 }, { c: gapAz + 3, w: 2, e: 12 + h / 14, g: 1 });
    return { id, lat: +(lat + dlat).toFixed(5), lon: +(lon + dlon).toFixed(5), h, towers, gap: [gapAz, 3.2] };
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

/**
 * One instance per search area. api.js keeps the instance from the latest search
 * so the skyline, sun path, height grid and contacts of a clicked spot belong to
 * the same synthetic city that produced the ranked list.
 */
export function createMock({ lat, lon, tz, date, text }) {
  const spotsRaw = makeSpots(lat, lon);
  const q = (text || '').toLowerCase();
  const rise = /sunrise|dawn|rising/.test(q);
  const brief = { text, area: { lat, lon, radius_m: 400 }, date, tz, event: rise ? 'sunrise' : 'sunset', standing: 'any', framing: 'clear', max_spots: 10 };

  // Demo story: the two lead spots have a tower the sun disappears behind, so the card
  // can say "sun drops behind that tower at HH:MM" even when the server is down.
  spotsRaw.slice(0, 2).forEach((s, i) => {
    const p = sunPath(date, s.lat, s.lon, tz);
    const idx = rise ? p.findIndex((x) => x.elev_deg >= 8) : p.length - 1 - [...p].reverse().findIndex((x) => x.elev_deg >= 8);
    if (idx < 0 || idx >= p.length || !p.some((x) => x.elev_deg >= 8)) return;
    const c = p[idx].az_deg + i * 1.5;
    s.towers = s.towers.filter((t) => Math.abs(t.c - c) > 8);
    s.towers.push({ c, w: 2.6, e: 13 + i, hero: 1 });
    s.hero = true;
  });

  function crossing(s, d = date) {
    const p = sunPath(d, s.lat, s.lon, tz);
    return { p, c: blockTime(p, horizonFn(horizon(s)), rise) };
  }

  function searchRaw() {
    const res = spotsRaw.map((s) => {
      const { p, c } = crossing(s);
      let sc = 0.35;
      const why = ['DEMO geometry only'];
      if (/roof/.test(q)) { sc += 0.12; why.push('labelled rooftop in demo set'); }
      const gap = s.gap && c && Math.abs(c.az - s.gap[0]) < s.gap[1];
      if (gap) { sc += /between|gap/.test(q) ? 0.28 : 0.1; why.push('sun sets in a synthetic tower gap'); }
      if (s.hero && /tower|building|behind|between|gap/.test(q)) { sc += 0.25; why.push('sun drops behind a synthetic tower'); }
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
        building_id: null,
      };
    }).sort((a, b) => b.score - a.score).slice(0, 6).map((s, i) => ({ ...s, rank: i + 1 }));
    return { brief, spots: res, heat: demoHeat(lat, lon, res), searched: spotsRaw.length, took_ms: 0 };
  }

  const search = () => normalizeSearch(searchRaw(), { demo: true });

  function agent() {
    const result = search();
    return {
      title: 'DEMO location scouting call sheet',
      brief,
      summary: 'Synthetic location recommendations; not verified filming sites.',
      timeline: [],
      spots: result.spots.slice(0, 3).map((spot, i) => ({
        role: i ? 'backup' : 'primary',
        spot: { id: spot.id, lat: spot.lat, lon: spot.lon, rank: i + 1, score: spot.score, standing: spot.standing, ground_m: spot.height_m, eye_m: spot.height_m + 1.7, sun: { hidden_at: spot.sun_visible_until }, why: [spot.why], building_id: null },
        arrive_by: spot.sun_visible_until,
        notes: ['DEMO only'],
        photo_url: null,
      })),
      access: [],
      caveats: ['DEMO geometry, no verified roof access'],
      model: 'mock',
    };
  }

  function sun(p) {
    const d = p.date || date;
    return { lat: +p.lat, lon: +p.lon, date: d, tz, sunrise: null, sunset: null, golden_hour: { morning: null, evening: null }, points: sunPath(d, +p.lat, +p.lon, tz) };
  }

  function hz(p) {
    const s = nearest(spotsRaw, +p.lat, +p.lon);
    const { c } = crossing(s, p.date || date);
    return {
      observer: { lat: s.lat, lon: s.lon, ground_m: 80 + s.h, eye_m: 81.7 + s.h },
      radius_m: 1500,
      step_deg: 1,
      points: horizon(s),
      sun: { date: p.date || date, flat_sunset: null, hidden_at: c ? new Date(c.ms).toISOString() : null, hidden_az_deg: c?.az ?? null, hidden_elev_deg: c?.el ?? null, golden_min_visible: 20 },
    };
  }

  /** SceneMeta + the u16le decimetre binary the real /api/scene serves. */
  function sceneBin(p) {
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
    const bytes = new Uint8Array(n * n * 2);
    const view = new DataView(bytes.buffer);
    hs.forEach((h, i) => view.setUint16(i * 2, Math.min(65534, Math.max(0, Math.round(h * 10))), true));
    const meta = {
      rings: [{ width: n, height: n, cell_m: cell, byte_offset: 0 }],
      base_m: 0,
      nodata: 65535,
      data_url: '/api/scene/demo.bin',
      attribution: 'DEMO height grid — not survey data',
    };
    return { meta, bytes };
  }

  function scene(p) {
    const { meta, bytes } = sceneBin(p);
    return decodeSceneRing(bytes, pickSceneRing(classifyScene(meta), { desktop: true }), meta);
  }

  function buildings() {
    return { buildings: [
      { id: 'demo:1', name: 'DEMO building (not a real listing)', height_m: 42, floors: 12, contacts: [{ role: 'leasing', name: 'DEMO contact' }], matterport: [] },
      { id: 'demo:2', name: 'DEMO neighbouring tower', height_m: 88, floors: 22, contacts: [], matterport: [] },
    ] };
  }

  /** SVG data URL, so the demo photo needs no canvas and also works under Node tests. */
  function photoDataUrl(az) {
    const r = rng(Math.round(az * 9));
    let b = '';
    for (let i = 0; i < 18; i++) {
      const w = 18 + r() * 46, h = 50 + r() * 220;
      b += `<rect x="${i * 42}" y="${Math.round(420 - h)}" width="${Math.round(w)}" height="${Math.round(h)}" fill="#16141c"/>`;
    }
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="720" height="420" viewBox="0 0 720 420"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a1830"/><stop offset=".45" stop-color="#c45a2a"/><stop offset="1" stop-color="#f0c27a"/></linearGradient></defs><rect width="720" height="420" fill="url(#g)"/>${b}<text x="18" y="28" fill="#ffe7c2" font-family="sans-serif" font-size="16" font-weight="600">DEMO aerial — looking ${Math.round(az)}° · not real imagery</text></svg>`;
    return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
  }

  return { search, searchRaw, agent, sun, hz, scene, sceneBin, buildings, photoDataUrl };
}

function demoHeat(lat, lon, spots) {
  const width = 15, height = 15, cell_m = 75;
  const values = [];
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const cy = lat - (y - (height - 1) / 2) * cell_m / 111320;
    const cx = lon + (x - (width - 1) / 2) * cell_m / (111320 * Math.cos(lat * Math.PI / 180));
    const intensity = spots.reduce((best, s) => {
      const dx = (cx - s.lon) * 111320 * Math.cos(lat * Math.PI / 180);
      const dy = (cy - s.lat) * 111320;
      return Math.max(best, s.score * Math.exp(-(dx * dx + dy * dy) / (2 * 160 * 160)));
    }, 0);
    values.push(+intensity.toFixed(3));
  }
  return { center_lat: lat, center_lon: lon, cell_m, width, height, values };
}

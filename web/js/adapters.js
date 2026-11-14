/**
 * Normalize backend JSON into UI shapes.
 * Supports the official docs/contracts.md payloads and the simpler
 * frontend-era shapes ({brief, spots, height grid, JPEG photo}).
 */

export function briefFromForm({ text, date, tz, lat, lon, radius_m }) {
  const q = String(text || '').toLowerCase();
  const radius = clamp(Math.round(radius_m || 400), 50, 800);
  return {
    text: String(text || ''),
    area: { lat: +lat, lon: +lon, radius_m: radius },
    date,
    tz,
    event: /sunrise|rising|dawn/.test(q) ? 'sunrise' : 'sunset',
    standing: /roof/.test(q) ? 'roof' : /street|ground/.test(q) ? 'ground' : 'any',
    framing: /between|gap|framed/.test(q) ? 'between' : /behind|silhouette/.test(q) ? 'behind' : 'clear',
    max_spots: 10,
  };
}

export function normalizeSun(data) {
  if (!data) return { points: [], meta: null };
  if (Array.isArray(data.points)) return { points: data.points.filter(isSunPoint), meta: data };
  return { points: [], meta: data };
}

export function normalizeHorizon(data) {
  if (!data) return { points: [], hidden_at: null, hidden_az_deg: null, hidden_elev_deg: null };
  const points = Array.isArray(data.points) ? data.points.filter(isHzPoint) : [];
  const sun = data.sun || {};
  return {
    points,
    hidden_at: sun.hidden_at || null,
    hidden_az_deg: numOrNull(sun.hidden_az_deg),
    hidden_elev_deg: numOrNull(sun.hidden_elev_deg),
    observer: data.observer || null,
  };
}

export function normalizeSpot(s, i = 0) {
  if (!s) return null;
  const why = Array.isArray(s.why) ? s.why.filter(Boolean).join(' · ') : String(s.why || '');
  const sun = s.sun || {};
  const height_m = firstNum(s.ground_m, s.eye_m);
  return {
    id: String(s.id || `spot-${i + 1}`),
    rank: s.rank ?? i + 1,
    lat: +s.lat,
    lon: +s.lon,
    score: clamp(+s.score || 0, 0, 1),
    height_m,
    height_label: heightLabel(s, height_m),
    standing: s.standing || null,
    sun_visible_until: sun.hidden_at || sun.flat_sunset || null,
    golden_min_visible: sun.golden_min_visible ?? null,
    why: why || 'No reason provided.',
    building_id: s.building_id || null,
    demo: isDemoText(s.id) || isDemoText(why),
  };
}

export function normalizeSearch(data, { demo = false } = {}) {
  const spots = (data?.spots || []).map((s, i) => normalizeSpot(s, i)).filter(Boolean);
  spots.sort((a, b) => b.score - a.score || a.rank - b.rank);

  return {
    spots: spots.map((s) => ({ ...s, demo: s.demo || demo })),

    heat: data?.heat || null,
    searched: data?.searched ?? null,
    took_ms: data?.took_ms ?? null,
    demo,
  };
}

export function composeCallSheet(spots, data, demo) {
  const date = data?.brief?.date || '';
  const brief = data?.brief?.text || data?.brief || '';
  const lines = [
    demo ? 'DEMO CALL SHEET — not a verified location list' : 'CALL SHEET',
    date ? `Shoot date ${date}` : '',
    brief ? `Brief: ${typeof brief === 'string' ? brief : JSON.stringify(brief)}` : '',
    '',
  ];
  spots.slice(0, 4).forEach((s, i) => {
    lines.push(`${i === 0 ? 'Primary' : 'Backup'}: ${s.id}`);
    lines.push(`  ${s.lat.toFixed(5)}, ${s.lon.toFixed(5)} · ${s.height_label}`);
    if (s.sun_visible_until) lines.push(`  Sun blocked / gone: ${s.sun_visible_until}`);
    lines.push(`  ${s.why}`);
    lines.push('');
  });
  if (demo) lines.push('Generated from DEMO geometry. Do not treat as a permitted filming site.');
  return lines.filter((l, i, a) => l !== '' || a[i - 1] !== '').join('\n').trim();
}

export function normalizeBuildings(data, { demo = false } = {}) {
  const list = data?.buildings || [];
  return list.map((b, i) => {
    const mp = Array.isArray(b.matterport) ? b.matterport[0] : null;
    const matterport_id = b.matterport_id || mp?.id || null;
    const matterport_url = mp?.url || (matterport_id ? `https://my.matterport.com/show/?m=${encodeURIComponent(matterport_id)}` : null);
    const contact = formatContact(b);
    const name = b.name || b.address || `Property ${i + 1}`;
    return {
      id: b.id || null,
      name,
      address: b.address || null,
      height_m: numOrNull(b.height_m),
      floors: b.floors ?? null,
      contact,
      matterport_id,
      matterport_url,
      property_type: b.property_type || null,
      demo: demo || isDemoText(name) || isDemoText(contact),
    };
  });
}

export function classifyScene(data) {
  if (!data) return null;
  if (Array.isArray(data.heights) && (data.n || data.width)) {
    const n = data.n || data.width;
    return {
      kind: 'grid',
      n,
      cell_m: data.cell_m || 1,
      heights: data.heights,
      attribution: data.attribution || null,
    };
  }
  if (Array.isArray(data.rings) && data.data_url) {
    return { kind: 'rings', ...data };
  }
  return null;
}

export function decodeSceneRing(bytes, ring, meta) {
  const view = new DataView(bytes.buffer, bytes.byteOffset + (ring.byte_offset || 0), ring.width * ring.height * 2);
  const heights = new Float32Array(ring.width * ring.height);
  const nodata = meta.nodata ?? 65535;
  const base = meta.base_m || 0;
  for (let i = 0; i < heights.length; i++) {
    const v = view.getUint16(i * 2, true);
    heights[i] = v === nodata ? NaN : base + v / 10;
  }
  return { n: ring.width, cell_m: ring.cell_m, heights, attribution: meta.attribution || null, kind: 'grid' };
}

export function pickSceneRing(meta, { desktop = true } = {}) {
  const rings = meta?.rings || [];
  if (!rings.length) return null;
  if (desktop) return rings[0];
  return rings[Math.min(1, rings.length - 1)];
}

export function isSunPoint(p) {
  return p && Number.isFinite(+p.az_deg) && Number.isFinite(+p.elev_deg) && p.t;
}

export function isHzPoint(p) {
  return p && Number.isFinite(+p.az_deg) && Number.isFinite(+p.elev_deg);
}

function formatContact(b) {
  if (typeof b.contact === 'string' && b.contact.trim()) return b.contact.trim();
  const contacts = Array.isArray(b.contacts) ? b.contacts : [];
  const c = contacts[0];
  if (!c) return '';
  return [c.role && titleCase(c.role), c.name, c.company, c.phone, c.email].filter(Boolean).join(' · ');
}

function relativeHeight(s) {
  if (s.eye_m == null || s.ground_m == null) return null;
  const d = +s.eye_m - +s.ground_m;
  if (d > 0 && d < 8) return +s.eye_m;
  return +s.ground_m;
}

function heightLabel(s, height_m) {

  if (s.eye_m != null) return `${Math.round(+s.eye_m)} m eye height (DSM)`;
  if (height_m != null) return `${Math.round(height_m)} m`;
  return 'Height unknown';
}

function isDemoText(s) {
  return /demo|sample|\(sample\)/i.test(String(s || ''));
}

function firstNum(...vals) {
  for (const v of vals) {
    const n = Number(v);
    if (Number.isFinite(n)) return n;
  }
  return 0;
}

function numOrNull(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

function clamp(n, a, b) {
  return Math.max(a, Math.min(b, n));
}

function titleCase(s) {
  return String(s).replace(/^\w/, (c) => c.toUpperCase());
}

export function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  }[c]));
}

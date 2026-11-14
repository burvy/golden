import { createClient } from './api.js';
import { esc } from './adapters.js';
import { blockTime, formatClock, formatTime, horizonFn, interpSun, localDateISO } from './geo.js';
import { createMap } from './map.js';
import { buildScene, currentYaw, renderAt } from './scene3d.js';

const $ = (s, r = document) => r.querySelector(s);
const CFG = { base: localStorage.getItem('golden.api') || 'http://localhost:8787' };

const state = {
  lat: 40.758,
  lon: -73.9855,
  name: 'Set a location',
  tz: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
  radius_m: 400,
  spots: [],
  heat: null,
  path: [],
  sheet: '',
  demo: false,
  sel: null,
  d: {},
  tab: 'h',
  t: Date.now(),
  tok: 0,
  rise: false,
  live: {},
  photoUrl: null,
};

const api = createClient(() => CFG.base);
const map = createMap({
  host: $('#map'),
  getBase: () => CFG.base,
  onSelect: (id) => {
    const sp = state.spots.find((s) => s.id === id);
    if (sp) select(sp);
  },
  onMove: (c) => {
    state.lat = c.lat;
    state.lon = c.lon;
    state.radius_m = c.radius_m;
    $('#areaHint').textContent = `${c.lat.toFixed(4)}, ${c.lon.toFixed(4)} · ${c.radius_m} m search radius`;
  },
});

function setApiLabel() {
  const live = api.status();
  const keys = ['/api/search', '/api/sun', '/api/horizon', '/api/agent', '/api/scene', '/api/photo', '/api/buildings'];
  const down = keys.filter((k) => live[k] === false);
  const up = keys.filter((k) => live[k] === true);
  const rejected = Object.entries(api.errors()).filter(([, m]) => /^HTTP 4/.test(m)).map(([k, m]) => `${k} (${m})`);
  const el = $('#apiStatus');
  if (rejected.length) el.textContent = `Server rejected ${rejected.join(', ')}. Possible contract mismatch; showing DEMO data`;
  else if (!up.length && !down.length) el.textContent = 'Waiting for the first request';
  else if (down.length && !up.length) el.textContent = 'Server unreachable — showing labeled DEMO data';
  else if (down.length) el.textContent = `Live API · DEMO fallback for ${down.join(', ')}`;
  else el.textContent = 'Connected to ' + CFG.base;
}

function showApp(detail) {
  $('#app').hidden = false;
  if (detail?.lat != null) {
    state.lat = detail.lat;
    state.lon = detail.lon;
    if (detail.tz) state.tz = detail.tz;
    if (detail.name) state.name = detail.name;
    $('#placeName').textContent = state.name;
    $('#tzLabel').textContent = state.tz;
  }
  map.init({ lat: state.lat, lon: state.lon });
  requestAnimationFrame(() => {
    map.resize();
    map.setView(state.lat, state.lon, 14.2);
  });
}

async function boot() {
  $('#base').value = CFG.base;
  $('#date').value = localDateISO(state.tz);
  $('#tzLabel').textContent = state.tz;
  ping();
  await locate({ silent: true });
  if (!$('#hero')) showApp();
}

async function ping() {
  await api.health();
  setApiLabel();
}

async function locate({ silent } = {}) {
  const pos = await new Promise((resolve) => {
    if (!navigator.geolocation) return resolve(null);
    navigator.geolocation.getCurrentPosition(
      (p) => resolve({ lat: p.coords.latitude, lon: p.coords.longitude, source: 'gps' }),
      () => resolve(null),
      { timeout: 7000, maximumAge: 600000 },
    );
  });
  if (pos) {
    await setPlace({ ...pos, name: 'Your location' });
    return;
  }
  if (!silent) setBanner('Location permission denied. Search a city instead.', 'warn');
}

async function setPlace({ lat, lon, name }) {
  state.lat = lat;
  state.lon = lon;
  state.name = name || 'Selected place';
  $('#placeName').textContent = state.name;
  map.setView(lat, lon, 14.4);
  await refreshPlaceMeta();
}

async function refreshPlaceMeta() {
  try {
    const u = `https://api.open-meteo.com/v1/forecast?latitude=${state.lat}&longitude=${state.lon}&current=temperature_2m,weather_code&timezone=auto&forecast_days=1`;
    const j = await (await fetch(u)).json();
    if (j.timezone) {
      state.tz = j.timezone;
      $('#tzLabel').textContent = state.tz;
      if (!$('#date').value) $('#date').value = localDateISO(state.tz);
    }
    if (j.current) {
      $('#wxChip').textContent = `${Math.round(j.current.temperature_2m)}°C`;
    }
  } catch {
    $('#wxChip').textContent = 'Weather n/a';
  }
}

async function geocode(q) {
  const url = `https://nominatim.openstreetmap.org/search?format=jsonv2&limit=5&q=${encodeURIComponent(q)}`;
  const r = await fetch(url, { headers: { Accept: 'application/json' } });
  if (!r.ok) throw new Error('geocode failed');
  return r.json();
}

async function search() {
  const brief = $('#brief').value.trim();
  if (!brief) {
    setBanner('Describe the shot you want to find.', 'warn');
    return;
  }
  state.rise = /sunrise|rising|dawn/i.test(brief);
  $('#find').disabled = true;
  $('#list').innerHTML = '<div class="msg">Searching the selected area…</div>';
  setBanner('Scoring candidate standpoints…', 'info');
  const ctx = {
    text: brief,
    date: $('#date').value,
    tz: state.tz,
    lat: state.lat,
    lon: state.lon,
    radius_m: state.radius_m,
  };
  try {
    const r = await api.search(ctx);
    state.spots = r.spots;
    state.heat = r.heat;
    map.setHeat(r.heat);
    state.path = [];
    state.sheet = '';
    state.demo = r.demo;
    $('#sheet').hidden = true;
    renderList();
    const az = state.path.length ? interpSun(state.path, Date.parse(state.spots[0]?.sun_visible_until || Date.now())).az : null;
    map.setSpots(state.spots, state.spots[0]?.id, az);
    if (r.demo) setBanner('DEMO results — synthetic geometry around your selected map location, not verified filming sites.', 'demo');
    else setBanner(`${state.spots.length} ranked spots · ${state.tz}`, 'ok');
    if (state.spots.length) select(state.spots[0]);
    else $('#list').innerHTML = '<div class="msg">No spots matched. Try another brief, date, or pan the map.</div>';
    // Call sheet is an independent agent request, not part of SearchResult.
    api.agent(ctx).then((sheet) => {
      state.sheet = formatCallSheet(sheet);
      $('#sheetText').textContent = state.sheet;
      $('#sheet').hidden = false;
      setApiLabel();
    }).catch(() => setApiLabel());
  } catch (e) {
    $('#list').innerHTML = `<div class="msg err">${esc(e.message || 'Search failed')}</div>`;
    setBanner('Search failed. Check the API base URL.', 'err');
  }
  $('#find').disabled = false;
  setApiLabel();
}

function formatCallSheet(sheet) {
  const lines = [sheet.title || 'Call sheet', sheet.summary || ''];
  for (const item of sheet.timeline || []) lines.push(`${formatTime(item.t, state.tz)} — ${item.what}`);
  for (const item of sheet.spots || []) lines.push(`${item.role}: ${item.spot?.id || 'Unknown spot'} — arrive ${formatTime(item.arrive_by, state.tz)}`);
  for (const item of sheet.access || []) lines.push(`Access: ${item.building_name || item.building_id || 'Building'} — ${item.ask || 'Contact property manager'}`);
  for (const caveat of sheet.caveats || []) lines.push(`Note: ${caveat}`);
  if (sheet.model === 'mock') lines.unshift('DEMO AGENT OUTPUT — verify before filming');
  return lines.filter(Boolean).join('\n');
}

function renderList() {
  const L = $('#list');
  if (!state.spots.length) return;
  L.innerHTML = state.spots.map((s) => `
    <button class="spot" type="button" data-id="${esc(s.id)}" aria-pressed="${state.sel && state.sel.id === s.id}">
      <span class="sc">${Math.round(s.score * 100)}</span>
      <span>
        <b>${esc(s.id)}</b>
        ${s.demo ? '<em class="pill">DEMO</em>' : ''}
        <small>${esc(s.height_label)} · sun until ${esc(formatTime(s.sun_visible_until, state.tz))}</small>
        <small>${esc(s.why)}</small>
      </span>
    </button>`).join('');
  L.querySelectorAll('.spot').forEach((b) => {
    b.onclick = () => select(state.spots.find((s) => s.id === b.dataset.id));
  });
}

async function select(sp) {
  if (!sp) return;
  const tok = ++state.tok;
  state.sel = sp;
  state.d = {};
  state.tab = 'h';
  renderList();
  const az0 = state.path.length ? interpSun(state.path, Date.parse(sp.sun_visible_until || Date.now())).az : 270;
  map.setSpots(state.spots, sp.id, az0);
  renderCard();
  const ctx = { lat: sp.lat, lon: sp.lon, date: $('#date').value, tz: state.tz, desktop: matchMedia('(min-width: 900px)').matches };
  const paint = () => { if (tok === state.tok) { renderCard(); setApiLabel(); } };
  api.sun(ctx).then((r) => {
    if (tok !== state.tok) return;
    state.d.sun = r.points;
    state.d.sunDemo = !!r.demo;
    state.path = r.points;
    state.d.sunMeta = r.meta;
    state.d.sunDemo = !!r.demo;
    state.t = Date.parse(sp.sun_visible_until) || Date.parse(r.points[Math.floor(r.points.length * 0.8)]?.t) || Date.now();
    paint();
  }).catch(() => { if (tok === state.tok) { state.d.sun = false; paint(); } });
  api.horizon(ctx).then((r) => {
    if (tok !== state.tok) return;
    state.d.hz = r.points;
    state.d.hzDemo = !!r.demo;
    state.d.hzMeta = r;
    paint();
  }).catch(() => { if (tok !== state.tok) return; state.d.hz = false; paint(); });
  api.scene(ctx).then((r) => { if (tok === state.tok) { state.d.scene = r; paint(); } }).catch(() => { if (tok === state.tok) { state.d.scene = false; paint(); } });
  api.buildings(ctx).then((r) => { if (tok === state.tok) { state.d.bld = r; paint(); } }).catch(() => { if (tok === state.tok) { state.d.bld = false; paint(); } });
}

function cross() {
  const d = state.d;
  if (d.hzMeta?.hidden_at && Number.isFinite(Date.parse(d.hzMeta.hidden_at))) {
    return { ms: Date.parse(d.hzMeta.hidden_at), az: d.hzMeta.hidden_az_deg ?? 270, el: d.hzMeta.hidden_elev_deg ?? 0, fromServer: true };
  }
  if (!d.sun?.length || !d.hz?.length) return null;
  return blockTime(d.sun, horizonFn(d.hz), state.rise);
}

function pano(c) {
  const W = 720, H = 280, gy = H - 24, sc = 9, a0 = c.az - 40;
  const X = (a) => ((a - a0) / 80) * W;
  const Y = (e) => gy - e * sc;
  const hf = horizonFn(state.d.hz);
  const sp = interpSun(state.d.sun, state.t);
  const t = Math.max(0, Math.min(1, (sp.el + 4) / 22));
  const top = mix([40, 22, 48], [70, 110, 170], t);
  const bot = mix([255, 110, 50], [250, 200, 140], t);
  let path = '', ticks = '';
  for (const p of state.d.sun) {
    if (p.az_deg < a0 - 2 || p.az_deg > a0 + 82 || p.elev_deg < -1) continue;
    path += (path ? 'L' : 'M') + X(p.az_deg).toFixed(1) + ' ' + Y(p.elev_deg).toFixed(1);
    const mn = +new Date(p.t).toLocaleString('en-US', { timeZone: state.tz, minute: 'numeric' });
    if (mn === 0 && p.elev_deg < 27) {
      ticks += `<circle cx="${X(p.az_deg)}" cy="${Y(p.elev_deg)}" r="3" fill="#fff"/><text x="${X(p.az_deg) + 7}" y="${Y(p.elev_deg) - 6}" fill="#fff" font-size="12">${esc(formatTime(p.t, state.tz))}</text>`;
    }
  }
  let sil = `M0 ${H}`;
  for (let x = 0; x <= W; x += 2) sil += `L${x} ${Y(hf(a0 + (x / W) * 80))}`;
  sil += `L${W} ${H}Z`;
  const sx = X(sp.az), sy = Y(sp.el);
  return `<svg viewBox="0 0 ${W} ${H}" role="img" aria-label="Simulated skyline with sun path"><defs><linearGradient id="sk" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="rgb(${top})"/><stop offset="1" stop-color="rgb(${bot})"/></linearGradient><radialGradient id="gl"><stop offset="0" stop-color="#fff2c4" stop-opacity=".9"/><stop offset="1" stop-color="#ffb04a" stop-opacity="0"/></radialGradient></defs><rect width="${W}" height="${H}" fill="url(#sk)"/>${sp.el > -3 ? `<circle cx="${sx}" cy="${sy}" r="46" fill="url(#gl)"/><circle cx="${sx}" cy="${sy}" r="11" fill="#fff6d8"/>` : ''}<path d="${path}" fill="none" stroke="#fff" stroke-opacity=".8" stroke-width="2" stroke-dasharray="5 5"/>${ticks}<path d="${sil}" fill="#141218"/><rect y="${gy}" width="${W}" height="24" fill="#0c0b10"/><text x="10" y="${H - 7}" fill="#9aa4ba" font-size="12">${Math.round(a0)}°</text><text x="${W / 2 - 14}" y="${H - 7}" fill="#9aa4ba" font-size="12">${Math.round(c.az)}°</text><text x="${W - 40}" y="${H - 7}" fill="#9aa4ba" font-size="12">${Math.round(a0 + 80)}°</text></svg>`;
}

function mix(a, b, t) {
  return a.map((v, i) => Math.round(v + (b[i] - v) * t));
}

function renderCard() {
  const sp = state.sel, d = state.d;
  if (!sp) {
    $('#card').innerHTML = '<div class="msg">Pick a ranked spot to open the shot preview.</div>';
    return;
  }
  const c = cross();
  const ready = d.sun && d.hz && d.sun !== false && d.hz !== false;
  let head = `<div class="card-head"><h2>${esc(sp.id)}</h2>
    <p class="mute">${esc(sp.height_label)} · ${sp.lat.toFixed(5)}, ${sp.lon.toFixed(5)}${sp.demo ? ' · DEMO' : ''}</p>
    <p>${esc(sp.why)}</p>`;
  if (c) {
    const hf = d.hz ? horizonFn(d.hz) : () => 0;
    const tw = hf(c.az) > 1.5;
    const label = state.rise ? 'Sun clears the skyline' : tw ? 'Sun drops behind buildings' : 'Sun sets over open horizon';
    head += `<p class="gold"><b>${label} at ${esc(formatClock(c.ms, state.tz))}</b>${sp.demo || d.hzDemo || d.sunDemo ? ' <em class="pill">DEMO ESTIMATE</em>' : ''}</p>`;
  }
  head += '</div>';
  const tabs = `<div class="row tabs" role="tablist">
    <button type="button" role="tab" aria-selected="${state.tab === 'h'}" data-t="h">Skyline</button>
    <button type="button" role="tab" aria-selected="${state.tab === '3'}" data-t="3">3D height grid</button>
  </div>`;
  let view;
  if (d.sun === false || d.hz === false) view = '<div class="msg err">Could not load sun path or horizon for this spot.</div>';
  else if (!ready) view = '<div class="msg">Loading skyline and sun path…</div>';
  else if (!c) view = '<div class="msg">No valid sun/horizon crossing is available for this date.</div>';
  else view = state.tab === 'h' ? pano(c) : '<div id="v3" class="v3"></div>';
  let sl = '';
  if (ready && d.sun.length) {
    const loP = d.sun.find((p) => p.elev_deg > -2) || d.sun[0];
    const hiP = [...d.sun].reverse().find((p) => p.elev_deg > -2) || d.sun[d.sun.length - 1];
    sl = `<div><label for="tm">Time in ${esc(state.tz)}: ${esc(formatClock(state.t, state.tz))}</label>
      <input id="tm" type="range" min="${Date.parse(loP.t)}" max="${Date.parse(hiP.t)}" step="60000" value="${state.t}"></div>`;
  }
  const bl = renderBuildings(d.bld);
  const simNote = `<p class="note">Skyline and 3D view are a height-grid simulation, not a photorealistic reconstruction.${d.sunDemo || d.hzDemo ? ' Sun/horizon data are DEMO estimates because the server is unavailable.' : ''}${d.scene?.demo ? ' Height grid is DEMO geometry.' : ''}</p>`;
  $('#card').innerHTML = head + tabs + `<div class="view">${view}</div>` + sl + simNote + `<div class="cols">
    <div><h3>Aerial view</h3><img id="photo" alt="Aerial view toward the sun"><p class="note" id="pnote"></p></div>
    <div><h3>Property contacts</h3>${bl}</div>
  </div>`;
  document.querySelectorAll('[data-t]').forEach((b) => {
    b.onclick = () => { state.tab = b.dataset.t; renderCard(); };
  });
  const tm = $('#tm');
  if (tm) tm.oninput = (e) => {
    state.t = +e.target.value;
    document.querySelector('label[for=tm]').textContent = `Time in ${state.tz}: ${formatClock(state.t, state.tz)}`;
    if (state.tab === 'h' && ready && c) document.querySelector('.view').innerHTML = pano(c);
    else renderAt(state.t);
  };
  if (d.photo) { const im = $('#photo'); if (im && d.photo.url) im.src = d.photo.url; $('#pnote').textContent = d.photo.note; }
  else if (c && !d.photoLoading) { d.photoLoading = true; loadPhoto(c.az); }
  if (state.tab === '3' && ready && d.scene && d.scene !== false) {
    buildScene({ host: $('#v3'), sceneGrid: d.scene, sunPath: d.sun, horizon: d.hz, spot: sp, timeMs: state.t, yaw: c.az });
  } else if (state.tab === '3' && d.scene === false) {
    document.querySelector('.view').innerHTML = '<div class="msg err">Could not load the height grid.</div>';
  }
}

function renderBuildings(bld) {
  if (bld === false) return '<div class="msg err">Could not load building contacts.</div>';
  if (!bld) return '<div class="msg">Loading buildings…</div>';
  if (!bld.length) return '<div class="msg">No CoStar properties in this radius. An empty list is normal for streets and houses.</div>';
  return bld.map((b) => `<div class="bld">
    <b>${esc(b.name)}</b>${b.demo ? ' <em class="pill">DEMO</em>' : ''}<br>
    <span class="mute">${b.height_m != null ? Math.round(b.height_m * 3.28084) + ' ft' : 'Height n/a'}${b.floors != null ? ' · ' + b.floors + ' floors' : ''}${b.address ? ' · ' + esc(b.address) : ''}</span><br>
    ${esc(b.contact) || 'No contact on file'}
    ${b.matterport_url ? ` · <a href="${esc(b.matterport_url)}" target="_blank" rel="noopener">Matterport tour</a>` : ''}
  </div>`).join('');
}

async function loadPhoto(az) {
  if (!state.sel) return;
  const selectedId = state.sel.id;
  const d = state.d;
  try {
    const p = await api.photo({ lat: state.sel.lat, lon: state.sel.lon, az_deg: az, tz: state.tz, date: $('#date').value });
    if (state.sel?.id !== selectedId) return;
    d.photo = { url: p.url, note: (p.demo ? 'DEMO illustration. ' : '') + (p.attribution || '') + (p.captured ? ` · captured ${p.captured}` : '') };
  } catch {
    if (state.sel?.id !== selectedId) return;
    d.photo = { url: '', note: 'Aerial photo unavailable.' };
  }
  // The card re-renders while the photo loads, so look the elements up again.
  const im = $('#photo'), note = $('#pnote');
  if (im && d.photo.url) im.src = d.photo.url;
  if (note) note.textContent = d.photo.note;
  setApiLabel();
}

function setBanner(text, kind) {
  const el = $('#banner');
  el.textContent = text;
  el.dataset.kind = kind || 'info';
}

window.addEventListener('golden:enter', (e) => showApp(e.detail));
$('#hgo')?.addEventListener('click', () => {
  // golden-hero.js owns the cinematic close. If it never started (script blocked or
  // failed), close the overlay here so the app is never stuck behind it.
  setTimeout(() => {
    const hero = $('#hero');
    if (!window.goldenHeroReady && hero) {
      hero.classList.add('out');
      setTimeout(() => hero.remove(), 1100);
    }
    if ($('#app').hidden) showApp();
  }, 250);
});

$('#find').onclick = search;
$('#locate').onclick = () => locate({ silent: false });
$('#save').onclick = () => {
  CFG.base = $('#base').value.replace(/\/$/, '');
  localStorage.setItem('golden.api', CFG.base);
  ping();
};
$('#copy').onclick = async (e) => {
  try {
    await navigator.clipboard.writeText(state.sheet);
    e.target.textContent = 'Copied';
  } catch {
    e.target.textContent = 'Select the text';
  }
  setTimeout(() => { e.target.textContent = 'Copy call sheet'; }, 1600);
};

const q = $('#placeQ');
const sug = $('#suggest');
let gtimer = 0;
q.addEventListener('input', () => {
  clearTimeout(gtimer);
  const v = q.value.trim();
  if (v.length < 3) { sug.hidden = true; return; }
  gtimer = setTimeout(async () => {
    try {
      const rows = await geocode(v);
      if (!rows.length) { sug.hidden = true; return; }
      sug.hidden = false;
      sug.innerHTML = rows.map((r, i) => `<button type="button" data-i="${i}">${esc(r.display_name)}</button>`).join('');
      sug.querySelectorAll('button').forEach((b) => {
        b.onclick = () => {
          const row = rows[+b.dataset.i];
          sug.hidden = true;
          q.value = row.display_name;
          setPlace({ lat: +row.lat, lon: +row.lon, name: row.display_name.split(',').slice(0, 2).join(',') });
        };
      });
    } catch {
      sug.hidden = true;
    }
  }, 280);
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.target === $('#brief') || e.target === q)) {
    if (e.target === $('#brief')) search();
  }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    if (!state.spots.length) return;
    const i = state.spots.findIndex((s) => s.id === state.sel?.id);
    const n = e.key === 'ArrowDown' ? i + 1 : i - 1;
    if (n >= 0 && n < state.spots.length) {
      e.preventDefault();
      select(state.spots[n]);
    }
  }
});

boot();
void currentYaw;

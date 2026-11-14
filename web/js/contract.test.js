import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { createClient } from './api.js';
import { createMock } from './mock.js';
import { briefFromForm, classifyScene, decodeSceneRing, normalizeBuildings, normalizeHorizon, normalizeSearch, normalizeSun } from './adapters.js';
import { blockTime, horizonFn } from './geo.js';

const ctx = { lat: 40.758, lon: -73.9855, tz: 'America/New_York', date: '2026-11-14', text: 'sunset between two towers, rooftop', radius_m: 400 };
const spotCtx = (s) => ({ lat: s.lat, lon: s.lon, date: ctx.date, tz: ctx.tz, desktop: true, az_deg: 250 });
const ENDPOINTS = ['/api/search', '/api/agent', '/api/sun', '/api/horizon', '/api/scene', '/api/photo', '/api/buildings', '/api/health'];
const realFetch = globalThis.fetch;

function stubFetch(handler) {
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    const u = new URL(String(url));
    calls.push({ path: u.pathname, params: Object.fromEntries(u.searchParams), method: init.method || 'GET', body: init.body ? JSON.parse(init.body) : null });
    return handler(u, init);
  };
  return calls;
}
const serverDown = () => { throw new TypeError('fetch failed'); };
const json = (o, status = 200) => new Response(JSON.stringify(o), { status, headers: { 'content-type': 'application/json' } });

test.afterEach(() => { globalThis.fetch = realFetch; });

test('server down: every client call falls back to DEMO data and says so', async () => {
  stubFetch(serverDown);
  const api = createClient(() => 'http://localhost:8787');
  const res = await api.search(ctx);
  assert.equal(res.demo, true);
  assert.ok(res.spots.length >= 2);
  const sp = res.spots[0];
  const [sun, hz, scene, photo, bld, sheet] = await Promise.all([
    api.sun(spotCtx(sp)), api.horizon(spotCtx(sp)), api.scene(spotCtx(sp)), api.photo(spotCtx(sp)), api.buildings(spotCtx(sp)), api.agent(ctx),
  ]);
  assert.ok(sun.demo && sun.points.length > 10);
  assert.ok(hz.demo && hz.points.length === 360);
  assert.ok(scene.demo && scene.heights.length === scene.n * scene.n && scene.heights.some(Number.isFinite));
  assert.ok(photo.demo && photo.url.startsWith('data:image/svg+xml'));
  assert.ok(bld.length > 0 && bld.every((b) => b.demo));
  assert.equal(sheet.model, 'mock');
  assert.ok(Object.values(api.status()).every((v) => v === false));
});

test('requests use the contract endpoints, methods and field names', async () => {
  const calls = stubFetch(serverDown);
  const api = createClient(() => 'http://localhost:8787/');
  const res = await api.search(ctx);
  const sp = res.spots[0];
  await Promise.all([api.sun(spotCtx(sp)), api.horizon(spotCtx(sp)), api.scene(spotCtx(sp)), api.photo(spotCtx(sp)), api.buildings(spotCtx(sp)), api.agent(ctx)]);
  const by = (p) => calls.find((c) => c.path === p);
  assert.ok(calls.every((c) => ENDPOINTS.includes(c.path)), `unexpected endpoint: ${calls.map((c) => c.path)}`);

  const search = by('/api/search');
  assert.equal(search.method, 'POST');
  assert.deepEqual(Object.keys(search.body), ['text', 'area', 'date', 'tz', 'event', 'standing', 'framing', 'max_spots']);
  assert.deepEqual(Object.keys(search.body.area), ['lat', 'lon', 'radius_m']);

  assert.equal(by('/api/agent').method, 'POST');
  assert.deepEqual(Object.keys(by('/api/agent').body), ['text', 'area', 'date', 'tz']);
  for (const p of ['/api/sun', '/api/horizon', '/api/scene', '/api/photo', '/api/buildings']) {
    assert.equal(by(p).method, 'GET', p);
    assert.ok(by(p).params.lat && by(p).params.lon, `${p} needs lat & lon`);
  }
  assert.ok(by('/api/sun').params.date);
  assert.ok(by('/api/horizon').params.above_m && by('/api/horizon').params.date);
  assert.ok(by('/api/scene').params.radius_m);
  assert.ok(by('/api/buildings').params.radius_m);
});

test('old DESIGN-draft shapes are gone', async () => {
  const calls = stubFetch(serverDown);
  const api = createClient(() => 'http://localhost:8787');
  await api.search(ctx);
  await api.horizon(spotCtx({ lat: 40.76, lon: -73.98 }));
  assert.ok(!calls.some((c) => c.path === '/api/spot/horizon'));
  const body = calls.find((c) => c.path === '/api/search').body;
  assert.ok(!('brief' in body) && !('bbox' in body.area));
});

test('live server: contract-shaped search is used as is, not DEMO', async () => {
  const raw = createMock(ctx).searchRaw();
  raw.spots.forEach((s) => { s.id = s.id.replace('DEMO ', 'Real '); s.why = ['tower gap']; });
  stubFetch((u) => (u.pathname === '/api/search' ? json(raw) : serverDown()));
  const api = createClient(() => 'http://localhost:8787');
  const res = await api.search(ctx);
  assert.equal(res.demo, false);
  assert.ok(res.spots.every((s) => !s.demo));
  assert.equal(api.status()['/api/search'], true);
  assert.ok(res.spots.every((s, i, a) => i === 0 || a[i - 1].score >= s.score));
});

test('a 4xx from the server is reported as a possible contract mismatch', async () => {
  stubFetch(() => json({ error: 'brief.area.radius_m out of range' }, 400));
  const api = createClient(() => 'http://localhost:8787');
  const res = await api.search(ctx);
  assert.equal(res.demo, true);
  assert.match(api.errors()['/api/search'], /^HTTP 400: brief\.area/);
});

test('mock search is contract-shaped: ground_m/eye_m + sun{}, heat, no sun_path or call sheet', () => {
  const raw = createMock(ctx).searchRaw();
  assert.deepEqual(Object.keys(raw.brief), Object.keys(briefFromForm(ctx)));
  assert.ok(!('sun_path' in raw) && !('call_sheet_text' in raw));
  assert.equal(raw.heat.values.length, raw.heat.width * raw.heat.height);
  for (const s of raw.spots) {
    assert.ok(Number.isFinite(s.ground_m) && Number.isFinite(s.eye_m));
    assert.ok(typeof s.sun.hidden_at === 'string' && 'golden_min_visible' in s.sun);
    assert.ok(!('height_m' in s) && !('sun_visible_until' in s));
    assert.ok(Array.isArray(s.why));
  }
});

test('mock sun/horizon/buildings/agent/scene match their contract wrappers', () => {
  const m = createMock(ctx);
  const sp = m.searchRaw().spots[0];
  const sun = m.sun({ lat: sp.lat, lon: sp.lon, date: ctx.date });
  assert.ok(!Array.isArray(sun) && Array.isArray(sun.points));
  assert.equal(normalizeSun(sun).points.length, sun.points.length);
  const hz = m.hz({ lat: sp.lat, lon: sp.lon, date: ctx.date });
  assert.ok(hz.observer && hz.sun && typeof hz.sun.hidden_at === 'string' && hz.points.length === 360);
  assert.equal(normalizeHorizon(hz).hidden_at, hz.sun.hidden_at);
  const b = m.buildings();
  assert.ok(Array.isArray(b.buildings) && b.buildings.every((x) => Array.isArray(x.contacts) && Array.isArray(x.matterport)));
  assert.ok(normalizeBuildings(b, { demo: true })[0].contact.includes('DEMO contact'));
  const ag = m.agent();
  for (const k of ['title', 'brief', 'summary', 'timeline', 'spots', 'access', 'caveats', 'model']) assert.ok(k in ag, k);
  const { meta, bytes } = m.sceneBin({ lat: sp.lat, lon: sp.lon, radius_m: 200 });
  const sm = classifyScene(meta);
  assert.equal(sm.kind, 'rings');
  const g = decodeSceneRing(bytes, sm.rings[0], meta);
  assert.equal(g.heights.length, g.n * g.n);
  assert.ok(g.heights.every(Number.isFinite));
});

test('hero flow: ranked list and the clicked spot card agree on when the sun is blocked', async () => {
  stubFetch(serverDown);
  const api = createClient(() => 'http://localhost:8787');
  const res = await api.search(ctx);
  for (const sp of res.spots) {
    const [sun, hz] = await Promise.all([api.sun(spotCtx(sp)), api.horizon(spotCtx(sp))]);
    const listed = Date.parse(sp.sun_visible_until);
    assert.ok(Math.abs(Date.parse(hz.hidden_at) - listed) <= 60000, `${sp.id}: horizon hidden_at differs from list`);
    const client = blockTime(sun.points, horizonFn(hz.points), false);
    assert.ok(client && Math.abs(client.ms - listed) <= 60000, `${sp.id}: client crossing differs from list`);
  }
});

test('demo story: the top spot has a tower that hides the sun, and the card can name the time', async () => {
  stubFetch(serverDown);
  const api = createClient(() => 'http://localhost:8787');
  const top = (await api.search(ctx)).spots[0];
  const hz = await api.horizon(spotCtx(top));
  assert.ok(hz.hidden_az_deg != null && hz.hidden_elev_deg > 1.5, 'sun should disappear behind a tower, not the flat horizon');
  assert.ok(horizonFn(hz.points)(hz.hidden_az_deg) > 1.5);
});

test('normalizeSearch drops spots without coordinates', () => {
  const r = normalizeSearch({ spots: [{ id: 'bad', score: 1 }, { id: 'ok', lat: 40, lon: -74, score: 0.5 }] });
  assert.deepEqual(r.spots.map((s) => s.id), ['ok']);
});

// When the server team commits fixtures/*.json these run against the same adapters the UI uses.
const FIXTURES = new URL('../../fixtures/', import.meta.url);
test('fixtures/*.json parse through the adapters', { skip: !existsSync(FIXTURES) && 'no fixtures/ directory yet' }, () => {
  const files = readdirSync(FIXTURES).filter((f) => f.endsWith('.json'));
  assert.ok(files.length > 0);
  for (const f of files) {
    const j = JSON.parse(readFileSync(new URL(f, FIXTURES), 'utf8'));
    if (j.spots && j.heat) assert.ok(normalizeSearch(j).spots.length > 0, f);
    else if (Array.isArray(j.points) && j.points[0]?.t) assert.ok(normalizeSun(j).points.length > 0, f);
    else if (Array.isArray(j.points)) assert.ok(normalizeHorizon(j).points.length > 0, f);
    else if (j.buildings) assert.ok(Array.isArray(normalizeBuildings(j)), f);
    else if (j.rings) assert.equal(classifyScene(j).kind, 'rings', f);
  }
});

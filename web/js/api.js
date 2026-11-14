import {
  briefFromForm,
  classifyScene,
  decodeSceneRing,
  normalizeBuildings,
  normalizeHorizon,
  normalizeSearch,
  normalizeSun,
  pickSceneRing,
} from './adapters.js';
import { createMock } from './mock.js';

export function createClient(getBase) {
  const live = {};
  const errors = {};
  /** DEMO instance of the latest search area, so per-spot fallbacks match the ranked list. */
  let demoMock = null;
  const mockFor = (ctx) => demoMock || createMock(ctx);

  function base() {
    return String(getBase() || 'http://localhost:8787').replace(/\/$/, '');
  }

  async function raw(path, { method = 'GET', params, body, timeout = 20000, as = 'auto' } = {}) {
    const qs = params ? '?' + new URLSearchParams(cleanParams(params)) : '';
    const ctrl = new AbortController();
    const to = setTimeout(() => ctrl.abort(), timeout);
    const headers = {};
    if (body) headers['Content-Type'] = 'application/json';
    let res;
    try {
      res = await fetch(base() + path + qs, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined,
        signal: ctrl.signal,
      });
    } finally {
      clearTimeout(to);
    }
    if (!res.ok) {
      let msg = `HTTP ${res.status}`;
      try {
        const j = await res.clone().json();
        if (j?.error) msg = j.error;
      } catch { /* text errors are fine */ }
      const err = new Error(msg);
      err.status = res.status;
      throw err;
    }
    if (as === 'blob') return res.blob();
    if (as === 'bytes') return new Uint8Array(await res.arrayBuffer());
    const ct = res.headers.get('content-type') || '';
    if (as === 'auto' && ct.includes('application/json')) return res.json();
    if (as === 'json') return res.json();
    if (ct.includes('image/') || ct.includes('octet-stream')) return res;
    const text = await res.text();
    try { return JSON.parse(text); } catch { return text; }
  }

  async function tryLive(key, fn) {
    try {
      const out = await fn();
      live[key] = true;
      delete errors[key];
      return out;
    } catch (e) {
      live[key] = false;
      errors[key] = e.status ? `HTTP ${e.status}: ${e.message}` : 'unreachable';
      console.warn(`[golden] ${key} -> DEMO fallback (${errors[key]})`);
      throw e;
    }
  }

  async function search(ctx) {
    const official = briefFromForm(ctx);
    try {
      const out = await tryLive('/api/search', async () => normalizeSearch(
        await raw('/api/search', { method: 'POST', body: official, timeout: 90000, as: 'json' }),
        { demo: false }
      ));
      demoMock = null;
      return out;
    } catch {
      demoMock = createMock(ctx);
      return demoMock.search();
    }
  }

  async function agent(ctx) {
    const brief = briefFromForm(ctx);
    const request = { text: brief.text, area: brief.area, date: brief.date, tz: brief.tz };
    try {
      return await tryLive('/api/agent', () => raw('/api/agent', { method: 'POST', body: request, timeout: 90000, as: 'json' }));
    } catch {
      return mockFor(ctx).agent();
    }
  }

  async function sun(ctx) {
    const params = { lat: ctx.lat, lon: ctx.lon, date: ctx.date, tz: ctx.tz, step_min: 10 };
    try {
      return await tryLive('/api/sun', async () => normalizeSun(await raw('/api/sun', { params, as: 'json' })));
    } catch {
      return { ...normalizeSun(mockFor(ctx).sun(params)), demo: true };
    }
  }

  async function horizon(ctx) {
    try {
      return await tryLive('/api/horizon', async () => normalizeHorizon(await raw('/api/horizon', {
        params: { lat: ctx.lat, lon: ctx.lon, above_m: 1.7, radius_m: 1500, step_deg: 1, date: ctx.date, tz: ctx.tz },
        timeout: 90000, as: 'json',
      })));
    } catch {
      return { ...normalizeHorizon(mockFor(ctx).hz({ lat: ctx.lat, lon: ctx.lon, date: ctx.date })), demo: true };
    }
  }

  async function scene(ctx) {
    const params = { lat: ctx.lat, lon: ctx.lon, radius_m: Math.max(100, ctx.radius_m || 200), detail: ctx.desktop ? 'desktop' : 'phone' };
    try {
      return await tryLive('/api/scene', async () => {
        const meta = classifyScene(await raw('/api/scene', { params, timeout: 90000, as: 'json' }));
        if (meta?.kind !== 'rings' || !meta.data_url) throw new Error('Invalid SceneMeta');
        const ring = pickSceneRing(meta, { desktop: ctx.desktop });
        const url = new URL(meta.data_url, base() + '/');
        if (url.origin !== new URL(base()).origin) throw new Error('Unexpected scene binary origin');
        const response = await fetch(url);
        if (!response.ok) throw new Error(`Scene binary HTTP ${response.status}`);
        const bytes = new Uint8Array(await response.arrayBuffer());
        return { ...decodeSceneRing(bytes, ring, meta), demo: false };
      });
    } catch {
      const { meta, bytes } = mockFor(ctx).sceneBin(params);
      const ring = pickSceneRing(classifyScene(meta), { desktop: ctx.desktop });
      return { ...decodeSceneRing(bytes, ring, meta), demo: true };
    }
  }

  async function photo(ctx) {
    const params = { lat: ctx.lat, lon: ctx.lon, kind: 'oblique', look_az_deg: Math.round(ctx.az_deg), radius_m: 150, max_px: 1024 };
    try {
      return await tryLive('/api/photo', () => loadPhoto(params));
    } catch {
      return { url: mockFor(ctx).photoDataUrl(ctx.az_deg), attribution: 'DEMO illustration — not aerial photography', demo: true };
    }
  }

  async function loadPhoto(params) {
    const qs = '?' + new URLSearchParams(cleanParams(params));
    const res = await fetch(base() + '/api/photo' + qs);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const ct = res.headers.get('content-type') || '';
    if (ct.includes('application/json') || ct.includes('text/json')) {
      const meta = await res.json();
      if (!meta.image_url) throw new Error('PhotoMeta missing image_url');
      const url = new URL(meta.image_url, base() + '/').href;
      return {
        url,
        attribution: meta.attribution || 'Aerial imagery',
        captured: meta.captured || null,
        demo: false,
      };
    }
    throw new Error('Expected PhotoMeta JSON');
  }

  async function buildings(ctx) {
    try {
      return await tryLive('/api/buildings', async () => {
        const data = await raw('/api/buildings', {
          params: { lat: ctx.lat, lon: ctx.lon, radius_m: ctx.radius_m || 150, limit: 20 },
          timeout: 20000,
          as: 'json',
        });
        return normalizeBuildings(data, { demo: false });
      });
    } catch {
      return normalizeBuildings(mockFor(ctx).buildings(), { demo: true });
    }
  }

  async function health() {
    try {
      const t = await raw('/api/health', { timeout: 2500 });
      live['/api/health'] = true;
      return String(t).toLowerCase().includes('ok') || t === 'ok' || true;
    } catch {
      live['/api/health'] = false;
      return false;
    }
  }

  function status() {
    return { ...live };
  }

  /** Last failure per endpoint: 'unreachable' or 'HTTP 4xx: msg' (a 4xx usually means a contract mismatch). */
  function lastErrors() {
    return { ...errors };
  }

  return { search, agent, sun, horizon, scene, photo, buildings, health, status, errors: lastErrors, base };
}

function cleanParams(obj) {
  const o = {};
  for (const [k, v] of Object.entries(obj)) {
    if (v != null && v !== '') o[k] = String(v);
  }
  return o;
}

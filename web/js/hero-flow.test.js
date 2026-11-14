import test from 'node:test';
import assert from 'node:assert/strict';
import { createMock } from './mock.js';
import { heatFeatures } from './map.js';
import { normalizeSun, normalizeHorizon, normalizeBuildings } from './adapters.js';

const ctx = { lat: 34.049, lon: -118.251, tz: 'America/Los_Angeles', date: '2026-06-24', text: 'rooftop sunset between towers' };

test('judge demo search returns ranked spots and populated heatmap', () => {
  const mock = createMock(ctx);
  const res = mock.search();
  assert.ok(res.demo);
  assert.ok(res.spots.length >= 2);
  assert.ok(res.spots.every(s => s.demo && Number.isFinite(s.score) && s.sun_visible_until));
  assert.ok(res.spots.every((s, i, a) => i === 0 || a[i - 1].score >= s.score));
  assert.equal(res.heat.values.length, res.heat.width * res.heat.height);
  assert.ok(heatFeatures(res.heat).features.length > 10);
});

test('judge demo spot loads horizon, sun, photo and contacts in contract shapes', () => {
  const mock = createMock(ctx);
  const spot = mock.search().spots[0];
  const sun = normalizeSun(mock.sun({ lat: spot.lat, lon: spot.lon, date: ctx.date }));
  const hz = normalizeHorizon(mock.hz({ lat: spot.lat, lon: spot.lon }));
  assert.ok(sun.points.length > 5);
  assert.ok(hz.points.length > 5);
  assert.equal(typeof mock.photoDataUrl, 'function');
  const buildings = normalizeBuildings(mock.buildings(), { demo: true });
  assert.ok(Array.isArray(buildings));
});

test('heatmap ignores nodata and out-of-range weights', () => {
  const heat = { center_lat: 34, center_lon: -118, cell_m: 100, width: 2, height: 2, values: [null, 0, 0.5, 2] };
  const f = heatFeatures(heat);
  assert.equal(f.features.length, 2);
  assert.equal(f.features[1].properties.weight, 1);
});

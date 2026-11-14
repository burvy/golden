const OSM_DARK = 'https://tiles.openfreemap.org/styles/dark';
const ESRI = 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}';

export function createMap({ host, getBase, onSelect, onMove }) {
  let map = null;
  let ready = false;
  let spots = [];
  let selectedId = null;
  let sunAz = null;
  let fallback = null;

  function init(center) {
    if (map || fallback) return;
    if (!window.maplibregl) {
      fallback = canvasFallback(host, { onSelect, onMove });
      fallback.setView(center.lat, center.lon, 14);
      ready = true;
      return;
    }
    map = new maplibregl.Map({
      container: host,
      style: OSM_DARK,
      center: [center.lon, center.lat],
      zoom: 14.2,
      pitch: 42,
      bearing: -18,
      attributionControl: true,
      hash: false,
    });
    map.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), 'top-right');
    map.addControl(new maplibregl.ScaleControl({ maxWidth: 100 }), 'bottom-right');
    map.on('load', () => {
      addLayers(map, getBase);
      ready = true;
      render();
    });
    map.on('moveend', () => {
      const c = map.getCenter();
      onMove?.({ lat: c.lat, lon: c.lng, zoom: map.getZoom(), radius_m: radiusFromZoom(map) });
    });
    map.on('click', 'spots-unclustered', (e) => {
      const f = e.features?.[0];
      if (f) onSelect?.(f.properties.id);
    });
    map.on('click', 'spots-clusters', (e) => {
      const f = e.features?.[0];
      const clusterId = f.properties.cluster_id;
      map.getSource('spots').getClusterExpansionZoom(clusterId, (err, zoom) => {
        if (err) return;
        map.easeTo({ center: f.geometry.coordinates, zoom });
      });
    });
    map.on('mouseenter', 'spots-unclustered', () => { map.getCanvas().style.cursor = 'pointer'; });
    map.on('mouseleave', 'spots-unclustered', () => { map.getCanvas().style.cursor = ''; });
  }

  function setView(lat, lon, zoom = 14.2) {
    if (map) map.easeTo({ center: [lon, lat], zoom, duration: 900 });
    else fallback?.setView(lat, lon, zoom);
  }

  function setSpots(next, selId, az) {
    spots = next || [];
    selectedId = selId;
    sunAz = az;
    render();
  }

  function render() {
    if (fallback) {
      fallback.setSpots(spots, selectedId, sunAz);
      return;
    }
    if (!map || !ready) return;
    const fc = {
      type: 'FeatureCollection',
      features: spots.map((s) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [s.lon, s.lat] },
        properties: { id: s.id, score: s.score, demo: !!s.demo },
      })),
    };
    const src = map.getSource('spots');
    if (src) src.setData(fc);
    const sel = spots.find((s) => s.id === selectedId);
    const selFc = {
      type: 'FeatureCollection',
      features: sel ? [{ type: 'Feature', geometry: { type: 'Point', coordinates: [sel.lon, sel.lat] }, properties: {} }] : [],
    };
    map.getSource('selected')?.setData(selFc);
    const line = sunLine(sel, sunAz);
    map.getSource('sunline')?.setData(line);
    if (sel) {
      map.easeTo({ center: [sel.lon, sel.lat], duration: 700 });
    }
  }

  function resize() {
    map?.resize();
  }

  function destroy() {
    map?.remove();
    map = null;
  }

  function current() {
    if (map) {
      const c = map.getCenter();
      return { lat: c.lat, lon: c.lng, zoom: map.getZoom(), radius_m: radiusFromZoom(map) };
    }
    return fallback?.current() || { lat: 0, lon: 0, zoom: 14, radius_m: 400 };
  }

  return { init, setView, setSpots, destroy, current, resize };
}

function addLayers(map, getBase) {
  map.addSource('esri', {
    type: 'raster',
    tiles: [ESRI],
    tileSize: 256,
    attribution: 'Esri, Maxar, Earthstar Geographics',
  });
  map.addLayer({
    id: 'esri',
    type: 'raster',
    source: 'esri',
    paint: { 'raster-opacity': 0.55, 'raster-saturation': -0.15 },
  });
  const base = String(getBase() || '').replace(/\/$/, '');
  if (base) {
    map.addSource('vexcel', {
      type: 'raster',
      tiles: [`${base}/api/tiles/vexcel/{z}/{x}/{y}`],
      tileSize: 256,
      minzoom: 12,
      maxzoom: 21,
      attribution: 'Vexcel Imaging US, Inc.',
    });
    map.addLayer({
      id: 'vexcel',
      type: 'raster',
      source: 'vexcel',
      paint: { 'raster-opacity': 0.85 },
    });
  }
  map.addSource('spots', { type: 'geojson', data: emptyFc(), cluster: true, clusterRadius: 48, clusterMaxZoom: 16 });
  map.addSource('selected', { type: 'geojson', data: emptyFc() });
  map.addSource('sunline', { type: 'geojson', data: emptyFc() });
  map.addLayer({
    id: 'spots-clusters',
    type: 'circle',
    source: 'spots',
    filter: ['has', 'point_count'],
    paint: {
      'circle-color': '#e8a15a',
      'circle-radius': ['step', ['get', 'point_count'], 16, 6, 20, 15, 26],
      'circle-stroke-width': 2,
      'circle-stroke-color': '#1a120c',
    },
  });
  map.addLayer({
    id: 'spots-cluster-count',
    type: 'symbol',
    source: 'spots',
    filter: ['has', 'point_count'],
    layout: { 'text-field': ['get', 'point_count_abbreviated'], 'text-size': 12 },
    paint: { 'text-color': '#1a120c' },
  });
  map.addLayer({
    id: 'spots-unclustered',
    type: 'circle',
    source: 'spots',
    filter: ['!', ['has', 'point_count']],
    paint: {
      'circle-color': ['interpolate', ['linear'], ['get', 'score'], 0, '#5c4a3a', 1, '#ffb15a'],
      'circle-radius': ['interpolate', ['linear'], ['get', 'score'], 0, 6, 1, 11],
      'circle-stroke-width': 1.5,
      'circle-stroke-color': '#fff3dc',
    },
  });
  map.addLayer({
    id: 'sunline',
    type: 'line',
    source: 'sunline',
    paint: { 'line-color': '#ffc078', 'line-width': 2, 'line-dasharray': [2, 2], 'line-opacity': 0.85 },
  });
  map.addLayer({
    id: 'selected-ring',
    type: 'circle',
    source: 'selected',
    paint: { 'circle-radius': 16, 'circle-color': 'rgba(255, 177, 90, 0.15)', 'circle-stroke-width': 2, 'circle-stroke-color': '#ffe0b0' },
  });
}

function sunLine(sel, az) {
  if (!sel || az == null) return emptyFc();
  const rad = (az * Math.PI) / 180;
  const m = 0.008;
  return {
    type: 'FeatureCollection',
    features: [{
      type: 'Feature',
      geometry: {
        type: 'LineString',
        coordinates: [
          [sel.lon, sel.lat],
          [sel.lon + Math.sin(rad) * m, sel.lat + Math.cos(rad) * m],
        ],
      },
    }],
  };
}

function emptyFc() {
  return { type: 'FeatureCollection', features: [] };
}

function radiusFromZoom(map) {
  const b = map.getBounds();
  const lat = map.getCenter().lat;
  const dlat = (b.getNorth() - b.getSouth()) * 111320;
  const dlon = (b.getEast() - b.getWest()) * 111320 * Math.cos((lat * Math.PI) / 180);
  const m = Math.min(dlat, dlon) * 0.35;
  return Math.max(50, Math.min(800, Math.round(m)));
}

function canvasFallback(host, { onSelect, onMove }) {
  const cv = document.createElement('canvas');
  cv.setAttribute('role', 'img');
  cv.setAttribute('aria-label', 'Map of ranked spots');
  host.innerHTML = '';
  host.appendChild(cv);
  let lat = 0, lon = 0, spots = [], sel = null, az = null, z = 14;
  const draw = () => {
    const dpr = devicePixelRatio || 1;
    const W = host.clientWidth || 600, H = host.clientHeight || 360;
    cv.width = W * dpr; cv.height = H * dpr; cv.style.width = W + 'px'; cv.style.height = H + 'px';
    const cx = cv.getContext('2d');
    cx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cx.fillStyle = '#16141c';
    cx.fillRect(0, 0, W, H);
    const span = 0.04 / Math.pow(2, z - 14);
    const px = (la, lo) => [((lo - (lon - span)) / (span * 2)) * W, ((lat + span - la) / (span * 2)) * H];
    cx.strokeStyle = '#2a2430';
    for (let i = 0; i < 8; i++) {
      cx.beginPath(); cx.moveTo((i / 8) * W, 0); cx.lineTo((i / 8) * W, H); cx.stroke();
      cx.beginPath(); cx.moveTo(0, (i / 8) * H); cx.lineTo(W, (i / 8) * H); cx.stroke();
    }
    for (const s of spots) {
      const [x, y] = px(s.lat, s.lon);
      cx.fillStyle = `rgba(255,177,90,${0.3 + s.score * 0.6})`;
      cx.beginPath(); cx.arc(x, y, 6 + s.score * 8, 0, 7); cx.fill();
    }
    const sp = spots.find((s) => s.id === sel);
    if (sp) {
      const [x, y] = px(sp.lat, sp.lon);
      cx.strokeStyle = '#ffe0b0'; cx.lineWidth = 2; cx.beginPath(); cx.arc(x, y, 12, 0, 7); cx.stroke();
      if (az != null) {
        const r = (az * Math.PI) / 180;
        cx.strokeStyle = '#ffc078'; cx.setLineDash([6, 4]);
        cx.beginPath(); cx.moveTo(x, y); cx.lineTo(x + Math.sin(r) * 80, y - Math.cos(r) * 80); cx.stroke();
        cx.setLineDash([]);
      }
    }
    cx.fillStyle = '#c9b8a0'; cx.font = '12px sans-serif';
    cx.fillText('Basemap unavailable — schematic view of ranked spots', 12, H - 12);
  };
  cv.onclick = (e) => {
    const b = cv.getBoundingClientRect();
    const x = e.clientX - b.left, y = e.clientY - b.top;
    const span = 0.04 / Math.pow(2, z - 14);
    const W = host.clientWidth, H = host.clientHeight;
    let best = null, d = 1e9;
    for (const s of spots) {
      const X = ((s.lon - (lon - span)) / (span * 2)) * W;
      const Y = ((lat + span - s.lat) / (span * 2)) * H;
      const k = Math.hypot(X - x, Y - y);
      if (k < d) { d = k; best = s; }
    }
    if (best && d < 28) onSelect?.(best.id);
  };
  addEventListener('resize', draw);
  return {
    setView(la, lo, zoom) { lat = la; lon = lo; z = zoom; onMove?.({ lat, lon, zoom: z, radius_m: 400 }); draw(); },
    setSpots(s, id, a) { spots = s; sel = id; az = a; draw(); },
    current() { return { lat, lon, zoom: z, radius_m: 400 }; },
  };
}

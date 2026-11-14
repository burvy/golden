const RAD = Math.PI / 180;

export function interpSun(path, ms) {
  if (!path?.length) return { az: 270, el: 0 };
  if (ms <= Date.parse(path[0].t)) return { az: path[0].az_deg, el: path[0].elev_deg };
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1], b = path[i];
    const ta = Date.parse(a.t), tb = Date.parse(b.t);
    if (ms <= tb) {
      const f = tb === ta ? 0 : Math.max(0, Math.min(1, (ms - ta) / (tb - ta)));
      return { az: a.az_deg + (b.az_deg - a.az_deg) * f, el: a.elev_deg + (b.elev_deg - a.elev_deg) * f };
    }
  }
  const l = path[path.length - 1];
  return { az: l.az_deg, el: l.elev_deg };
}

export function horizonFn(hz) {
  const a = (hz || []).slice().sort((x, y) => x.az_deg - y.az_deg);
  if (!a.length) return () => 0;
  return (az) => {
    az = ((az % 360) + 360) % 360;
    const j = a.findIndex((p) => p.az_deg > az);
    const hi = j < 0 ? a[0] : a[j];
    const lo = j <= 0 ? a[a.length - 1] : a[j - 1];
    let la = lo.az_deg, ha = hi.az_deg;
    if (la > az) la -= 360;
    if (ha < az) ha += 360;
    return lo.elev_deg + (hi.elev_deg - lo.elev_deg) * (ha === la ? 0 : (az - la) / (ha - la));
  };
}

export function blockTime(path, hf, rise) {
  if (!path?.length) return null;
  const pk = path.reduce((m, p) => (p.elev_deg > m.elev_deg ? p : m));
  const pm = Date.parse(pk.t);
  const t0 = Date.parse(path[0].t);
  const t1 = Date.parse(path[path.length - 1].t);
  for (let ms = rise ? t0 : pm; ms <= (rise ? pm : t1); ms += 60000) {
    const p = interpSun(path, ms);
    if (rise ? p.el >= hf(p.az) : p.el < hf(p.az)) return { ms, ...p };
  }
  return null;
}

export function formatTime(iso, tz) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(+d)) return String(iso).slice(11, 16);
  try {
    return d.toLocaleTimeString('en-US', { timeZone: tz || undefined, hour: 'numeric', minute: '2-digit' });
  } catch {
    return d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  }
}

export function formatClock(ms, tz) {
  return formatTime(new Date(ms).toISOString(), tz);
}

export function sunVector(azDeg, elDeg, r = 1) {
  const az = azDeg * RAD, el = elDeg * RAD;
  return {
    x: Math.sin(az) * Math.cos(el) * r,
    y: Math.sin(el) * r,
    z: -Math.cos(az) * Math.cos(el) * r,
  };
}

export function rng(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

/** NOAA-style solar position; used for the hero and DEMO fallback. */
export function sunAt(dateStr, min, lat, lon, tzOffsetHours) {
  const [Y, M, D] = dateStr.split('-').map(Number);
  const n = Math.floor((Date.UTC(Y, M - 1, D) - Date.UTC(Y, 0, 0)) / 86400000);
  const g = (2 * Math.PI) / 365 * (n - 1);
  const eot = 229.18 * (0.000075 + 0.001868 * Math.cos(g) - 0.032077 * Math.sin(g) - 0.014615 * Math.cos(2 * g) - 0.040849 * Math.sin(2 * g));
  const dec = 0.006918 - 0.399912 * Math.cos(g) + 0.070257 * Math.sin(g) - 0.006758 * Math.cos(2 * g) + 0.000907 * Math.sin(2 * g) - 0.002697 * Math.cos(3 * g) + 0.00148 * Math.sin(3 * g);
  const H = ((min + eot + 4 * lon - 60 * tzOffsetHours) / 4 - 180) * RAD;
  const p = lat * RAD;
  const el = Math.asin(Math.sin(p) * Math.sin(dec) + Math.cos(p) * Math.cos(dec) * Math.cos(H));
  const az = ((Math.atan2(Math.sin(H), Math.cos(H) * Math.sin(p) - Math.tan(dec) * Math.cos(p)) / RAD) + 540) % 360;
  return { az, el: el / RAD };
}

export function offsetISO(date, min, tzOffsetHours) {
  const sign = tzOffsetHours >= 0 ? '+' : '-';
  const abs = Math.abs(tzOffsetHours);
  const hh = String(Math.floor(abs)).padStart(2, '0');
  const mm = String(Math.round((abs % 1) * 60)).padStart(2, '0');
  const h = String(Math.floor(min / 60)).padStart(2, '0');
  const m = String(min % 60).padStart(2, '0');
  return `${date}T${h}:${m}:00${sign}${hh}:${mm}`;
}

export function tzOffsetHours(tz, date = new Date()) {
  try {
    const s = new Intl.DateTimeFormat('en-US', { timeZone: tz, timeZoneName: 'shortOffset' }).format(date);
    const m = s.match(/GMT([+-]?\d+)(?::(\d+))?/);
    if (!m) return -new Date().getTimezoneOffset() / 60;
    return +m[1] + (m[2] ? +m[2] / 60 : 0);
  } catch {
    return -new Date().getTimezoneOffset() / 60;
  }
}

export function localDateISO(tz = Intl.DateTimeFormat().resolvedOptions().timeZone) {
  try {
    return new Intl.DateTimeFormat('en-CA', { timeZone: tz, year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date());
  } catch {
    return new Date().toISOString().slice(0, 10);
  }
}

export function metersPerDegree(lat) {
  return { lat: 111320, lon: 111320 * Math.max(0.2, Math.cos(lat * RAD)) };
}

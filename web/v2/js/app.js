// Frontend v2 mock (task 057): the chat steers, goals are live filters, the funnel and map show their effect.
// Data comes from mock.js in the shape of the v2 contract; swap mockChat for POST /api/chat when the server speaks v2.
import { LIBRARY, OBJECTS, featFor, whyFor, mockChat, INVESTIGATIONS, CONTACTS, CACHED_VIEWS } from './mock.js';
import { resolveGoal, runFunnel } from './funnel.js';
import { createMap, RAMP_CSS } from './map.js';

const $ = s => document.querySelector(s);
const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fmt = n => n.toLocaleString('en-US');
const params = new URLSearchParams(location.search);
// API base for tiles, photos and the 3D mesh: ?api=/golden (burvy.dev), else this origin when golden's server serves the page
const BASE = (() => { const a = params.get('api'); if (a && /^\/(?!\/)/.test(a)) return location.origin + a.replace(/\/$/, '');
  return location.protocol.startsWith('http') ? location.origin : 'http://localhost:8787'; })();
const COLORS = ['--g1', '--g2', '--g3', '--g4', '--g5', '--g6'].map(v => getComputedStyle(document.documentElement).getPropertyValue(v).trim());
const ICON = {
  eye: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>',
  x: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 6l12 12M18 6L6 18"/></svg>',
};

const S = { messages: [], busy: false, goals: [], run: null, spots: [], sel: 0, layer: 'all', checked: new Map(), color: {}, brief: null };
// MOCK: the AI's note for a spot it checked; the real one comes back from the investigate tool
function check(i) { if (S.checked.has(i)) return; const c = S.candidates[i], n = S.checked.size;
  let note = INVESTIGATIONS[n % INVESTIGATIONS.length]; if (c.roof && note.verdict === 'caution') note = INVESTIGATIONS[(n + 1) % INVESTIGATIONS.length];
  S.checked.set(i, note); }

// ---------------------------------------------------------------- chat (POST /api/chat shape)
const GREET = 'What do you want to shoot? Tell me what should be in the frame, the light and the day if you know them. I’ll ask about anything that changes where you stand.';
function bubble(role, html) { const d = document.createElement('div'); d.className = `b ${role}`; d.innerHTML = html; $('#log').append(d); $('#log').scrollTop = 1e9; return d; }
function pickChip(t) { return / \(my pick\)$/.test(t) ? `<span class="chip pick" title="My pick: say if you want something else">${esc(t.replace(/ \(my pick\)$/, ''))}, my pick</span>` : `<span class="chip">${esc(t)}</span>`; }
function showUnderstood(list) {
  $('#hbrief').innerHTML = list.map(t => `<span>${esc(t.replace(/ \(my pick\)$/, ''))}</span>`).join('');
}
function choices(list) {
  $('#choices').innerHTML = list.map((t, i) => `<button type="button" class="${i ? '' : 'def'}">${esc(t)}</button>`).join('');
  $('#choices').querySelectorAll('button').forEach((b, i) => (b.onclick = () => send(list[i])));
}
async function send(text, go = false, instant = false) {
  text = (text || '').trim(); if (S.busy || (!text && !go)) return;
  if (S.brief && text) newBrief();
  if (text) { S.messages.push({ role: 'user', text }); bubble('user', esc(text)); }
  $('#msg').value = ''; choices([]); S.busy = true;
  const wait = bubble('agent', '<span class="dots" aria-label="Thinking"><i></i><i></i><i></i></span>');
  try {
    const r = await mockChat({ messages: S.messages, area: null, date: null, tz: 'America/Los_Angeles', go, instant });
    wait.remove(); showUnderstood(r.understood);
    if (r.ask) {
      S.messages.push({ role: 'agent', text: r.ask.question });
      bubble('agent', esc(r.ask.question)); choices(r.ask.choices);
      $('#known').innerHTML = r.understood.map(pickChip).join(''); $('#gonow').hidden = false;
    } else {
      $('#known').innerHTML = ''; $('#gonow').hidden = true;
      const steps = r.sheet.steps.map(s => `<li><b>${esc(s.kind.replace(/_/g, ' '))}</b>: ${esc(s.text)}</li>`).join('');
      const log = document.createElement('ol'); log.className = 'steps'; log.innerHTML = steps; $('#log').append(log);
      applyBrief(r.brief, r.sheet);
      if (matchMedia('(max-width:980px)').matches && !instant) $('#funnel').scrollIntoView({ behavior: 'smooth', block: 'start' });
      const must = S.goals.filter(g => g.must).length;
      bubble('agent', `I set ${S.goals.length} goals: ${must} that every spot must meet, the rest to rank by. ${S.spots.length} spots made it, and I looked at each one in an aerial photo. Change the goals on the right if I got something wrong.`);
    }
  } catch (e) { wait.remove(); bubble('agent', `That didn’t work: ${esc(e.message)}. Try again, or press “Search now”.`); }
  S.busy = false;
}
function newBrief() { S.messages = []; S.brief = null; $('#log').innerHTML = ''; bubble('agent', esc(GREET)); }

// ---------------------------------------------------------------- goals -> funnel -> spots
function applyBrief(brief, sheet) {
  S.brief = brief; S.candidates = sheet.candidates;
  S.goals = brief.goals.map((g, i) => resolveGoal(g, LIBRARY, i));
  S.goals.forEach((R, i) => (S.color[R.key] ??= COLORS[i % COLORS.length]));
  rerun({ investigate: true });
}
let tmr = 0;
function rerun({ investigate = false, soon = false } = {}) {
  if (soon) { clearTimeout(tmr); tmr = setTimeout(() => rerun(), 120); return; }
  const prev = S.spots[S.sel]?.i;
  S.run = runFunnel(S.goals.filter(g => !g.removed), S.candidates, featFor);
  if (investigate) S.run.picks.forEach(check);
  S.spots = S.run.picks.map((i, n) => spotFor(i, n));
  const keep = S.spots.findIndex(s => s.i === prev); S.sel = keep >= 0 ? keep : 0;
  if (S.layer !== 'all' && !S.goals.some(g => g.key === S.layer && !g.removed)) S.layer = 'all';
  renderGoals(); renderFunnel(); renderLayers(); renderSpot(); drawMap();
  $('#mapempty').hidden = true;
  $('#addgoal').hidden = false;
}
function spotFor(i, n) {
  const c = S.candidates[i], live = S.goals.filter(g => !g.removed);
  return { i, c, rank: n + 1, score: S.run.total[i],
    scores: live.map(R => ({ goal: R.key, score: S.run.layers[R.key][i], why: whyFor(R, c) })),
    investigation: S.checked.get(i) || null,
    contact: CONTACTS[(c.r + c.k) % CONTACTS.length] };
}
const objName = ref => { if (!ref) return ''; const o = ref.id ? OBJECTS.find(x => x.id === ref.id) : OBJECTS.find(x => x.kind === ref.kind); return o ? o.name : ref.kind; };
function title(R) {
  const a = R.args;
  switch (R.fn.name) {
    case 'in_front': return `${objName(a.a)} in front of ${objName(a.b)}`;
    case 'lined_up': return `${objName(a.a)} lined up with ${objName(a.b)}`;
    case 'fills': return `${objName(a.a)} fills the frame`;
    case 'close_to_tower': return `Close to ${objName(a.b)}`;
    case 'sun_between': return 'Sun in a gap';
    case 'sun_behind': return 'Sun behind a building';
    case 'sun_clear': return 'Clear view of the sun';
    case 'high_vantage': return 'Up high';
    case 'street_level': return 'Street level only';
    default: return R.fn.name.replace(/_/g, ' ').replace(/^./, c => c.toUpperCase());
  }
}
const TIER_WORD = { Point: 'distance and bearing, instant', Ray: 'line of sight through the height map', Skyline: 'the skyline toward the sun', AI: 'needs the AI' };

function renderGoals() {
  const live = S.goals.filter(g => !g.removed);
  const stageOf = key => S.run.stages.findIndex(s => s.key === key);
  const row = R => {
    const st = stageOf(R.key), before = st > 0 ? S.run.stages[st - 1].n : 0, after = st > 0 ? S.run.stages[st].n : 0;
    return `<li class="goal ${R.must ? 'must' : ''}" style="--c:${S.color[R.key]}" data-k="${esc(R.key)}">
      <div class="ghead"><h4>${esc(title(R))}</h4>
        <button class="ib" data-a="layer" aria-pressed="${S.layer === R.key}" aria-label="Show this goal on the map" title="Show on the map">${ICON.eye}</button>
        <button class="ib" data-a="rm" aria-label="Remove this goal" title="Remove">${ICON.x}</button></div>
      <p>${esc(R.fn.about)}</p>
      <div class="ctl">
        <label class="sw"><input type="checkbox" data-a="must" ${R.must ? 'checked' : ''}> Must</label>
        ${R.must ? '' : `<label class="wt"><span class="sr">Weight</span><input type="range" min="0" max="2" step="0.1" value="${R.weight}" data-a="w" aria-label="Weight for ${esc(title(R))}"><output>${R.weight.toFixed(1)}</output></label>`}
      </div>
      <div class="gmeta">${R.must ? `<span>Dropped ${fmt(before - after)} of ${fmt(before)} points</span>` : `<span>Checks ${TIER_WORD[R.tier]}</span>`}
        ${R.saved ? '' : `<span class="new">New, written for this brief</span><button data-a="save">Save to library</button>`}</div>
      <details><summary>How it’s scored</summary><pre>${esc(JSON.stringify(R.saved ? { use: R.fn.name, args: R.goal.args, ...(R.must ? { must: true } : { weight: R.weight }) } : R.fn, null, 1))}</pre></details>
    </li>`;
  };
  const musts = live.filter(R => R.must), soft = live.filter(R => !R.must);
  $('#glists').innerHTML = `
    <div class="group"><h3>Every spot must <small>${musts.length ? 'checked cheapest first' : ''}</small></h3>
      ${musts.length ? `<ul class="glist">${musts.map(row).join('')}</ul>` : '<div class="gempty">No musts: every point stays in, ranked by the goals below.</div>'}</div>
    <div class="group"><h3>Then rank by <small>${soft.length ? 'heavier counts more' : ''}</small></h3>
      ${soft.length ? `<ul class="glist">${soft.map(row).join('')}</ul>` : '<div class="gempty">Nothing to rank by, so the musts’ own scores decide.</div>'}</div>`;
  $('#glists').querySelectorAll('.goal').forEach(li => {
    const R = S.goals.find(g => g.key === li.dataset.k);
    li.querySelector('[data-a=must]').onchange = e => { R.must = e.target.checked; flash(); rerun(); };
    const w = li.querySelector('[data-a=w]');
    if (w) w.oninput = e => { R.weight = +e.target.value; li.querySelector('output').textContent = R.weight.toFixed(1); flash(); rerun({ soon: true }); };
    li.querySelector('[data-a=rm]').onclick = () => { R.removed = true; flash(); rerun(); };
    li.querySelector('[data-a=layer]').onclick = () => { S.layer = S.layer === R.key ? 'all' : R.key; renderGoals(); renderLayers(); drawMap(); };
    const sv = li.querySelector('[data-a=save]');
    if (sv) sv.onclick = () => { LIBRARY.push({ ...R.fn, saved: 'just now, from this brief' }); R.saved = true; renderGoals(); renderLib(); };
  });
}

// ---------------------------------------------------------------- the funnel: a band of light, narrowing
function renderFunnel() {
  const st = S.run.stages, n0 = st[0].n || 1, N = st.length, el = $('#funnel');
  const W = 1000, H = 84, w = W / N, h = n => 22 + 58 * Math.sqrt(n / n0);
  const pts = st.map((s, i) => [i * w + w / 2, h(s.n)]);
  const top = [[0, (H - pts[0][1]) / 2], ...pts.map(([x, hh]) => [x, (H - hh) / 2]), [W, (H - pts[N - 1][1]) / 2]];
  let d = `M${top[0][0]},${top[0][1]}`;
  for (let i = 1; i < top.length; i++) { const [x0, y0] = top[i - 1], [x1, y1] = top[i], mx = (x0 + x1) / 2; d += ` C${mx},${y0} ${mx},${y1} ${x1},${y1}`; }
  const bot = top.map(([x, y]) => [x, H - y]).reverse();
  d += ` L${bot[0][0]},${bot[0][1]}`;
  for (let i = 1; i < bot.length; i++) { const [x0, y0] = bot[i - 1], [x1, y1] = bot[i], mx = (x0 + x1) / 2; d += ` C${mx},${y0} ${mx},${y1} ${x1},${y1}`; }
  const lastChecked = S.run.picks.filter(i => S.checked.has(i)).length;
  el.style.setProperty('--n', N);
  el.innerHTML = `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" aria-hidden="true">
      <defs><linearGradient id="fg" x1="0" x2="1"><stop offset="0" stop-color="#d3e8f8"/><stop offset=".45" stop-color="#fbd7a6"/><stop offset=".8" stop-color="#f9a03f"/><stop offset="1" stop-color="#f26b1d"/></linearGradient></defs>
      <path d="${d} Z" fill="url(#fg)"/></svg>
    <div class="fsteps">${st.map((s, i) => `<div class="fs ${i === N - 1 ? 'last' : ''}"><span class="num">${s.ai ? `Top ${s.n}` : fmt(s.n)}</span></div>`).join('')}</div>
    <div class="fcap">${st.map((s, i) => {
      if (i === 0) return `<div>Points I can stand on, ${S.brief.area}</div>`;
      if (s.ai) return `<div>${lastChecked === s.n ? 'Checked by the AI in aerial photos' : `${lastChecked} of ${s.n} checked by the AI`}${lastChecked < s.n ? '<br><button class="check" id="checknew">Check the new ones</button>' : ''}</div>`;
      const R = S.goals.find(g => g.key === s.key);
      return `<div><b style="--c:${S.color[s.key]}"></b>${esc(title(R))}</div>`;
    }).join('')}</div>`;
  const ck = $('#checknew'); if (ck) ck.onclick = investigateNew;
}
function flash() { const f = $('#funnel'); f.classList.remove('flash'); void f.offsetWidth; f.classList.add('flash'); }
async function investigateNew() {
  const b = $('#checknew'); b.disabled = true; b.textContent = 'Looking at the photos…';
  await new Promise(r => setTimeout(r, 1200));
  S.run.picks.forEach(check); S.spots = S.run.picks.map((i, n) => spotFor(i, n)); renderFunnel(); renderSpot();
}

// ---------------------------------------------------------------- map layers
const map = createMap($('#map'), { base: BASE, onPick: n => { S.sel = n; renderSpot(); drawMap(); } });
function renderLayers() {
  const live = S.goals.filter(g => !g.removed);
  $('#layers').innerHTML = [`<button data-l="all" aria-pressed="${S.layer === 'all'}"><i style="--c:conic-gradient(#f9a03f,#f26b1d,#d6480c,#f9a03f)"></i>All goals</button>`,
    ...live.map(R => `<button data-l="${esc(R.key)}" aria-pressed="${S.layer === R.key}"><i style="--c:${S.color[R.key]}"></i>${esc(title(R))}</button>`)].join('');
  $('#layers').querySelectorAll('button').forEach(b => (b.onclick = () => { S.layer = b.dataset.l; renderLayers(); renderGoals(); drawMap(); }));
  const L = $('#legend'); L.hidden = false;
  const R = live.find(g => g.key === S.layer);
  L.innerHTML = R ? `<b>${esc(title(R))}</b><div class="ramp" style="background:linear-gradient(90deg,transparent,${S.color[R.key]})"></div><div class="ends"><span>poor</span><span>great</span></div>${R.must ? '<span class="mute">Shaded: dropped by this must</span>' : ''}`
    : `<b>How well each point fits</b><div class="ramp" style="background:${RAMP_CSS}"></div><div class="ends"><span>poor</span><span>best</span></div><span class="mute">Shaded: dropped by a must</span>`;
}
function drawMap() {
  if (!S.run) return map.draw();
  map.set({ candidates: S.candidates, run: S.run, layer: S.layer, color: k => S.color[k], picks: S.run.picks, sel: S.spots[S.sel]?.i ?? null });
  $('#attrib').textContent = map.attribution();
}

// ---------------------------------------------------------------- spot card
let mediaTab = 'photo';
function renderSpot() {
  const el = $('#spot'), sp = S.spots[S.sel];
  if (!sp) { el.hidden = false; el.innerHTML = '<div class="sp-head"><div class="sp-title"><h2>No spot passes every must</h2><p>Turn a must into a weighted goal, or remove one, to let points back in.</p></div></div>'; return; }
  el.hidden = false;
  const c = sp.c, inv = sp.investigation;
  const where = c.roof ? `Roof, eye ${c.eye_m} m up` : 'Street level';
  el.innerHTML = `
    <div class="sp-head">
      <div class="sp-rank" role="tablist" aria-label="Top spots">${S.spots.map((s, n) => `<button role="tab" aria-pressed="${n === S.sel}" aria-label="Spot ${n + 1}">${n + 1}</button>`).join('')}</div>
      <div class="sp-title"><h2>${where}</h2><p>${c.lat.toFixed(5)}, ${c.lon.toFixed(5)}. Arrive by 3:40 PM, sunset 4:48 PM.</p></div>
      <div class="sp-score"><span class="num">${Math.round(sp.score * 100)}</span><br><small>fit, of 100</small></div>
    </div>
    <div class="sp-body">
      <div class="sp-why">
        <ul class="reasons">${sp.scores.map(r => { const R = S.goals.find(g => g.key === r.goal); return `
          <li style="--c:${S.color[r.goal]}"><span class="gname"><i></i>${esc(title(R))}${R.must ? ' <em>must, passes</em>' : ` <em>weight ${R.weight.toFixed(1)}</em>`}</span>
          <span class="bar"><b style="width:${Math.round(r.score * 100)}%"></b></span><span class="v">${Math.round(r.score * 100)}</span>
          <span class="w">${esc(r.why)}</span></li>`; }).join('')}</ul>
        <div class="ai">${inv
          ? `<h3>What the AI saw in the photo <span class="verdict ${inv.verdict === 'caution' ? 'caution' : ''}">${esc(inv.verdict)}</span></h3><p>${esc(inv.note)}</p>`
          : `<h3>Not checked yet <span class="verdict none">new after your edit</span></h3><p>This spot moved into the top ${S.spots.length} after you changed a goal. The numbers say it fits; the AI hasn’t looked at its photo yet.</p><button type="button" id="checkone">Check it in the photo</button>`}</div>
      </div>
      <div class="sp-media">
        <div class="sp-tabs" role="tablist"><button role="tab" data-t="photo" aria-selected="${mediaTab === 'photo'}">Photo</button><button role="tab" data-t="3d" aria-selected="${mediaTab === '3d'}">3D view</button></div>
        <div class="view" id="view"></div>
        <ul class="calls"><li><span><b>${esc(sp.contact.who)}</b><br><span class="mute">${esc(sp.contact.org)}</span></span><a href="tel:${sp.contact.phone.replace(/[^\d]/g, '')}">${esc(sp.contact.phone)}</a></li>
          <li><span class="mute">Ask for roof or plaza access for a small crew, ${S.brief.date === '2026-11-15' ? 'Sat 15 Nov' : S.brief.date}, 3:30 to 5:15 PM.</span></li></ul>
      </div>
    </div>`;
  el.querySelectorAll('.sp-rank button').forEach((b, n) => (b.onclick = () => { S.sel = n; renderSpot(); drawMap(); }));
  el.querySelectorAll('.sp-tabs button').forEach(b => (b.onclick = () => { mediaTab = b.dataset.t; renderSpot(); }));
  const one = $('#checkone'); if (one) one.onclick = async () => { one.disabled = true; one.textContent = 'Looking…'; await new Promise(r => setTimeout(r, 900)); check(sp.i); S.spots[S.sel] = spotFor(sp.i, S.sel); renderFunnel(); renderSpot(); };
  media(sp);
}
let view3d = null;
async function media(sp) {
  const v = $('#view'), c = sp.c;
  if (view3d) { try { view3d.dispose?.(); } catch (_) {} view3d = null; }
  // MOCK: fake spots have no cached photo or mesh, so each borrows one of the hero spots' cached views
  const cam = CACHED_VIEWS[(sp.rank - 1) % CACHED_VIEWS.length], look = cam.az;
  if (mediaTab === 'photo') {
    v.innerHTML = '<div class="ph">Loading the aerial photo…</div>';
    try {
      const r = await fetch(`${BASE}/api/photo?lat=${cam.lat}&lon=${cam.lon}&look_az_deg=${look}&radius_m=150&max_px=1024`);
      if (!r.ok) throw 0; const m = await r.json();
      if (S.spots[S.sel] !== sp || mediaTab !== 'photo') return;
      v.innerHTML = `<img alt="Aerial photo looking toward the sunset from spot ${sp.rank}" src="${esc(BASE + m.image_url)}">`;
    } catch (_) { v.innerHTML = '<div class="ph">The oblique photo appears here (golden’s server, /api/photo). It isn’t reachable from this page right now.</div>'; }
  } else {
    v.innerHTML = '<div class="ph">Loading the 3D view…</div>';
    try {
      const at = await fetch(`${BASE}/api/mesh/at?lat=${cam.lat}&lon=${cam.lon}`).then(r => (r.ok ? r.json() : Promise.reject()));
      const m = await import('../../js/meshview.js');
      if (S.spots[S.sel] !== sp || mediaTab !== '3d') return;
      v.innerHTML = '';
      view3d = await m.create({ host: v, base: BASE, lat: cam.lat, lon: cam.lon, yaw: look, tilesetUrl: at.tileset_url, attribution: at.attribution, onStatus: () => {} });
    } catch (_) { v.innerHTML = '<div class="ph">The 3D view (the existing js/meshview.js) appears here when golden’s server is reachable.</div>'; }
  }
}

// ---------------------------------------------------------------- library drawer
function renderLib() {
  const q = $('#libq').value.trim().toLowerCase(), used = new Set(S.goals.filter(g => !g.removed).map(g => g.fn.name));
  const list = LIBRARY.filter(f => !q || (f.name + ' ' + f.about).toLowerCase().includes(q));
  $('#liblist').innerHTML = list.map(f => {
    const R = resolveGoal({ use: f.name }, LIBRARY, 0), objs = f.params.length ? `Needs ${f.params.length === 1 ? 'one thing' : 'two things'} from the brief` : 'Works on any brief';
    return `<li><b>${esc(f.name.replace(/_/g, ' '))}</b><p>${esc(f.about)}</p><small>${f.seed ? 'Built in' : `Saved ${esc(f.saved)}`}. ${objs}. Checks ${TIER_WORD[R.tier]}.</small>
      <button type="button" data-n="${esc(f.name)}" ${used.has(f.name) || !S.run ? 'disabled' : ''}>${used.has(f.name) ? 'In use' : 'Add'}</button></li>`;
  }).join('') || '<li><p class="mute">Nothing matches. Describe the goal in the chat and I’ll write a new one.</p></li>';
  $('#liblist').querySelectorAll('button[data-n]').forEach(b => (b.onclick = () => addGoal(b.dataset.n)));
}
function addGoal(name) {
  const f = LIBRARY.find(x => x.name === name);
  // fill object params from the brief: the fountain and the tower it mentioned
  const fill = { a: { kind: 'fountain', near: { id: 'costar:52227' }, within_m: 300 }, b: { id: 'costar:52227' } };
  const args = Object.fromEntries(f.params.map((p, i) => [p, f.params.length === 1 ? fill.b : fill[p] || fill[['a', 'b'][i]]]));
  const R = resolveGoal({ key: `${name}+${S.goals.length}`, use: name, args, weight: 0.6 }, LIBRARY, S.goals.length);
  S.goals.push(R); S.color[R.key] = COLORS[(S.goals.length - 1) % COLORS.length];
  $('#lib').classList.remove('open'); flash(); rerun(); renderLib();
}
$('#addgoal').onclick = () => { renderLib(); $('#lib').classList.add('open'); $('#libq').focus({ preventScroll: true }); };
$('#libclose').onclick = () => $('#lib').classList.remove('open');
$('#libq').oninput = renderLib;
document.addEventListener('keydown', e => { if (e.key === 'Escape') $('#lib').classList.remove('open'); });

// ---------------------------------------------------------------- start
$('#say').onsubmit = e => { e.preventDefault(); send($('#msg').value); };
$('#msg').addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send($('#msg').value); } });
$('#gonow').onclick = () => send('', true);
newBrief();
renderGoalsEmpty(); map.draw();
function renderGoalsEmpty() { $('#glists').innerHTML = '<div class="gempty">Once the brief is clear, the goals I chose show up here, and you can change any of them.</div>'; }
fetch(`${BASE}/api/health`, { signal: AbortSignal.timeout(2000) }).then(r => { if (!r.ok) throw 0; }).catch(() => map.noTiles());
// ?demo=chat|results|edited: jump to a state, for screenshots and for showing burvy
const demo = params.get('demo');
(async () => {
  if (!demo) return;
  await send('fountain in front of a tower at sunset', false, true);
  if (demo === 'chat') return;
  await send('Grand Tower, 48 floors', false, true); await send('Either is fine', false, true);
  if (demo === 'edited') { // you drop "Up high", then add "long sun window" from the library: new spots come in, not yet checked by the AI
    S.goals.find(g => g.key === 'high').removed = true; addGoal('long_sun_window');
    const n = S.spots.findIndex(s => !s.investigation); if (n >= 0) { S.sel = n; renderSpot(); drawMap(); } }
})();

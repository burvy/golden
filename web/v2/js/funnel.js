// The funnel (docs/metrics.md): candidates -> musts, cheapest first -> weighted score -> top K, spaced apart.
// Same steps as core's search, run in the browser on the candidates the server sent, so editing a goal
// (weight, must, remove, add) re-ranks at once without a new chat turn. Pure, no DOM.
import { evaluate, tierOf, tierRank } from './expr.js';

/** A goal ({use|def, args, weight|must}) -> what the page needs to run and show it. */
export function resolveGoal(g, library, i) {
  const fn = g.def || library.find(f => f.name === g.use);
  if (!fn) throw Error(`no cost function "${g.use}"`);
  const tier = [fn.must, fn.score].filter(Boolean).map(tierOf).sort((a, b) => tierRank(b) - tierRank(a))[0];
  return { key: g.key || `${fn.name}#${i}`, goal: g, fn, args: g.args || {}, must: !!g.must, weight: g.weight ?? 1, tier,
           saved: !g.def, hidden: !!g.hidden };
}

/** Run the goals over the candidates. `featFor(c)` returns that candidate's feature function. */
export function runFunnel(goals, candidates, featFor, { k = 5, spacingM = 70 } = {}) {
  const score = (c, R) => evaluate(R.fn.score, featFor(c), R.args);
  const gate = (c, R) => (R.fn.must ? evaluate(R.fn.must, featFor(c), R.args) > 0 : score(c, R) >= 0.5);
  // per-goal score of every candidate, for the map layers (cached per goal key)
  const layers = {};
  for (const R of goals) layers[R.key] = candidates.map(c => score(c, R));

  const stages = [{ label: 'Points on the height map', n: candidates.length }];
  let alive = candidates.map((_, i) => i);
  const droppedBy = new Array(candidates.length).fill(null);
  const musts = goals.filter(R => R.must).sort((a, b) => tierRank(a.tier) - tierRank(b.tier));
  for (const R of musts) {
    const keep = [];
    for (const i of alive) { if (gate(candidates[i], R)) keep.push(i); else droppedBy[i] = R.key; }
    alive = keep; // an empty result is shown as 0, never hidden
    stages.push({ label: R.fn.about, n: alive.length, key: R.key });
  }
  // the cost function: weighted mean of the soft goals (musts rank only when there is nothing else)
  const soft = goals.filter(R => !R.must && R.weight > 0);
  const ranking = soft.length ? soft : musts;
  const wsum = ranking.reduce((s, R) => s + (R.must ? 1 : R.weight), 0) || 1;
  const total = new Array(candidates.length).fill(0);
  for (const i of alive) total[i] = ranking.reduce((s, R) => s + (R.must ? 1 : R.weight) * layers[R.key][i], 0) / wsum;

  const order = alive.slice().sort((a, b) => total[b] - total[a]);
  const picks = [];
  for (const i of order) {
    if (picks.length >= k) break;
    const c = candidates[i];
    if (picks.every(j => Math.hypot(candidates[j].x - c.x, candidates[j].y - c.y) >= spacingM)) picks.push(i);
  }
  stages.push({ label: 'Investigated by the AI', n: picks.length, ai: true });
  return { stages, alive, droppedBy, total, layers, picks, ranked: ranking.map(R => R.key) };
}

# Metrics: cutting down the points, one class of shot at a time

Status: approved by burvy (task 048); v2 section added after judges' feedback. Being built on branches.

## The idea in one paragraph

A brief ("fountain in front of the tower at sunset") gives us many candidate standpoints. Each
**metric** is a pure function that rates one standpoint for one kind of shot (0..1 plus a one-line
reason). The brief becomes a list of **goals**, where each goal is a metric that is either a *must*
(a gate) or *weighted* (part of the score). Search runs the goals over the candidates, cheapest
first, and throws a point away as soon as it fails a must. The weighted sum of the soft goals is
the **cost function** that ranks whatever survives. Only the few best points go to the AI to
"investigate" (look at the photo, check what the numbers can't see). Each new metric we add
solves a whole class of shots without AI, so fewer AI calls are needed. If no metric fits the
brief, the AI does all the judging. If the AI can't do it either, it says what data it would need.

Sunsets are already solved this way (today's `score.rs`). They become the first metric family.
Objects in relation to each other are the second.

## The funnel

```
area -> candidates (grid, today's HeatLayout)          ~1000 points
     -> musts, cheapest metric first, short-circuit     prune
     -> cost function = sum(weight * score) on the rest rank
     -> pick top K, spaced apart (today's `pick`)       ~5-10 points
     -> AI investigates each (oblique photo + numbers)  K calls, confirm / re-rank / explain
```

- **No metric matches the brief**: the AI gets a coarse sample of the candidates plus what we know
  about them, and judges them directly.
- **Impossible**: the AI answers with what's missing ("needs water detection for reflections"),
  and that becomes the next metric to build.

Each metric declares its **cost tier**, so the funnel can order the work:

| Tier | Needs | Example |
|---|---|---|
| Point | the standpoint + object list | distance/bearing to an object, "both objects within 300 m" |
| Ray | a few line-of-sight marches through the height map | "the tower's top is visible" |
| Skyline | the skyline over a window of azimuths | today's sun metrics |
| AI | a model call | "does it look good?" |

## Metrics, families

### Sun (exists, gets moved into the framework without changing the maths)
`Sun { framing: Clear | Between | Behind }` = today's `score::look` + `score::score`. The hero
brief must return the same spots before and after (regression test).

### Relations between objects (new)
An **object** comes from any source, and core doesn't care which:
`Object { id, kind, lat, lon, base_m, top_m, radius_m, source }`. Sources include CoStar buildings
(we have them), Vexcel detection (if task 049 says our key has it), and OSM points. The server
fetches and caches them. Core gets them as plain data.

Pure building blocks, all from one standpoint:
- `bearing(obj)`, `distance(obj)`, `angular_size(obj)`.
- `visible_fraction(obj)`: march rays from the eye through the height map to a few points on the
  object (base, middle, top), reusing the horizon ray march. 0 = hidden, 1 = fully in view.

First relation kinds:
- `InFront { a, b }`: a and b within one frame (bearing difference < half the lens angle), a
  nearer than b, b still mostly visible.
- `LinedUp { a, b }`: bearings about equal (a stacks on b).
- `Between { a, left, right }`: a's bearing lies between the two others.
- `Beside { a, b, side }`: a left or right of b, both in frame.
- `Fills { a, min_deg, max_deg }`: a's angular size is in range, i.e. it isn't a speck or too big.

Object references in a goal: a specific object (`id`) or a kind (`"fountain"`), meaning "any
object of that kind". For kinds, the search first finds **pairs within range of each other**
(e.g. a fountain within 300 m of a tower), which already prunes a lot.

**Marching (candidate generation, much cheaper than the full grid):** for `InFront`/`LinedUp`,
the good standpoints lie on the line from b through a and beyond. March outward along that line
(plus a narrow cone for the lens angle), test the standpoints near it, and stop when a gets too
small or is hidden. This turns "test 1000 points" into "test ~50 per pair".

The sun is an object too: at a given time it has a bearing and an elevation. That means "sun
behind the tower" and "sun between two towers" can later be written as relations. For now they
stay as the Sun family.

## v2 (judges' feedback, 2026-11-14 ~22:00): the AI writes its own cost functions

Two changes on top of the funnel above:
1. **The brief box becomes a chat.** An agent asks short questions until it understands the shot
   ("which tower?", "sunrise or sunset?", "street level OK?"), then hands search a set of goals.
2. **The agent designs cost functions and keeps a library of them.** It doesn't write code. It
   writes small **expressions** in a JSON language over a fixed set of pure **features** that core
   computes. Core checks and runs them. Good ones get saved under a name and reused, so the library
   grows into a list of solved shot classes. That's the "eliminate whole classes of points" idea,
   made by the AI instead of by us.

Why expressions and not generated code: they're safe (no code execution), fast (evaluated in Rust
inside the funnel), checkable before use (unknown feature or wrong type = rejected with a reason the
agent can fix), and each one is plain data we can show, save, diff and test.

### Features (pure, computed in core per candidate, lazily, cached per candidate)
Each feature has a **tier** (Point < Ray < Skyline). An expression's tier is the highest tier among
its features, and the funnel runs cheap expressions first.

| Feature | Tier | Meaning |
|---|---|---|
| `ground_m`, `eye_m`, `roof` (0/1) | Point | the standpoint |
| `lens_deg` | Point | the brief's lens angle (built in 052) |
| `dist(obj)`, `bearing(obj)`, `bearing_diff(a, b)` | Point | geometry to objects (deg, m); `bearing_diff` is signed, > 0 = a right of b |
| `size_deg(obj)` | Point | angular height of an object |
| `in_frame(a, b)` | Point | 0..1: 1 within half the lens angle, 0 a whole lens angle apart (built in 052) |
| `visible(obj)` | Ray | 0..1, one ray march through the height map to 5 heights on the object's near side |
| `sun_clear`, `sun_between`, `sun_behind` | Skyline | today's sun scores, 0..1, maths unchanged |
| `sun_visible_min`, `sun_hidden_elev`, `gap_width_deg` | Skyline | today's `Look` numbers |
| `sun_bearing_diff(obj)` | Skyline | sun azimuth at sunset minus object bearing |

`obj` is an object reference: `{"id": "costar:123"}`, or `{"kind": "tree", "near": "<other ref>",
"within_m": 300}`, meaning the best-scoring object of that kind (which is how "pairs within range"
works).

How core resolves them (052, `core::objects::bind`): each goal turns its references into up to 64
**choices** of concrete objects, nearest the area's centre first, and at each candidate scores its
best choice that passes its gates (the spot's reason names the objects it used). `kind` matches
any object of that label within the area radius + 1500 m; `within_m` without `near` is measured from
the area's centre; `near` without `within_m` means 300 m. A `near` target written the same as one
of the goal's own references is that object (`a = {"kind":"tree","near":{"kind":"building"}}, b =
{"kind":"building"}` is a tree near b). Different references are always different objects. A
reference that finds nothing is a goal error that says what the area does have.

**Marching (052):** every feature comparing two objects (`bearing_diff(a, b)`, `in_frame(a, b)`) in a
goal adds candidates on the line from b through a and beyond (plus 2° either side), out to where a
looks 1° tall, inside the area, for the goal's first 8 choices, at most 800 per search. They rank
like grid candidates but aren't on the heatmap.

### Expression language (JSON, small on purpose)
```jsonc
{"f": "visible", "obj": {"kind": "fountain"}}        // feature
{"num": 0.5}                                         // number
{"op": "ramp", "x": E, "lo": 20, "hi": 5}            // 0 at lo, 1 at hi, linear, clamped (lo > hi allowed)
{"op": "bump", "x": E, "at": 0, "width": 10}         // 1 at `at`, 0 beyond ±width
{"op": "and" | "or" | "min" | "max" | "mul" | "add" | "avg", "of": [E, ...]}
{"op": "not", "x": E}  {"op": "lt" | "gt", "x": E, "than": E}   // 0/1
```
Every expression evaluates to a number. Scores are clamped to 0..1.

A **cost function** (library entry):
```jsonc
{ "name": "fountain_in_front_of_tower",
  "about": "Fountain in frame, nearer than the tower, both visible",
  "params": ["a", "b"],                    // object refs filled in per brief
  "must": E,                               // optional gate: 0 drops the point
  "score": E }                             // 0..1
```
Built-in entries (`sun_clear`, `sun_between`, `sun_behind`, `in_front`, `lined_up`, `between`,
`beside`, `fills`) ship as seed JSON and are written in the same language, so the agent can read
them as examples and copy them. Params are object refs only, so `beside` takes no `side` (it
accepts either; the agent can write a one-sided copy with the sign of `bearing_diff`) and `fills`
uses fixed angles (8-25° tall).

### Goals in the brief (contract)
```jsonc
"goals": [
  { "use": "sun_between", "weight": 1.0 },
  { "use": "fountain_in_front_of_tower", "args": {"a": {"kind":"fountain"}, "b": {"id":"costar:123"}}, "must": true },
  { "def": { ...inline cost function... }, "weight": 0.5 }       // a new one, not saved yet
],
"lens_deg": 60
```
No goals = `[{"use": "sun_<framing>"}]`, i.e. exactly today. Each Spot gets
`scores: [{goal, score, why}]`. `why` comes from the entry's `about` plus the numbers.

### Agent loop (server, Claude)
Tools: `list_cost_functions`, `define_cost_function` (validates, then test-runs the function on a
sample of ~50 candidates and returns the score distribution plus 3 example points, so the agent can
see whether it actually separates good points from bad), `save_cost_function`, `find_objects`
(CoStar + cached Vexcel detection), `search` (goals -> spots), `investigate` (top K: oblique photo
plus numbers, judged by the model), and `ask_user` (the chat).
Library: `library/*.json` in git (seeds, plus ones we promote) plus `cache/library/*.json` (saved at
runtime, gitignored).

### Code shape
```
crates/contracts   Goal, CostFn, Expr, ObjRef, Rated (serde)
crates/core        features.rs (pure features + tiers), expr.rs (check + eval), funnel in search.rs
crates/server      library store, object sources (CoStar, Vexcel detect, cached), agent tools, /api/chat
web                chat panel replaces the brief box; spot card lists per-goal reasons
```

### Build order (v2)
1. Contract PR: Goal / CostFn / Expr / ObjRef / Rated, plus old-brief compatibility.
2. core: expr.rs + features.rs (sun features from today's code) + funnel. Regression: the hero LA
   and Princeton briefs give identical spots.
3. core: object features (dist/bearing/size/visible) + seed relation entries + marching candidates.
4. server: library store + object sources + agent tools (define/test/save).
5. server + web: /api/chat (multi-turn, asks until confident) + chat panel.

## Decisions (burvy, 2026-11-14)
- Lens angle for "in one frame": 60° default (about 30 mm on full frame), and a brief can override it.
- Object kinds: a fixed list in contracts (whatever Vexcel detects that we've checked, plus "building"). Vexcel detection is
  open-vocabulary (task 049, docs/burvy/detect-probe.md), so a kind is just its label string, and adding one is one line.
- Weights: fixed per metric to start. The AI picking weights per brief comes later.
- Object positions from Vexcel: `POST /v2/elements/detect` returns a lon/lat box per object (centre = position), ~3 s per
  150 m box. Cache every response and only query boxes around the search area.

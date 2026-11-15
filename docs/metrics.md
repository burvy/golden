# Metrics: cutting down the points, one class of shot at a time

Status: DRAFT for burvy (task 048). Nothing here is built yet.

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

## Contract change (separate small PR, per AGENTS.md rule 2)

```jsonc
// Brief: new optional field; old briefs still work
"goals": [
  { "metric": { "kind": "sun", "framing": "between" }, "weight": 1.0 },
  { "metric": { "kind": "relation", "rel": "in_front",
                "a": { "kind": "fountain" }, "b": { "id": "costar:123" } }, "must": true }
]
// If "goals" is empty: goals = [ Sun(brief.framing) ], i.e. exactly today's behaviour.

// Spot: new field, one entry per goal, same order
"scores": [ { "score": 0.8, "why": "Sun sets in a 4° gap, 300 m away" }, ... ]
```
`Spot.sun` and `Spot.why` stay, so the web keeps working unchanged.

## Code shape (functional, core stays pure)

```
crates/core/src/objects.rs   Object, bearing/distance/size, visible_fraction (ray march)
crates/core/src/metrics.rs   Metric enum, Goal, tier(), score(metric, &View, &Ctx) -> Rated
crates/core/src/search.rs    candidates -> funnel(goals) -> pick; sun code moved, not changed
crates/server                object sources (CoStar now, Vexcel/OSM later), cached
crates/server/src/agent      Claude: text -> goals; investigate top K; AI-only fallback
```
`search(grid, objects, brief, utc_offset_s)`: objects come in as plain data, no IO.

## Build order (each on its own branch, main stays demo-safe)

1. 049 probe Vexcel object detection (classes, points or polygons, LA/Princeton).
2. Contract PR: `goals` + `scores`, fixture updated, old briefs unchanged.
3. core: metrics.rs + funnel, Sun ported; regression test: hero brief gives the same spots.
4. core: objects.rs + relation metrics + marching; unit tests on a synthetic block.
5. server: object source (CoStar first) + `/api/search` takes goals.
6. agent: Claude picks goals from the text; investigates top K; AI-only fallback; "what's missing".
7. web: show the per-goal reasons on the spot card.

Steps 1-3 can merge before judging only if main's sunset demo is unchanged. If they aren't ready,
the demo still shows sunsets solved, plus this doc as the roadmap.

## Open questions
- Lens angle for "in one frame": a fixed default (e.g. 60°, ~30 mm on full frame) or from the brief?
- Object kinds vocabulary: whatever Vexcel detects + CoStar "building". Fixed list or free text the
  AI maps onto it?
- Weights: from the AI per brief, or fixed per metric to start?

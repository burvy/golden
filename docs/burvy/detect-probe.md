# Vexcel object detection probe (task 049, 11/14)

**Answer: yes, our key has it, in both LA and Princeton.** One endpoint, open-vocabulary (you type the object name).

## Endpoint
`POST https://api.vexcelgroup.com/v2/elements/detect` (header `X-API-KEY`, JSON body). Found in the OpenAPI spec at
`GET /v2/api-docs` (840 KB, readable with our key; the Swagger UI needs a Viewer login, we don't have one).

Body (what worked):
```json
{"wkt":"POINT (-118.2550 34.0505)","bbox-dimensions":[150,150],
 "images":[{"endpoint":"oriented/extract","request":{"layer":"urban","wkt":"POINT (-118.2550 34.0505)","bbox-dimensions":"150,150"}}],
 "model-input":{"elements":[{"label":"tree","detection-threshold":0.5},{"label":"car","detection-threshold":0.5}],
                "include":["geometry","confidence","bbox"]}}
```
`wkt` is a point (+ `bbox-dimensions` m, or `buffer-meter`) or a polygon. `include` options: `geometry, confidence, bbox, pixel-bbox, pixel-mask`
(`pixel-mask` requires `pixel-bbox`, else 400).

## Classes
There is no fixed list. The model is SAM 3 (`model` default `sam3`): each `label` is a plain-English prompt, with its own
`detection-threshold` and `mask-threshold`. Tried: `tree`, `car`, `building`, `rooftop HVAC unit` (all returned hits),
`swimming pool`, `water tower` (valid, 0 hits in a downtown box, which is plausible). Vexcel's own marketing examples:
propane tanks, putting greens, telephone poles, boats, docks, bridges. So "classes" = anything we can name; quality varies
by prompt and is untested beyond a glance (counts only, no ground truth check).

## Point or polygon?
**Polygon, not point.** Each feature is a GeoJSON `Polygon` + `properties.label` + `properties.detections[{imageId, bbox, confidence, pixel-mask?}]`.
- `geometry` is a **5-point quad (a bounding box in lon/lat)**, not a tight outline (all 27 features in the mask run were 5 points).
  Centre of the quad = a usable object position.
- A real outline needs `pixel-mask` (base64 PNG, ~4 KB per object) plus `pixel-bbox`, i.e. you vectorise it yourself in image
  space and project with the camera metadata in `images[].metadata` (there is also `GET/POST /oriented/transform-points`).
- Detection runs on **imagery chosen by the request** (`images[]`, here the `urban` nadir frame, GSD ~6-8 cm). One call = one frame;
  the response lists that frame (`image-name`, `capture-date`, `collection`). A box can straddle frames: untested.

## Where it answers (one call each, 150 m box)
| Place | HTTP | Time | Result | Imagery used |
|---|---|---|---|---|
| Downtown LA 34.0505,-118.2550 | 200 | 2.7 s | 53 features: 29 tree, 24 car (40 KB) | `us-ca-losangeles-2025`, shot 2025-12-01 |
| Princeton 40.3487,-74.6593 | 200 | 3.6 s | 30 features: 30 tree, 0 car (23 KB) | `us-pa-philadelphia-2026`, shot 2026-07-25 (the Princeton-area frame came from the Philadelphia collection) |
| LA again: building, pool, water tower, HVAC | 200 | 3.3 s | 5 building, 22 HVAC (112 KB with masks) | same as LA |

## Size / cost
About 0.3-0.8 KB per feature without masks (23-40 KB per 150 m box for tens of objects); with `pixel-mask` ~4 KB per object (112 KB for 27).
2.7-3.6 s per call. Cache every response (AGENTS.md rule 7).

## Other things we tried
- `GET /v2/ogc-api/collections` (properties / structures / damage-assessment feature collections, i.e. the "Elements" building footprints and
  parcel attributes): **403 "Unauthorized by app permissions"**. So no ready-made footprint layer on our key; get buildings by prompting
  `building` here, or from CoStar / our height map.
- Public docs (Vexcel Atlassian wiki, "Custom Elements" launch page) are marketing-level; the OpenAPI spec was the real source.

## Caveats / not checked
- 8 key calls total for data/spec (spec, 3 detect, 1 ogc, plus 4 failed doc-URL guesses). No 429s. Key never printed or stored in the repo.
- Detection quality (recall, tree vs bush, car vs truck) is unverified. Tree count 29 in a downtown box, plausible but unchecked.
- Request limits (max box, rate limit, per-call billing) are unknown. Vexcel's detection product may be metered separately: ask Vexcel
  before building a feature that calls it per search.
- Whether a box spanning two frames merges or duplicates objects: untested.

Sample: `fixtures/vexcel_detect_sample.json` (trimmed LA response, mask truncated).

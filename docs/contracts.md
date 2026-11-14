# Contracts

The source of truth for every shared data shape and endpoint. Each shape has a sample in `fixtures/`.
Change a shape only in its own small PR, and update its fixture in the same PR (AGENTS.md rule 2).

## Conventions
- Angles in degrees. Azimuth clockwise from true north.
- Heights in metres, as Vexcel's DSM gives them: roughly **above sea level** (downtown LA ground ≈ 80–90 m).
  The exact vertical datum isn't stated by Vexcel; treat heights as comparable to each other, not to GPS altitude.
- Times in ISO 8601 with offset.
- Lat/lon WGS84 in every API. EPSG:3857 is only used inside `crates/vexcel`.

## Shapes

| Shape | Status | Owner | Fixture |
|---|---|---|---|
| `DsmGrid` | **defined** | vexcel | `fixtures/dsm_grid.json` + `fixtures/dsm_grid.f32` |
| `Brief` | TODO | server | |
| `HorizonProfile` | TODO (`[{az_deg, elev_deg}]` from an observer) | core | |
| `SunPath` | TODO (`[{t, az_deg, elev_deg}]`) | core | |
| `Spot` | TODO (lat, lon, height_m, score, sun_visible_until, why) | core | |
| `Building` | TODO | costar | |
| `CallSheet` | TODO | server | |

### DsmGrid

A north-up square of surface heights (roofs, trees, ground) around a centre point, in ground-metre cells.
Made by `vexcel::get_dsm(lat, lon, radius_m)`. Width = height = `ceil(2 * radius_m / cell_m)`.

On disk it is two files: a JSON meta and a raw little-endian `f32` file named by `data_file`.

```json
{
  "center_lat": 34.049,
  "center_lon": -118.251,
  "cell_m": 1.0,
  "width": 64,
  "height": 64,
  "nodata": -9999.0,
  "source": {
    "provider": "vexcel",
    "collection": "n34w118-us-ca-losangeles-2026",
    "captured_from": "Sun Apr 19 19:06:57 UTC 2026",
    "captured_to": "Wed Apr 29 16:44:04 UTC 2026",
    "native_res_m": 0.062
  },
  "data_file": "dsm_grid.f32"
}
```

| Field | Meaning |
|---|---|
| `center_lat`, `center_lon` | The point asked for. It sits at the centre of the grid (between cells when the size is even). |
| `cell_m` | Cell size in **ground metres** (default 1.0). |
| `width`, `height` | Cells per row, rows. |
| `nodata` | Value of a cell with no data (`-9999`). Always check for it. |
| `source.provider` | `"vexcel"` or `"synthetic"` (fixtures). |
| `source.collection`, `captured_from`, `captured_to` | Which Vexcel flight, and when. `null` for synthetic. |
| `source.native_res_m` | Finest source pixel used, ground metres, before downsampling (~0.062 in LA). |
| data | `width * height` `f32` heights in metres. **Row-major, row 0 = north edge, column 0 = west edge.** |

**Finding the cell for a lat/lon** (R = 6378137):
```
east_m  = (lon - center_lon) * pi/180 * R * cos(center_lat)
north_m = (lat - center_lat) * pi/180 * R
col = floor(width/2  + east_m  / cell_m)
row = floor(height/2 - north_m / cell_m)
index = row * width + col
```
Over the 1–3 km we use, this flat approximation is off by centimetres.

**Each cell holds the highest point inside it**, not the average, so a roof edge anywhere in the cell
counts as blocking the sun.

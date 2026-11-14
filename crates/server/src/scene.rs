//! Pure: cut a `DsmGrid` into the rings of a `SceneMeta` (docs/contracts.md#scenemeta--binary).
//! Ring k has cells of `4^k` metres, all rings the same number of cells, all centred on the spot; each cell
//! keeps the highest height under it so coarse rings never hide a tower.

use contracts::scene::{encode_dm, SceneRing, SCENE_ENCODING, SCENE_NODATA};
use contracts::{DsmGrid, SceneMeta};

/// Built scene: the meta (without `data_url`/`id` filled from the bytes yet) and the binary.
pub struct Built {
    pub meta: SceneMeta,
    pub bytes: Vec<u8>,
}

/// Rings around (lat, lon) out to `radius_m`, `ring_px` cells a side, first ring at the grid's cell size.
pub fn build(grid: &DsmGrid, lat: f64, lon: f64, radius_m: f64, ring_px: u32) -> Built {
    let mut rings = Vec::new();
    let mut cell = grid.meta.cell_m.max(1.0);
    loop {
        rings.push(cell);
        if cell * f64::from(ring_px) / 2.0 >= radius_m {
            break;
        }
        cell *= 4.0;
    }
    let maxes: Vec<Vec<f32>> = rings.iter().map(|&c| ring_max(grid, lat, lon, c, ring_px)).collect();
    let nodata = grid.meta.nodata;
    let lowest = maxes.iter().flatten().copied().filter(|&v| v != nodata).fold(f32::INFINITY, f32::min);
    let base_m = if lowest.is_finite() { f64::from(lowest).floor() - 1.0 } else { 0.0 };

    let mut bytes = Vec::with_capacity(maxes.len() * (ring_px * ring_px * 2) as usize);
    let mut metas = Vec::new();
    for (cell_m, data) in rings.iter().zip(&maxes) {
        metas.push(SceneRing { cell_m: *cell_m, width: ring_px, height: ring_px, byte_offset: bytes.len() as u64 });
        for &v in data {
            bytes.extend_from_slice(&encode_dm(v, base_m, nodata).to_le_bytes());
        }
    }
    let id = format!("{:016x}", fnv1a(&bytes) ^ fnv1a(format!("{lat:.6},{lon:.6},{base_m}").as_bytes()));
    let attribution = match grid.meta.source.provider.as_str() {
        "vexcel" => "Height data © Vexcel Imaging US, Inc.",
        "usgs-3dep" => "Height data: USGS 3D Elevation Program (LiDAR)",
        _ => "Synthetic height data",
    }
    .to_string();
    let meta = SceneMeta {
        data_url: format!("/api/scene/{id}.bin"),
        id,
        center_lat: lat,
        center_lon: lon,
        base_m,
        encoding: SCENE_ENCODING.into(),
        nodata: SCENE_NODATA,
        rings: metas,
        bytes: bytes.len() as u64,
        attribution,
    };
    Built { meta, bytes }
}

/// One ring: `n x n` cells of `cell_m`, row 0 north, each the max of the grid cells inside it.
fn ring_max(grid: &DsmGrid, lat: f64, lon: f64, cell_m: f64, n: u32) -> Vec<f32> {
    let g = &grid.meta;
    let nodata = g.nodata;
    // where the ring's centre sits on the grid, in grid cells
    let (gc, gr) = grid.cell_of(lat, lon);
    let k = cell_m / g.cell_m; // grid cells per ring cell
    let half = f64::from(n) / 2.0;
    let mut out = vec![nodata; (n * n) as usize];
    for r in 0..n {
        let r0 = gr + (f64::from(r) - half) * k;
        let (ra, rb) = (r0.floor().max(0.0) as i64, ((r0 + k).ceil() as i64).min(i64::from(g.height)));
        if ra >= rb {
            continue;
        }
        for c in 0..n {
            let c0 = gc + (f64::from(c) - half) * k;
            let (ca, cb) = (c0.floor().max(0.0) as i64, ((c0 + k).ceil() as i64).min(i64::from(g.width)));
            let mut best = nodata;
            for rr in ra..rb {
                let row = &grid.data[(rr * i64::from(g.width)) as usize..];
                for cc in ca..cb {
                    let v = row[cc as usize];
                    if v != nodata && (best == nodata || v > best) {
                        best = v;
                    }
                }
            }
            out[(r * n + c) as usize] = best;
        }
    }
    out
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::scene::decode_dm;
    use contracts::{DsmMeta, DsmSource, NODATA};

    /// 200 x 200 grid of 1 m, ground at 50 m, one 30 m tower 40 m east of the centre.
    fn grid() -> DsmGrid {
        let n = 200u32;
        let mut data = vec![50.0f32; (n * n) as usize];
        for r in 95..105 {
            for c in 140..150 {
                data[r * n as usize + c] = 80.0;
            }
        }
        let source = DsmSource { provider: "vexcel".into(), collection: None, captured_from: None, captured_to: None, native_res_m: 0.06 };
        let meta = DsmMeta { center_lat: 34.0, center_lon: -118.0, cell_m: 1.0, width: n, height: n, nodata: NODATA, source };
        DsmGrid { meta, data }
    }

    #[test]
    fn rings_reach_the_radius_and_keep_the_tower() {
        let b = build(&grid(), 34.0, -118.0, 120.0, 64);
        let cells: Vec<f64> = b.meta.rings.iter().map(|r| r.cell_m).collect();
        assert_eq!(cells, vec![1.0, 4.0]); // 64 x 4 m = 256 m across covers 120 m each way
        assert_eq!(b.meta.bytes, 2 * 64 * 64 * 2);
        assert_eq!(b.meta.base_m, 49.0);
        let decode = |ring: usize, r: usize, c: usize| {
            let off = b.meta.rings[ring].byte_offset as usize + (r * 64 + c) * 2;
            decode_dm(u16::from_le_bytes([b.bytes[off], b.bytes[off + 1]]), b.meta.base_m)
        };
        // ring 1 (4 m cells): tower at 40-50 m east -> columns 32+10..32+12
        assert_eq!(decode(1, 32, 42), Some(80.0));
        assert_eq!(decode(1, 32, 20), Some(50.0));
        // ring 0 covers only 32 m each way: the tower is outside it
        assert!((0..64).all(|c| decode(0, 32, c) == Some(50.0)));
        // the outer ring runs past the 200 m grid: nodata there
        assert_eq!(decode(1, 0, 0), None);
        assert!(b.meta.data_url.ends_with(".bin") && b.meta.attribution.contains("Vexcel"));
    }

    #[test]
    fn same_input_same_id() {
        assert_eq!(build(&grid(), 34.0, -118.0, 100.0, 32).meta.id, build(&grid(), 34.0, -118.0, 100.0, 32).meta.id);
    }
}

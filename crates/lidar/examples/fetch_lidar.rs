//! Fetch a LiDAR height map and write it as `<out>.json` + `<out>.f32` (the DsmGrid file pair).
//! `cargo run --release -p lidar --example fetch_lidar -- 40.3487 -74.6593 1000 cache/princeton_dsm`
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lat, lon, radius): (f64, f64, f64) = (a[0].parse()?, a[1].parse()?, a[2].parse()?);
    let out = a.get(3).cloned().unwrap_or_else(|| "cache/lidar_dsm".into());
    let t = Instant::now();
    let g = lidar::get_dsm(lat, lon, radius).await?;
    let filled = g.data.iter().filter(|&&v| v != lidar::NODATA).count();
    let (lo, hi) = g.data.iter().filter(|&&v| v != lidar::NODATA).fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    println!(
        "{}x{} cells of {} m in {:.1}s; {:.1}% filled; z {lo:.1}..{hi:.1}; {:?}",
        g.meta.width, g.meta.height, g.meta.cell_m, t.elapsed().as_secs_f64(),
        100.0 * filled as f64 / g.data.len() as f64, g.meta.source
    );
    let name = std::path::Path::new(&out).file_name().unwrap().to_string_lossy().to_string();
    let (json, bytes) = g.to_file_bytes(&name);
    if let Some(d) = std::path::Path::new(&out).parent() { std::fs::create_dir_all(d)?; }
    std::fs::write(format!("{out}.json"), json)?;
    std::fs::write(format!("{out}.f32"), bytes)?;
    Ok(())
}

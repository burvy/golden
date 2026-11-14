//! Try Vexcel for real: `cargo run -p vexcel --example fetch_dsm -- <lat> <lon> <radius_m>`
//! Defaults to 60 m around Pershing Square, downtown LA. Writes `cache/vexcel/out/dsm.{json,f32}`.

use std::time::Instant;
use vexcel::NODATA;

#[tokio::main]
async fn main() -> Result<(), vexcel::Error> {
    let args: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (lat, lon, radius) = match args[..] {
        [a, b, c] => (a, b, c),
        _ => (34.0490, -118.2510, 60.0),
    };
    let plan = vexcel::plan_tiles(lat, lon, radius, vexcel::DsmOptions::default().near_radius_m);
    let fine = plan.iter().filter(|t| t.level == 0).count();
    println!("{lat}, {lon}, radius {radius} m -> {fine} fine + {} coarse tiles", plan.len() - fine);

    let started = Instant::now();
    let g = vexcel::get_dsm(lat, lon, radius).await?;
    let valid: Vec<f32> = g.data.iter().copied().filter(|&v| v != NODATA).collect();
    let min = valid.iter().cloned().fold(f32::MAX, f32::min);
    let max = valid.iter().cloned().fold(f32::MIN, f32::max);
    println!("{} x {} cells of {} m in {:.1?}", g.meta.width, g.meta.height, g.meta.cell_m, started.elapsed());
    println!("heights {min:.1} .. {max:.1} m, {:.1}% cells with data", 100.0 * valid.len() as f64 / g.data.len() as f64);
    println!("source {:?}", g.meta.source);

    let out = std::path::Path::new("cache").join("vexcel").join("out");
    g.write_files(&out, "dsm")?;
    println!("wrote {}", out.join("dsm.json").display());
    Ok(())
}

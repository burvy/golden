//! Try obliques for real: `cargo run -p vexcel --example fetch_oblique -- <lat> <lon> <az_deg>`
//! Defaults to downtown LA looking toward the June sunset (297°). Writes `cache/vexcel/out/oblique.jpg`.

#[tokio::main]
async fn main() -> Result<(), vexcel::Error> {
    let args: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (lat, lon, az) = match args[..] {
        [a, b, c] => (a, b, c),
        _ => (34.0490, -118.2510, 297.0),
    };
    let started = std::time::Instant::now();
    let o = vexcel::get_oblique_with(lat, lon, az, &vexcel::ObliqueOptions::default()).await?;
    println!(
        "asked for {az}°, camera looks {}° ({:?}): {} x {} px, {} KB in {:.1?}",
        o.look_az_deg,
        vexcel::Facing::nearest(az),
        o.width,
        o.height,
        o.jpeg.len() / 1024,
        started.elapsed()
    );
    let out = std::path::Path::new("cache").join("vexcel").join("out");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("oblique.jpg"), &o.jpeg)?;
    println!("wrote {}", out.join("oblique.jpg").display());
    Ok(())
}

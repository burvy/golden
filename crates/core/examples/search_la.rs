//! Run search on real downtown LA heights, for all 3 framings on two dates, and print the top spots.
//!
//! `cargo run -p golden-core --release --example search_la -- [lat lon radius_m] [--max N] [--ground|--roof] [--json]`
//!
//! Defaults: Pershing Square, 400 m area. The height map is fetched once with vexcel (area + 1500 m of
//! skyline, as the server does) and cached under `cache/vexcel/`. Only the first run needs a key
//! (`VEXCEL_API_KEY`, or `VEXCEL_ENV_FILE`); the key is never printed.
//! Fine 6 cm tiles are skipped (`near_radius_m` 1 m): search uses 1 m cells and >=20 m candidates,
//! so the ~0.9 m far tiles are enough and the fetch stays small.
//! `--json` also writes each result to `cache/search_la/<date>-<framing>-<standing>.json` for a closer look.

use std::time::Instant;

use contracts::{Area, Brief, Event, Framing, Spot, Standing};
use golden_core::search::{search, skyline_need_m, SearchOutput};

const DATES: [(&str, i32, &str); 2] = [("2026-06-21", -7 * 3600, "PDT"), ("2026-11-14", -8 * 3600, "PST")];
const FRAMINGS: [Framing; 3] = [Framing::Clear, Framing::Between, Framing::Behind];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let nums: Vec<f64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let (lat, lon, radius_m) = match nums[..] {
        [a, b, c, ..] => (a, b, c),
        _ => (34.0490, -118.2510, 400.0),
    };
    let max_spots = flag_value(&args, "--max").unwrap_or(5);
    let write_json = args.iter().any(|a| a == "--json");
    let standing = if args.iter().any(|a| a == "--ground") {
        Standing::Ground
    } else if args.iter().any(|a| a == "--roof") {
        Standing::Roof
    } else {
        Standing::Any
    };

    let opts = vexcel::DsmOptions { near_radius_m: 1.0, ..vexcel::DsmOptions::default() };
    let started = Instant::now();
    let grid = vexcel::get_dsm_with(lat, lon, skyline_need_m(radius_m), &opts).await?;
    println!(
        "Height map: {} x {} cells of {} m around {lat}, {lon} ({:.1?})",
        grid.meta.width,
        grid.meta.height,
        grid.meta.cell_m,
        started.elapsed()
    );

    for (date, offset_s, zone) in DATES {
        for framing in FRAMINGS {
            let brief = Brief {
                text: String::new(),
                area: Area { lat, lon, radius_m },
                date: date.to_string(),
                tz: contracts::search::DEFAULT_TZ.to_string(),
                event: Event::Sunset,
                standing,
                framing,
                max_spots,
                goals: Vec::new(),
                lens_deg: contracts::DEFAULT_LENS_DEG,
            };
            let t = Instant::now();
            let out = search(&grid, &brief, offset_s)?;
            print_result(date, zone, framing, standing, &out, t.elapsed());
            if write_json {
                save_json(date, framing, standing, &out)?;
            }
        }
    }
    Ok(())
}

fn flag_value(args: &[String], name: &str) -> Option<u32> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1)?.parse().ok()
}

fn print_result(date: &str, zone: &str, framing: Framing, standing: Standing, out: &SearchOutput, took: std::time::Duration) {
    let scored = out.heat.values.iter().flatten().count();
    let above_half = out.heat.values.iter().flatten().filter(|&&v| v >= 0.5).count();
    println!();
    println!("=== {date} sunset, {framing:?}, standing {standing:?} ({zone}) ===");
    println!("{} candidates searched, {above_half} of {scored} score >= 0.5, {:.2?}", out.searched, took);
    if out.spots.is_empty() {
        println!("  no spot scored above 0");
    }
    for s in &out.spots {
        print_spot(s);
    }
}

fn print_spot(s: &Spot) {
    let standing = match s.standing {
        Standing::Roof => "roof",
        _ => "ground",
    };
    let sun = &s.sun;
    println!(
        "#{} score {:.2}  {standing}, surface {:.0} m  {}",
        s.rank,
        s.score,
        s.ground_m,
        maps_link(s.lat, s.lon)
    );
    println!(
        "   flat sunset {}  hidden {}{}  golden min in view {}",
        sun.flat_sunset.as_deref().unwrap_or("-"),
        sun.hidden_at.as_deref().unwrap_or("never"),
        match (sun.hidden_az_deg, sun.hidden_elev_deg) {
            (Some(az), Some(el)) => format!(" (az {az:.1}, sun {el:.1} deg up)"),
            _ => String::new(),
        },
        sun.golden_min_visible
    );
    for w in &s.why {
        println!("   - {w}");
    }
}

fn maps_link(lat: f64, lon: f64) -> String {
    format!("https://www.google.com/maps?q={lat:.5},{lon:.5}")
}

fn save_json(date: &str, framing: Framing, standing: Standing, out: &SearchOutput) -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new("cache").join("search_la");
    std::fs::create_dir_all(&dir)?;
    let body = serde_json::json!({ "spots": out.spots, "heat": out.heat, "searched": out.searched });
    let path = dir.join(format!("{date}-{framing:?}-{standing:?}.json").to_lowercase());
    std::fs::write(&path, serde_json::to_string_pretty(&body)?)?;
    Ok(())
}

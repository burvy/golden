//! Try the real kit: buildings near a point, nearest first.
//!
//!   $env:COSTAR_KIT_DIR = "C:\path\to\costar-hackathon-kit"
//!   cargo run --release -p costar --example near -- 34.0490 -118.2510 300
//!
//! Prints to your terminal only. Don't paste the output into git, issues or docs (CoStar licence).

fn main() {
    let args: Vec<f64> = std::env::args().skip(1).map(|a| a.parse().expect("numbers: lat lon radius_m")).collect();
    let [lat, lon, radius_m] = args[..] else {
        eprintln!("usage: near <lat> <lon> <radius_m>");
        std::process::exit(2);
    };

    let started = std::time::Instant::now();
    let kit = costar::shared_kit().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    eprintln!("loaded {} buildings in {:.1?}", kit.len(), started.elapsed());

    let near = kit.buildings_near(lat, lon, radius_m);
    let count = |f: fn(&costar::Building) -> bool| near.iter().filter(|b| f(b)).count();
    println!(
        "{} buildings within {radius_m} m: {} with contacts ({} with a manager), {} with a CoStar height, {} with Matterport tours",
        near.len(),
        count(|b| !b.contacts.is_empty()),
        count(|b| b.contacts.iter().any(|c| c.role == costar::ContactRole::Manager)),
        count(|b| b.height_m.is_some()),
        count(|b| !b.matterport.is_empty()),
    );
    for b in near.iter().take(10) {
        println!(
            "{:>6.0} m  {:<12} {:<40} height {:>6} floors {:>3}  contacts {}  tours {}",
            b.dist_m.unwrap_or_default(),
            b.id,
            b.name.as_deref().or(b.address.as_deref()).unwrap_or("?"),
            b.height_m.map(|h| format!("{h:.1} m")).unwrap_or("-".into()),
            b.floors.map(|f| f.to_string()).unwrap_or("-".into()),
            b.contacts.iter().map(|c| format!("{:?}", c.role).to_lowercase()).collect::<Vec<_>>().join("/"),
            b.matterport.len(),
        );
    }
}

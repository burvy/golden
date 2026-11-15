//! The cost-function library (docs/metrics.md v2): the seeds that ship in `library/*.json` (compiled into core,
//! read-only) plus the ones the agent saved at runtime, one file each in `<cache>/library/<name>.json`
//! (gitignored; promote a good one by copying it into `library/`).
//!
//! Robustness rules:
//! - Names are `[a-z][a-z0-9_]*`, at most `NAME_MAX` characters, so a name can't be a path.
//! - A saved name can't shadow a seed or replace an earlier save: reuse it, or pick a new name.
//! - Every file is checked again on load (`core::expr::check`); one that no longer passes is skipped and logged,
//!   so one bad file can't break the library. A file whose name doesn't match its entry is skipped too.
//! - At most `MAX_SAVED` saved entries.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use contracts::CostFn;
use golden_core::expr::check;
use golden_core::features::Tier;

pub const NAME_MAX: usize = 40;
pub const MAX_SAVED: usize = 200;

/// Saves are check-then-write; one at a time so two can't take the same name.
static SAVING: Mutex<()> = Mutex::new(());

/// Where an entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Seed,
    Saved,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Seed => "seed",
            Origin::Saved => "saved",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub cost: CostFn,
    pub origin: Origin,
    pub tier: Tier,
}

/// The library under one cache root.
#[derive(Debug, Clone)]
pub struct Library {
    dir: PathBuf,
}

impl Library {
    pub fn at(cache_root: &Path) -> Library {
        Library { dir: cache_root.join("library") }
    }

    /// Seeds, then saved entries by name. Bad saved files are skipped (and logged).
    pub fn entries(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = golden_core::library::seeds()
            .into_iter()
            .filter_map(|c| check(&c).ok().map(|k| Entry { tier: k.tier(), cost: c, origin: Origin::Seed }))
            .collect();
        let seeds: Vec<String> = out.iter().map(|e| e.cost.name.clone()).collect();
        let mut saved: Vec<Entry> = self.saved_files().into_iter().filter_map(|p| load(&p, &seeds)).collect();
        saved.sort_by(|a, b| a.cost.name.cmp(&b.cost.name));
        out.extend(saved);
        out
    }

    /// Every cost function, for `golden_core::search::search_with`.
    pub fn all(&self) -> Vec<CostFn> {
        self.entries().into_iter().map(|e| e.cost).collect()
    }

    pub fn get(&self, name: &str) -> Option<Entry> {
        self.entries().into_iter().find(|e| e.cost.name == name)
    }

    /// Save a checked cost function under its name. Errors are worded for the agent.
    pub fn save(&self, cost: &CostFn) -> Result<PathBuf, String> {
        valid_name(&cost.name)?;
        check(cost).map_err(|errs| errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "))?;
        let _one = SAVING.lock().unwrap_or_else(|p| p.into_inner());
        match self.get(&cost.name) {
            Some(e) if e.origin == Origin::Seed => return Err(format!("\"{}\" is a built-in function; pick another name", cost.name)),
            Some(_) => return Err(format!("\"{}\" is already saved; reuse it with use, or save this one under a new name", cost.name)),
            None => {}
        }
        if self.saved_files().len() >= MAX_SAVED {
            return Err(format!("the library is full ({MAX_SAVED} saved functions)"));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("can't write the library: {e}"))?;
        let path = self.dir.join(format!("{}.json", cost.name));
        let tmp = path.with_extension("part");
        let text = serde_json::to_string_pretty(cost).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| format!("can't write the library: {e}"))?;
        Ok(path)
    }

    fn saved_files(&self) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect()
    }
}

/// Pure: why a name can't be used, if it can't.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = name.len() <= NAME_MAX
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("{name:?} isn't a good name: use a-z, 0-9 and _, starting with a letter, at most {NAME_MAX} characters"))
    }
}

/// One saved file, if it's still a good entry.
fn load(path: &Path, seeds: &[String]) -> Option<Entry> {
    let skip = |why: String| {
        eprintln!("library: skipping {}: {why}", path.display());
        None
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return skip(e.to_string()),
    };
    let cost: CostFn = match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(e) => return skip(e.to_string()),
    };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if stem != cost.name || valid_name(stem).is_err() || seeds.contains(&cost.name) {
        return skip(format!("file name {stem:?} doesn't match entry {:?}, or shadows a seed", cost.name));
    }
    match check(&cost) {
        Ok(k) => Some(Entry { tier: k.tier(), cost, origin: Origin::Saved }),
        Err(errs) => skip(errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn cost(json: &str) -> CostFn {
        serde_json::from_str(json).unwrap()
    }

    const SUNNY: &str = r#"{"name":"stays_sunny","about":"Keeps direct sun for most of golden hour",
        "must":{"op":"gt","x":{"f":"sun_visible_min"},"than":{"num":10}},
        "score":{"op":"ramp","x":{"f":"sun_visible_min"},"lo":10,"hi":30}}"#;

    #[test]
    fn names_are_plain() {
        assert!(valid_name("stays_sunny").is_ok() && valid_name("a1").is_ok());
        for bad in ["", "Sunny", "1st", "../x", "a/b", "a.b", "a b", &"x".repeat(NAME_MAX + 1)] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn seeds_are_listed_and_read_only() {
        let lib = Library::at(&testkit::temp_path("lib"));
        let names: Vec<String> = lib.entries().iter().map(|e| e.cost.name.clone()).collect();
        assert!(["sun_clear", "sun_between", "sun_behind"].iter().all(|n| names.contains(&n.to_string())), "{names:?}");
        assert!(lib.entries().iter().all(|e| e.origin == Origin::Seed && e.tier == Tier::Skyline));
        let e = lib.save(&cost(r#"{"name":"sun_clear","about":"mine","score":{"num":1}}"#)).unwrap_err();
        assert!(e.contains("built-in"), "{e}");
    }

    #[test]
    fn save_list_reuse_and_no_overwrite() {
        let root = testkit::temp_dir("lib");
        let lib = Library::at(&root);
        let path = lib.save(&cost(SUNNY)).unwrap();
        assert_eq!(path, root.join("library").join("stays_sunny.json"));
        let got = lib.get("stays_sunny").unwrap();
        assert_eq!((got.origin, got.tier), (Origin::Saved, Tier::Skyline));
        assert_eq!(got.cost, cost(SUNNY));
        assert!(lib.all().iter().any(|c| c.name == "stays_sunny"));
        let e = lib.save(&cost(SUNNY)).unwrap_err();
        assert!(e.contains("already saved"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_functions_are_not_saved() {
        let lib = Library::at(&testkit::temp_path("lib"));
        let e = lib.save(&cost(r#"{"name":"shady","about":"x","score":{"f":"shade"}}"#)).unwrap_err();
        assert!(e.contains("score: unknown feature \"shade\""), "{e}");
        assert!(lib.save(&cost(r#"{"name":"../evil","about":"x","score":{"num":1}}"#)).is_err());
    }

    #[test]
    fn broken_files_are_skipped_not_fatal() {
        let root = testkit::temp_dir("lib");
        let lib = Library::at(&root);
        lib.save(&cost(SUNNY)).unwrap();
        let dir = root.join("library");
        std::fs::write(dir.join("garbage.json"), "{not json").unwrap();
        std::fs::write(dir.join("renamed.json"), SUNNY).unwrap(); // name inside doesn't match the file
        std::fs::write(dir.join("stale.json"), r#"{"name":"stale","about":"x","score":{"f":"gone_feature"}}"#).unwrap();
        std::fs::write(dir.join("sun_clear.json"), r#"{"name":"sun_clear","about":"x","score":{"num":1}}"#).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let saved: Vec<String> = lib.entries().into_iter().filter(|e| e.origin == Origin::Saved).map(|e| e.cost.name).collect();
        assert_eq!(saved, ["stays_sunny"]);
        assert_eq!(lib.get("sun_clear").unwrap().origin, Origin::Seed, "a file can't shadow a seed");
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! Shared data shapes from `docs/contracts.md`, as plain Rust structs.
//!
//! Pure on purpose: no network, no files, no clocks. `core` can depend on this crate; the IO crates
//! (`vexcel`, `costar`, `server`) fill these structs from the outside world.
//! Change a shape only in its own small PR, together with `docs/contracts.md` and its fixture.

pub mod dsm;

pub use dsm::{DsmFileError, DsmGrid, DsmMeta, DsmSource, EARTH_R, NODATA};

//! Shared data shapes from `docs/contracts.md`, as plain Rust structs.
//!
//! Pure on purpose: no network, no files, no clocks. `core` can depend on this crate; the IO crates
//! (`vexcel`, `costar`, `server`) fill these structs from the outside world.
//! Change a shape only in its own small PR, together with `docs/contracts.md` and its fixture.
//! Times are ISO 8601 strings with the local offset, so this crate needs no time library.

pub mod agent;
pub mod building;
pub mod dsm;
pub mod error;
pub mod goal;
pub mod horizon;
pub mod photo;
pub mod scene;
pub mod search;
pub mod sun;

pub use agent::{Access, AgentRequest, AgentStep, CallSheet, ChatAsk, ChatMessage, ChatReply, ChatRequest, ChatRole, SheetSpot, SpotRole, TimelineItem};
pub use building::{Building, BuildingList, Contact, ContactRole, Matterport};
pub use dsm::{DsmFileError, DsmGrid, DsmMeta, DsmSource, EARTH_R, NODATA};
pub use error::ApiError;
pub use goal::{CmpOp, CostFn, Expr, Goal, GoalFn, ListOp, ObjArg, ObjRef, Object, Rated, DEFAULT_LENS_DEG, EXPR_OPS, OBJECT_KINDS};
pub use horizon::{HorizonPoint, HorizonProfile, Observer, SunBlock};
pub use photo::{PhotoKind, PhotoMeta};
pub use scene::{SceneMeta, SceneRing};
pub use search::{Area, Brief, Event, Framing, HeatGrid, SearchResult, Spot, Standing};
pub use sun::{GoldenHour, SunEvent, SunPath, SunPoint, TimeRange};

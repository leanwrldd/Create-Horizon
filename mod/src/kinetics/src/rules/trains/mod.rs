//! Create's train system, parts 1 and 2: the rules for track geometry, the track graph, travelling along
//! it, signals, stations, navigation and schedules. See `README.md` in this folder for what is ported from
//! which Java file and what comes next.

pub mod bezier;
pub mod graph;
pub mod navigation;
pub mod network;
pub mod placement;
pub mod propagator;
pub mod schedule;
pub mod signal;
pub mod station;
pub mod track_shape;
pub mod travelling_point;
pub mod vec;

//! Create's train system, parts 1 to 3: the rules for track geometry, the track graph, travelling along
//! it, signals, stations, navigation, schedules, and the train itself (carriages, ticking, the manager's
//! train half, assembly, edge point targeting, observers). See `README.md` in this folder for what is ported from
//! which Java file and what comes next.

pub mod assembly;
pub mod bezier;
pub mod bogey;
pub mod carriage;
pub mod carriage_contraption;
pub mod carriage_pose;
pub mod edge_blocks;
pub mod graph;
pub mod graph_helper;
pub mod navigation;
pub mod network;
pub mod observer;
pub mod placement;
pub mod propagator;
pub mod schedule;
pub mod schedule_edit;
pub mod schedule_nbt;
pub mod signal;
pub mod railways;
pub mod station;
pub mod targeting;
pub mod train;
pub mod track_block;
pub mod train_nbt;
pub mod track_render;
pub mod track_shape;
pub mod travelling_point;
pub mod vec;

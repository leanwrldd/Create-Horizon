//! Create's train system, parts 1 to 3: the rules for track geometry, the track graph, travelling along
//! it, signals, stations, navigation, schedules, and the train itself (carriages, ticking, the manager's
//! train half, assembly, edge point targeting, observers). See `docs/trains.md` at the repository root for what is ported from
//! which Java file and what comes next.

pub mod assembly;
pub mod bezier;
pub mod bogey;
pub mod carriage;
pub mod carriage_contraption;
pub mod carriage_pose;
pub mod curve_interaction;
pub mod display;
pub mod edge_blocks;
pub mod graph;
pub mod graph_helper;
pub mod mail;
pub mod navigation;
pub mod network;
pub mod observer;
pub mod placement;
pub mod portal_cutoff;
pub mod portal_track;
pub mod propagator;
pub mod relocator;
pub mod schedule;
pub mod schedule_hand;
pub mod schedule_edit;
pub mod schedule_nbt;
pub mod signal;
pub mod railways;
pub mod station;
pub mod station_marker;
pub mod station_screen;
pub mod targeting;
pub mod train;
pub mod train_cargo;
pub mod track_block;
pub mod track_outline;
pub mod track_overlay;
pub mod train_controls;
pub mod train_doors;
pub mod train_hat;
pub mod train_nbt;
pub mod train_passengers;
pub mod track_render;
pub mod track_shape;
pub mod track_tilt;
pub mod travelling_point;
pub mod vec;

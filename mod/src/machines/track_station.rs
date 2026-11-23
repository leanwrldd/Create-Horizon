//! The Train Station (`StationBlock`, `StationBlockEntity`, `StationEditPacket`): a block that targets a spot on a track edge
//! and makes it a station of the graph (`GlobalStation`): a name, an assembly mode in which the track beyond it is scanned for
//! bogeys, and the place trains stop. The decisions are `create_kinetics::rules::trains::{edge_blocks, station, assembly,
//! targeting}`; the shared part (making the edge point) is `train_net.rs`.
//!
//! The block entity keeps the station's name (`StationName`) and gives it to the point whenever the point is made again (the
//! graphs aren't saved). In assembly mode (`assembling`) it scans the track every second (`refreshAssemblyInfo`: the
//! bogeys along the track, `scan_assembly`) and a click with a Train Casing on the assembly track puts a bogey there
//! (`trackClicked`); a click on a place that isn't a valid offset changes the bogey beside it to the other size.
//!
//! Known differences from Create, for the README:
//! - **No station or assembly screen** (`StationScreen`, `AssemblyScreen`): right-clicking the station tells its state and the
//!   `/station` command stands in for the buttons: `name <text>`, `assembly` (enter assembly mode), `cancel`, `assemble`,
//!   `disassemble`, `schedule`. It works on the station right-clicked last.
//! - **Assembling goes as far as the track**: `assemble` checks the bogeys (`frontmost_bogey_at_station`, `bogeys_too_close`,
//!   `no_bogeys`) and puts the wheel points on the graph (`place_travelling_points`), then stops: gathering each carriage's
//!   blocks into a moving structure (`CarriageContraption.assemble`) and the train's entities are part 3. Nothing is removed from
//!   the world and no train is made. Disassembling and the schedule slot have no train to work on until then.
//! - No train presence (`trainPresent`, `imminentTrain`, the comparator's 15, `applyAutoSchedule`): a station has no train yet.
//! - Not drawn: the flag (`StationRenderer`), the overlay on the track and the assembly outlines (`TRACK_ASSEMBLING_OVERLAY`,
//!   the green and red bogey spots): `/station` tells the bogey offsets instead.
//! - No door controls (`DoorControlBehaviour`), no depot slot for an auto-schedule, no map markers (`StationMarker`), no computer
//!   behaviour, no display sources, no package ports (`attachPackagePort`).
//! - Waterlogging is left out, as elsewhere.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::trains::bogey::BogeyType;
use create_kinetics::rules::trains::edge_blocks::{
    self as rules, AssemblyArea, EditStep, StationCommand, StationEdit, TrackClick,
};
use create_kinetics::rules::trains::graph::{EdgePointType, TrackNodeLocation};
use create_kinetics::rules::trains::graph_helper::TrackQuery;
use create_kinetics::rules::trains::propagator::TrackWorld;
use create_kinetics::rules::trains::station::{
    AssemblyError, AssemblyScan, PlacePointsError, assembly_direction, bogey_point_offsets, place_travelling_points,
    scan_assembly,
};
use create_kinetics::rules::trains::targeting::{BlockEntityInfo, TargetingState};
use create_kinetics::rules::trains::vec::{Vec3, block_containing};
use create_kinetics::Pos;
use crate::compat::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, Compound, Flow, HasAnalogOutputEvent, Hooks,
    Host, LevelHost, NativeCommand, NativeServerPlayer, PerformCommandEvent, RegisterCommandsEvent, RightClickBlockEvent, Tag,
    UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::train_net as net;
use super::track as world;
use crate::util;

pub const STATION: &str = "create:track_station";
const RAILWAY_CASING: &str = "create:railway_casing";
const NAME: &str = "StationName";
const SMALL_BOGEY: &str = "create:small_bogey";
const LARGE_BOGEY: &str = "create:large_bogey";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Stations;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Stations)
}

// --- the assembly scan, kept for the clicks and the commands --------------------------------------------------------------

/// What `refreshAssemblyInfo` found for a station.
#[derive(Clone, Debug)]
struct Assembly {
    /// The track block the station targets.
    target: Pos,
    /// `assemblyDirection`'s normal.
    direction: [i32; 3],
    /// `BlockPos.containing(track.getUpNormal(..))`.
    up: Pos,
    scan: AssemblyScan<BogeyType>,
}

/// `StationBlockEntity.assemblyAreas` and what each station last scanned, by dimension and station position.
static ASSEMBLIES: LazyLock<Mutex<HashMap<(&'static str, Pos), Assembly>>> = LazyLock::new(Mutex::default);

fn bogey_type(block: &str) -> Option<BogeyType> {
    match block {
        SMALL_BOGEY => Some(BogeyType::small()),
        LARGE_BOGEY => Some(BogeyType::large()),
        _ => None,
    }
}

fn offset(p: Pos, d: [i32; 3], n: i32) -> Pos {
    [p[0] + d[0] * n, p[1] + d[1] * n, p[2] + d[2] * n]
}

/// `getAssemblyDirection()` and `refreshAssemblyInfo()`: scans the track beyond the station. `None` when the station has no
/// valid track.
fn refresh(host: &dyn Host, target: &TargetingState, pos: Pos) -> Option<Assembly> {
    let track = target.global_position(pos);
    let shape = world::shape_at(host, track)?;
    let axis = *shape.axes().first()?;
    let direction = rules_direction(axis, if target.target_positive { 1 } else { -1 });
    let up = block_containing(shape.normal());
    let scan = scan_assembly(
        rules::MAX_ASSEMBLY_LENGTH,
        rules::MAX_BOGEY_COUNT,
        |i| world::shape_at(host, offset(track, direction, i + 1)) == Some(shape),
        |i, upside_down| {
            if upside_down {
                // `isUpsideDown` is false for the standard bogeys: nothing hangs below.
                return None;
            }
            let at = offset(track, direction, i + 1);
            let at = [at[0] + up[0], at[1] + up[1], at[2] + up[2]];
            bogey_type(&host.block(at)).map(|b| (b, false))
        },
    );
    Some(Assembly { target: track, direction, up, scan })
}

fn rules_direction(axis: Vec3, step: i32) -> [i32; 3] {
    assembly_direction(axis, step).normal()
}

fn assembling(host: &dyn Host, pos: Pos) -> bool {
    host.block_property(pos, "assembling").as_deref() == Some("true")
}

// --- the tick -----------------------------------------------------------------------------------------------------------

fn text(nbt: &Compound, key: &str) -> Option<String> {
    match util::get(nbt, key) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// `StationBlockEntity.tick` and `lazyTick` (server).
fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let level = signal::host_level(&*host);
    let dimension = host.dimension();
    let mut nbt = event.entity.nbt.clone();
    let mut target = net::load(&nbt, EdgePointType::Station, pos);
    let assembling = assembling(&*host, pos);
    let info = BlockEntityInfo {
        pos,
        dimension: world::dimension_id(dimension),
        signal_type: None,
        reported_power: false,
        assembling,
        filter: None,
    };
    net::ensure_point(&*host, &mut target, &info);

    // The point and the block entity agree on the name (the block entity's is the saved one) and on assembling.
    let saved = text(&nbt, NAME);
    let found = net::with_point(&*host, EdgePointType::Station, target.id, |point| {
        let Some(station) = point.station_mut() else { return None };
        match &saved {
            Some(name) if station.name != *name => station.name = name.clone(),
            _ => {}
        }
        if station.assembling != assembling {
            station.assembling = assembling;
        }
        station.block_entity_pos = Some(pos);
        Some(station.name.clone())
    })
    .flatten();
    if saved.is_none() && let Some(name) = found {
        nbt.insert(NAME.into(), Tag::String(name.as_str().into()));
    }

    // `lazyTick`: every second while assembling, the track is scanned again.
    if assembling {
        if util::game_time(&*host) % 20 == 0 || !lock_assemblies().contains_key(&(dimension, pos)) {
            match refresh(&*host, &target, pos) {
                Some(found) => {
                    lock_assemblies().insert((dimension, pos), found);
                }
                None => {
                    lock_assemblies().remove(&(dimension, pos));
                }
            }
        }
    } else if !lock_assemblies().is_empty() {
        lock_assemblies().remove(&(dimension, pos));
    }
    net::set_output(level, pos, rules::station_analog_output(false));

    net::store(&target, &mut nbt, pos);
    if nbt != event.entity.nbt {
        event.entity.nbt = nbt;
    }
}

fn lock_assemblies() -> std::sync::MutexGuard<'static, HashMap<(&'static str, Pos), Assembly>> {
    net::lock(&ASSEMBLIES)
}

// --- what the player is told -----------------------------------------------------------------------------------------------

/// The station each player works with: the one right-clicked last.
static OPEN: LazyLock<Mutex<HashMap<u128, (&'static str, Pos)>>> = LazyLock::new(Mutex::default);

fn station_name(host: &dyn Host, pos: Pos) -> String {
    host.mod_block_entity(pos).and_then(|e| text(&e.nbt, NAME)).unwrap_or_else(|| "Track Station".to_owned())
}

/// The lines `/station` and a right click show.
fn describe(host: &dyn Host, pos: Pos) -> Vec<String> {
    let name = station_name(host, pos);
    let mut lines = vec![format!("Train Station '{name}'")];
    if !assembling(host, pos) {
        lines.push("Not assembling. /station assembly enters assembly mode; /station name <text> renames it.".to_owned());
        return lines;
    }
    let found = lock_assemblies().get(&(host.dimension(), pos)).cloned();
    match found {
        None => lines.push("Assembling, but the station has no straight track to build on.".to_owned()),
        Some(a) => {
            lines.push(format!(
                "Assembling: {} blocks of assembly track, {} bogey{} found.",
                a.scan.length,
                a.scan.bogey_count(),
                if a.scan.bogey_count() == 1 { "" } else { "s" }
            ));
            if !a.scan.bogey_locations.is_empty() {
                let places: Vec<String> = a.scan.bogey_locations.iter().map(|l| l.to_string()).collect();
                lines.push(format!("Bogeys at offsets {} along the track.", places.join(", ")));
            }
            lines.push("Right-click the track with Train Casing to put a bogey (offset 0 or at least 3 from the others). /station assemble checks them; /station cancel leaves assembly mode.".to_owned());
        }
    }
    lines
}

fn say_error(host: &mut dyn Host, uuid: u128, error: &AssemblyError) {
    let (key, args) = rules::assembly_error_message(error);
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    net::say(host, uuid, &key, &args);
}

// --- the edits ----------------------------------------------------------------------------------------------------------------

/// `StationBlockEntity.tryEnterAssemblyMode`.
fn can_enter_assembly(host: &dyn Host, pos: Pos) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else { return false };
    let target = net::load(&entity.nbt, EdgePointType::Station, pos);
    let axes = net::query(host).axes(target.global_position(pos));
    rules::can_enter_assembly_mode(!axes.is_empty(), axes.first().copied())
}

/// `updateStationState`: a change to the point's own data, `false` when there is no point.
fn update_station(host: &dyn Host, pos: Pos, change: impl FnOnce(&mut create_kinetics::rules::trains::station::GlobalStation)) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else { return false };
    let id = net::load(&entity.nbt, EdgePointType::Station, pos).id;
    net::with_point(host, EdgePointType::Station, id, |point| point.station_mut().map(change)).flatten().is_some()
}

/// `StationBlockEntity.assemble`, as far as the track goes (see the module header).
fn assemble(host: &mut dyn Host, uuid: u128, pos: Pos) {
    let dimension = host.dimension();
    let Some(entity) = host.mod_block_entity(pos) else { return };
    let target = net::load(&entity.nbt, EdgePointType::Station, pos);
    // `refreshAssemblyInfo`.
    let Some(assembly) = refresh(&*host, &target, pos) else {
        net::tell(host, uuid, "The station has no track to assemble on");
        return;
    };
    lock_assemblies().insert((dimension, pos), assembly.clone());
    let scan = &assembly.scan;
    let offsets = match bogey_point_offsets(&scan.bogey_locations, |i| scan.bogey_types[i].wheel_point_spacing) {
        Ok(offsets) => offsets,
        Err(error) => {
            say_error(host, uuid, &error);
            return;
        }
    };
    // The end of the station's track block in the assembly direction.
    let query = net::query(&*host);
    let track = assembly.target;
    let elevation = query.elevation_at_center(track);
    let centre = [f64::from(track[0]) + 0.5, f64::from(track[1]) + elevation, f64::from(track[2]) + 0.5];
    let dir_vec = [f64::from(assembly.direction[0]), f64::from(assembly.direction[1]), f64::from(assembly.direction[2])];
    let ends: Vec<(TrackNodeLocation, Vec3)> = query.connected(track, true, None).iter().map(|e| (e.loc, e.loc.location())).collect();
    let Some(start) = create_kinetics::rules::trains::assembly::find_assembly_start(&ends, centre, dir_vec) else {
        net::tell(host, uuid, "Cannot assemble: the station's track has no end that way");
        return;
    };
    let length = scan.length;
    let placed = net::with_nets(&*host, |nets| {
        // `getGraph(level, currentLocation)` is asked at each step until a graph answers.
        let mut graph = None;
        let mut at = start;
        for _ in 0..(length * 2 + 40) {
            if let Some(found) = nets.get_graph(&at) {
                graph = Some(found);
                break;
            }
            let next = [at.location()[0] + dir_vec[0] * 0.5, at.location()[1] + dir_vec[1] * 0.5, at.location()[2] + dir_vec[2] * 0.5];
            at = TrackNodeLocation::new(next, at.dimension);
        }
        let graph = nets.graphs.get(&graph?)?;
        Some(place_travelling_points(graph, start, dir_vec, length, &offsets))
    })
    .flatten();
    match placed {
        None => net::tell(host, uuid, "Cannot assemble: the track isn't part of a graph yet (build or touch it so it joins one)"),
        Some(Err(error)) => net::tell(
            host,
            uuid,
            match error {
                PlacePointsError::NoStartingNode => "Cannot assemble: No valid starting node found",
                PlacePointsError::MissingEdge => "Cannot assemble: Missing graph edge",
                PlacePointsError::NotAllPoints => "Cannot assemble: Not all Points created",
            },
        ),
        Some(Ok(points)) => net::tell(
            host,
            uuid,
            format!(
                "The track is ready for {} bogey{} ({} wheel points). Attaching the carriages (a moving structure each) and making the train isn't implemented yet.",
                scan.bogey_count(),
                if scan.bogey_count() == 1 { "" } else { "s" },
                points.len()
            ),
        ),
    }
}

/// `StationBlockEntity.enterAssemblyMode` / `exitAssemblyMode`'s block half.
fn set_assembling(host: &mut dyn Host, pos: Pos, on: bool) {
    host.set_block_property(pos, "assembling", if on { "true" } else { "false" });
    update_station(&*host, pos, |s| s.assembling = on);
    let dimension = host.dimension();
    if !on {
        lock_assemblies().remove(&(dimension, pos));
    }
    host.block_entity_sync(pos);
}

/// `StationEditPacket.applySettings`.
fn apply_edit(host: &mut dyn Host, uuid: u128, pos: Pos, edit: &StationEdit) {
    let is_station = host.block(pos) == STATION;
    let steps = rules::plan_edit(edit, is_station, assembling(&*host, pos));
    for step in steps {
        match step {
            EditStep::DropSchedule => net::tell(host, uuid, "No train is standing at this station"),
            EditStep::Rename(name) => {
                host.change_mod_block_entity(pos, &mut |e| {
                    e.nbt.insert(NAME.into(), Tag::String(name.as_str().into()));
                });
                if !update_station(&*host, pos, |s| s.name = name.clone()) {
                    net::tell(host, uuid, "The station isn't on the track yet; the name is kept");
                }
                host.block_entity_sync(pos);
                net::tell(host, uuid, format!("The station is now called '{name}'"));
            }
            EditStep::Assemble => assemble(host, uuid, pos),
            EditStep::Disassemble => net::tell(host, uuid, "No train is standing at this station"),
            // No train is made, so there is never one present afterwards.
            EditStep::ContinueIfTrainPresent => return,
            EditStep::EnterAssembly => {
                if assembling(&*host, pos) {
                    continue;
                }
                if !can_enter_assembly(&*host, pos) {
                    net::tell(host, uuid, "Cannot build trains on curved or diagonal tracks (or the station has no track)");
                    return;
                }
                set_assembling(host, pos, true);
                net::tell(host, uuid, "Assembly mode on");
            }
            EditStep::ExitAssembly => {
                if assembling(&*host, pos) {
                    set_assembling(host, pos, false);
                    net::tell(host, uuid, "Assembly mode off");
                }
            }
        }
    }
}

// --- the clicks on the track in the assembly area -----------------------------------------------------------------------------

/// `StationBlockEntity.trackClicked`; true when the click was taken.
fn track_clicked(host: &mut dyn Host, player: &NativeServerPlayer, station: Pos, assembly: &Assembly, pos: Pos) -> bool {
    let area = AssemblyArea::new(assembly.target, assembly.direction, assembly.scan.length);
    let bogey_offset = rules::dist_manhattan(pos, assembly.target) - 1;
    let creative = player.game_mode == util::CREATIVE;
    let holds_casing = host.held_stack(player.uuid).is_some_and(|s| &*s.item == RAILWAY_CASING);
    let outcome = rules::track_click(area.contains(pos), assembly.scan.is_valid_bogey_offset(bogey_offset), creative, holds_casing);
    let dimension = host.dimension();
    match outcome {
        TrackClick::Ignored => false,
        TrackClick::NeedsCasing => {
            net::say(host, player.uuid, "create.train_assembly.requires_casing", &[]);
            false
        }
        TrackClick::CycleSize => {
            for (upside_down, i) in rules::bogey_search_order() {
                let along = offset(pos, assembly.direction, i);
                let sign = if upside_down { -1 } else { 1 };
                let bogey_pos = [along[0] + assembly.up[0], along[1] + assembly.up[1] * sign, along[2] + assembly.up[2]];
                let block = host.block(bogey_pos);
                let Some(next) = rules::next_bogey_block(&block) else { continue };
                let axis = host.block_property(bogey_pos, "axis");
                let Some(state) = host.default_state(next) else { continue };
                host.set_block(dimension, bogey_pos, state, util::UPDATE_ALL);
                if let Some(axis) = axis {
                    host.set_block_property(bogey_pos, "axis", &axis);
                }
                host.play_sound("minecraft:block.netherite_block.hit", util::centre(bogey_pos), BLOCKS, 1.0, 1.0);
                return true;
            }
            false
        }
        TrackClick::PlaceBogey { consume_casing } => {
            let at = [pos[0] + assembly.up[0], pos[1] + assembly.up[1], pos[2] + assembly.up[2]];
            let Some(shape) = world::shape_at(&*host, pos) else { return false };
            let Some(state) = host.default_state(SMALL_BOGEY) else { return false };
            // `level.destroyBlock(targetPos, true)`, then the bogey in its place.
            if host.block(at) != "minecraft:air" {
                host.destroy_block(at, None, true);
            }
            host.set_block(dimension, at, state, util::UPDATE_ALL);
            host.set_block_property(at, "axis", rules::bogey_axis(shape));
            net::say(host, player.uuid, "create.train_assembly.bogey_created", &[]);
            host.play_sound("minecraft:block.netherite_block.place", util::centre(at), BLOCKS, 1.0, 0.8);
            if consume_casing {
                host.take_items(RAILWAY_CASING, 1);
            }
            let _ = station;
            true
        }
    }
}

// --- the hooks --------------------------------------------------------------------------------------------------------------------

impl Hooks for Stations {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == STATION).then(|| (STATION.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == STATION {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == STATION {
            tick(host, event);
        }
    }

    /// `StationBlock.getAnalogOutputSignal`: 15 while a train is here (never, until trains).
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if signal::boolean(level, event.state, "assembling").is_some() {
            event.output = true;
        }
    }

    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if signal::boolean(level, event.state, "assembling").is_some() {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            event.output = net::output(signal::level_index(event.dimension), pos);
        }
    }

    /// `StationBlock.useItemOn` opens the screen; here it tells the station's state and remembers it for `/station`.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != STATION {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.sneaking || host.held_stack(player.uuid).is_some_and(|s| &*s.item == util::WRENCH) {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        net::lock(&OPEN).insert(player.uuid, (host.dimension(), pos));
        for line in describe(&*host, pos) {
            net::tell(host, player.uuid, line);
        }
        Flow::Cancel
    }

    /// `TrackBlock.useItemOn`: a click on the track inside a station's assembly area.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != world_track() {
            return Flow::Continue;
        }
        let dimension = host.dimension();
        let stations: Vec<(Pos, Assembly)> = lock_assemblies()
            .iter()
            .filter(|((d, _), a)| {
                *d == dimension && AssemblyArea::new(a.target, a.direction, a.scan.length).contains(pos)
            })
            .map(|((_, p), a)| (*p, a.clone()))
            .collect();
        if stations.is_empty() {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        // A track in hand is the track item's own click.
        if &*event.stack.item == world_track() {
            return Flow::Continue;
        }
        // The rest of a click this handler took: cancel it. A click it doesn't take goes on to the item (`useItemOn` passes).
        if util::click_repeated(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        for (station, assembly) in stations {
            // `refreshAssemblyInfo` first.
            let fresh = host.mod_block_entity(station).and_then(|e| {
                let target = net::load(&e.nbt, EdgePointType::Station, station);
                refresh(&*host, &target, station)
            });
            let assembly = fresh.unwrap_or(assembly);
            if track_clicked(host, &player, station, &assembly, pos) {
                lock_assemblies().insert((dimension, station), assembly);
                util::click_taken(&*host, player.uuid, pos);
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("station"));
        event.commands.push(NativeCommand::new("station <text:text>"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "station" { Some("") } else { line.strip_prefix("station ") }) else {
            return Flow::Continue;
        };
        let uuid = event.player.uuid;
        let opened = net::lock(&OPEN).get(&uuid).copied();
        let Some((dimension, pos)) = opened.filter(|(d, p)| *d == host.dimension() && host.block(*p) == STATION) else {
            net::tell(host, uuid, "Right-click a Train Station first");
            return Flow::Cancel;
        };
        let _ = dimension;
        match StationCommand::parse(rest) {
            Err(message) => net::tell(host, uuid, message),
            Ok(command) => match command.edit() {
                None => {
                    for line in describe(&*host, pos) {
                        net::tell(host, uuid, line);
                    }
                }
                Some(edit) => apply_edit(host, uuid, pos, &edit),
            },
        }
        Flow::Cancel
    }
}

fn world_track() -> &'static str {
    create_kinetics::rules::trains::track_block::TRACK
}

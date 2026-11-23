//! Making a train at a station and taking it apart again: the second half of `StationBlockEntity.assemble` and
//! `Train.disassemble` (content/trains/station/StationBlockEntity.java, entity/Train.java), over the host.
//! `track_station.rs` does the first half (the bogeys, the wheel points on the graph); the rules are
//! `create_kinetics::rules::trains::{assembly, carriage_contraption, railways}`.
//!
//! # Assembling
//!
//! Each carriage is what `Contraption.searchMovedStructure` finds from its bogey's block (glue, slime, the bogey's
//! sticky sides: `rules::contraption::search`), summed up by `carriage_contraption::summarize` (bogeys, controls,
//! conductor seats). `assemble_train` checks the carriages in order and makes the `Train`. Only then the blocks leave
//! the world (`removeBlocksFromWorld`, as the pistons and bearings take theirs) and are kept in the carriages' runtime
//! (`trains.rs`), drawn from an anchor block entity (`train_draw.rs`). A failed assembly changes nothing.
//!
//! # Disassembling
//!
//! `Railways::disassemble_train` says where each carriage goes (`canDisassemble`: every carriage level and square to
//! the track); each is set down by `place_in_world` turned by its yaw (`makeStructureTransform`: `-yaw +
//! initialYaw`), the station remembers the train's name for the next assembly (`lastDisassembledTrainName`), and the
//! train's schedule is given back (`dropSchedule`).
//!
//! Differences from Create: the cargo (`TrainCargoManager`) and the seats' passengers aren't carried over, the
//! bogeys' data (`bogeyData`, the style) is empty (only the standard style exists), no advancement.

use std::collections::{BTreeMap, HashSet};

use create_kinetics::rules::contraption::{self as rules, Config, State, Transform};
use create_kinetics::rules::trains::assembly::{AssembledTrain, AssemblyInput, ContraptionResult, SecondBogeyFound, assemble_train};
use create_kinetics::rules::trains::carriage_contraption as cc;
use create_kinetics::rules::trains::carriage_pose as pose;
use create_kinetics::rules::trains::graph::{EdgePointType, Uuid};
use create_kinetics::rules::trains::station::AssemblyError;
use create_kinetics::rules::trains::travelling_point::TravellingPoint;
use create_kinetics::{Axis, Dir, Pos};
use crate::compat::{Hooks, Host, Tag};

use super::contraption as engine;
use super::track_station::{self as station, Assembly};
use super::train_net as net;
use super::trains::{self, CarriageRt};
use super::track as world;
use crate::util;

struct Assemble;

/// Nothing is hooked here: `track_station.rs` calls [`finish`] and [`disassemble`].
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Assemble)
}

impl Hooks for Assemble {}

const LAST_NAME: &str = "LastTrainName";
const LAST_COLOR: &str = "LastTrainColor";

/// A carriage found by a search, kept until the assembly is known to work.
struct Found {
    start: Pos,
    blocks: Vec<(Pos, State)>,
    summary: cc::Summary,
}

fn bogey_pos(assembly: &Assembly, index: usize) -> Pos {
    let scan = &assembly.scan;
    let along = station::offset(assembly.target, assembly.direction, scan.bogey_locations[index] + 1);
    let sign = if scan.upside_down[index] { -1 } else { 1 };
    [along[0] + assembly.up[0], along[1] + assembly.up[1] * sign, along[2] + assembly.up[2]]
}

fn say(host: &mut dyn Host, uuid: u128, error: &AssemblyError) {
    if let AssemblyError::Structure { message } = error {
        net::tell(host, uuid, message.clone());
        return;
    }
    let (key, args) = create_kinetics::rules::trains::edge_blocks::assembly_error_message(error);
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    net::say(host, uuid, &key, &args);
}

/// The rest of `StationBlockEntity.assemble` once the wheel points are on the graph: gathers the carriages, makes the
/// train and takes the blocks out of the world. True when a train was made.
pub(super) fn finish(
    host: &mut dyn Host,
    uuid: u128,
    station_pos: Pos,
    assembly: &Assembly,
    station_id: Uuid,
    graph: Uuid,
    points: Vec<TravellingPoint>,
) -> bool {
    let dimension = host.dimension();
    let Some(direction) = Dir::from_normal(assembly.direction) else { return false };
    let scan = &assembly.scan;
    let count = scan.bogey_count();
    engine::prepare(host);

    // The carriages' blocks, found one bogey at a time.
    let mut found: Vec<Found> = Vec::new();
    let last = host.mod_block_entity(station_pos).and_then(|e| {
        let name = match util::get(&e.nbt, LAST_NAME) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        }?;
        Some((name, util::int(&e.nbt, LAST_COLOR)))
    });
    let input = AssemblyInput {
        scan,
        points,
        graph,
        train_id: net::new_id(),
        player: Some(uuid),
        last_disassembled: last,
        // `Carriage.netIdGenerator`: ids are unique across trains, so the glue finds a carriage by its id.
        first_carriage_id: (util::random() & 0x3fff_ffff) as i32 + 1,
    };
    let result = {
        let host_ref: &dyn Host = &*host;
        let Some(world_view) = engine::HostWorld::new(host_ref) else { return false };
        let mut assemble_contraption = |index: usize, _offset: i32| -> Result<ContraptionResult, AssemblyError> {
            let start = bogey_pos(assembly, index);
            let anchoring = |_: Pos| false;
            let config = Config { forced: None, anchoring: &anchoring, frontier: vec![start], preexisting: 0, known: HashSet::new() };
            let searched = rules::search(&world_view, &config).map_err(|fail| AssemblyError::Structure { message: fail.message() })?;
            let summary = cc::summarize(&searched.blocks, direction);
            let ok = cc::verdict(searched.blocks.len(), &summary)?;
            let second_expected = (index + 1 < count).then(|| bogey_pos(assembly, index + 1));
            let second_bogey = summary
                .second_bogey
                .map(|p| SecondBogeyFound { matches_next_bogey: Some(p) == second_expected, data: Vec::new() });
            let out = ContraptionResult {
                success: ok,
                has_forward_controls: summary.forward_controls,
                has_backward_controls: summary.backward_controls,
                first_bogey_data: Vec::new(),
                second_bogey,
            };
            found.push(Found { start, blocks: searched.blocks, summary });
            Ok(out)
        };
        let mut derail_angle = || (util::random() % 60) as i32 - 30;
        assemble_train(input, &mut assemble_contraption, &mut derail_angle)
    };
    let AssembledTrain { mut train, bogey_indices } = match result {
        Ok(assembled) => assembled,
        Err(failure) => {
            say(host, uuid, &failure.error);
            return false;
        }
    };

    // The train is made: the blocks leave the world.
    let mut carriages: Vec<(i32, CarriageRt)> = Vec::new();
    for (i, carriage) in train.carriages.iter().enumerate() {
        let _ = &bogey_indices[i];
        // A carriage of two bogeys was found from its first bogey.
        let Some(f) = found.iter().find(|f| f.start == bogey_pos(assembly, bogey_indices[i].0)) else { continue };
        let anchor = f.start;
        let mut moving = engine::capture(host, anchor, &f.blocks, &[], &|p| cc::local(p, anchor));
        moving.anchor = anchor;
        engine::remove_from_world(host, &moving, &|local| [anchor[0] + local[0], anchor[1] + local[1], anchor[2] + local[2]], &mut |_, _, _| false);
        carriages.push((carriage.id, CarriageRt::new(moving, direction, &f.summary)));
    }
    let carriage_count = carriages.len();
    trains::with_dim(dimension, |d| {
        for (id, rt) in carriages {
            d.add_carriage(train.id, id, rt);
        }
    });
    if let Some(Tag::String(_)) = host.mod_block_entity(station_pos).and_then(|e| util::get(&e.nbt, LAST_NAME).cloned()) {
        host.change_mod_block_entity(station_pos, &mut |e| {
            util::remove(&mut e.nbt, LAST_NAME);
            util::remove(&mut e.nbt, LAST_COLOR);
        });
    }
    train.owner = Some(uuid);
    world::with_railways(dimension, |rails| rails.add_assembled_train(train, Some(station_id)));
    net::tell(host, uuid, format!("Train assembled: {carriage_count} carriage{}", if carriage_count == 1 { "" } else { "s" }));
    true
}

/// `Train.disassemble` for the train standing at the station: each carriage back into blocks. `false` when it
/// can't (a carriage isn't level or square to the track).
pub(super) fn disassemble(host: &mut dyn Host, uuid: Option<u128>, station_pos: Pos) -> bool {
    let dimension = host.dimension();
    let Some(entity) = host.mod_block_entity(station_pos) else { return false };
    let target = net::load(&entity.nbt, EdgePointType::Station, station_pos);
    let Some(train_id) = trains::train_at(dimension, target.id) else {
        if let Some(uuid) = uuid {
            net::tell(host, uuid, "No train is standing at this station");
        }
        return false;
    };
    let Some(assembly) = station::refresh(&*host, &target, station_pos) else { return false };
    let above = [assembly.target[0] + assembly.up[0], assembly.target[1] + assembly.up[1], assembly.target[2] + assembly.up[2]];
    // `(pitch, yaw)` of each carriage, as `canDisassemble` asks.
    let poses: BTreeMap<i32, (f32, f32)> = trains::carriage_poses(dimension, train_id);
    let entity_of = |c: &create_kinetics::rules::trains::carriage::Carriage| poses.get(&c.id).copied();
    let plan = world::with_railways(dimension, |rails| rails.disassemble_train(train_id, assembly.direction, above, &entity_of)).flatten();
    let Some(plan) = plan else {
        if let Some(uuid) = uuid {
            net::tell(host, uuid, "The train can't be disassembled: its carriages aren't level or square to the track");
        }
        return false;
    };
    // The schedule goes back to the player or the station (`dropSchedule`) before the train is forgotten.
    for (_, carriage_id, at) in &plan.placements {
        let Some(rt) = trains::with_dim(dimension, |d| d.take_carriage(train_id, *carriage_id)).flatten() else { continue };
        let yaw = rt.last_pose.map_or(pose::initial_yaw(trains::horizontal_step(rt.direction)), |p| p.yaw);
        let initial = pose::initial_yaw(trains::horizontal_step(rt.direction));
        let transform = Transform::from_angle(*at, Axis::Y, -yaw + initial);
        if let Some(anchor) = rt.anchor {
            trains::remove_anchor_block(host, anchor);
        }
        engine::place_in_world(host, &rt.structure, &transform, &mut |_, _, _| false);
    }
    trains::forget_train(dimension, train_id);
    // `dropSchedule`: the Schedule item goes to the player's hands or hangs at the station.
    if let Some(schedule) = &plan.schedule {
        if let Some(stack) = super::schedule::stack_with(&*host, schedule) {
            let given = uuid.filter(|u| host.held_stack(*u).is_none()).and_then(|u| host.give_stack(u, stack.clone()));
            // `give_stack` hands back what didn't fit.
            if uuid.is_none() || host.held_stack(uuid.unwrap_or(0)).is_some() || given.is_some() {
                util::spawn_item(host, util::centre(station_pos), &stack, [0.0; 3], 10);
            }
        }
    }
    host.change_mod_block_entity(station_pos, &mut |e| {
        e.nbt.insert(LAST_NAME.into(), Tag::String(plan.train_name.as_str().into()));
        e.nbt.insert(LAST_COLOR.into(), Tag::Int(plan.map_color_index));
    });
    true
}

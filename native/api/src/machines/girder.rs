//! Metal Girders and the Metal Girder Encased Shaft (`GirderBlock`, `GirderEncasedShaftBlock`,
//! `GirderWrenchBehavior`, `GirderPlacementHelper`, `ShaftBlock.useItemOn`). Every rule is in
//! `create_kinetics::rules::girder`; this reads the world, asks the rules and writes what they say.
//! The header there lists what a girder cannot see of its neighbours.
//!
//! Known differences from Create, for the README:
//! - Waterlogging is left out (as for most blocks of this mod).
//! - A girder's connections are worked out from neighbours' block state ids only (Horizon's shape
//!   updates hand a mod no more): see the rules' header for what that leaves out (nixie tubes,
//!   placards, brackets, tracks; any block that isn't air, fluid or a girder counts as solid).
//! - The Wrench's highlight box (`GirderWrenchBehavior.tick`) is not drawn, and the point the Wrench
//!   clicked is worked out from the player's look (a hook doesn't carry it).
//! - The Extendo Grip's longer placement-assist range isn't there.
//! - The encased shaft carries rotation like an Encased Shaft: no stress of its own
//!   (`BlockStressValues.getImpact` of an unregistered block is 0).

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::girder::{self as rules, ENCASED_SHAFT, GIRDER, Face, Side, State};
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack, LevelHost,
    LevelTicksHost, StateForPlacementEvent, UpdateShapeEvent, UseItemOnEvent,
};

use crate::machines::encased;
use crate::{kinetic, util};

const SHAFT: &str = "create:shaft";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `Block.UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE`, as `GirderWrenchBehavior` writes (`2 | 16`).
const QUIET: i32 = 2 | 16;

struct Girders;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Girders)
}

// --- reading the world -------------------------------------------------------------------------

/// What a block state is to a girder, by state id: the answer never changes for a state, so it
/// is kept.
static SIDES: Mutex<Option<HashMap<u32, Side>>> = Mutex::new(None);

fn is(level: &dyn LevelHost, id: BlockStateId, key: &str, value: &str) -> bool {
    level.state_with_property(id, key, value) == Some(id)
}

fn has(level: &dyn LevelHost, id: BlockStateId, key: &str) -> bool {
    ["true", "false", "x", "floor", "0"]
        .iter()
        .any(|v| level.state_with_property(id, key, v).is_some())
}

/// A girder or encased shaft's state, when `id` is one: they are the blocks with `top`, `bottom`
/// and an `axis`, a girder with `x` and `z` as well.
fn girder_state(level: &dyn LevelHost, id: BlockStateId) -> Option<State> {
    if level.state_with_property(id, "top", "true").is_none()
        || level.state_with_property(id, "bottom", "true").is_none()
    {
        return None;
    }
    let axis = Axis::ALL.into_iter().find(|a| is(level, id, "axis", a.name()))?;
    let encased = level.state_with_property(id, "x", "true").is_none();
    Some(State {
        encased,
        x: is(level, id, "x", "true"),
        z: is(level, id, "z", "true"),
        top: is(level, id, "top", "true"),
        bottom: is(level, id, "bottom", "true"),
        axis,
    })
}

fn classify(level: &dyn LevelHost, id: BlockStateId) -> Side {
    let girder = girder_state(level, id);
    let face = if has(level, id, "face") {
        [("floor", Face::Floor), ("wall", Face::Wall), ("ceiling", Face::Ceiling)]
            .into_iter()
            .find(|(name, _)| is(level, id, "face", name))
            .map(|(_, face)| face)
    } else {
        None
    };
    Side {
        // Air is the first state of the registry.
        air: id.0 == 0,
        // A fluid block has a `level` and nothing else of a girder's interest.
        fluid: girder.is_none() && has(level, id, "level"),
        girder,
        up: is(level, id, "up", "true"),
        hanging: has(level, id, "hanging").then(|| is(level, id, "hanging", "true")),
        chain_y: girder.is_none() && is(level, id, "axis", "y") && has(level, id, "waterlogged"),
        face,
    }
}

fn side_of(level: &dyn LevelHost, id: BlockStateId) -> Side {
    if let Some(side) = SIDES.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&id.0).copied())) {
        return side;
    }
    let side = classify(level, id);
    if let Ok(mut guard) = SIDES.lock() {
        guard.get_or_insert_with(HashMap::new).insert(id.0, side);
    }
    side
}

fn side_at(level: &dyn LevelHost, pos: Pos, d: Dir) -> Side {
    side_of(level, level.block_state(util::add(pos, d.normal())))
}

/// `state` with the girder properties of `new` set.
fn apply(level: &dyn LevelHost, base: BlockStateId, new: &State) -> Option<BlockStateId> {
    let mut id = base;
    for (key, value) in new.properties() {
        id = level.state_with_property(id, key, value)?;
    }
    Some(id)
}

// --- reading and writing a placed block ----------------------------------------------------------

fn read(host: &dyn Host, pos: Pos) -> Option<State> {
    let block = host.block(pos);
    let encased = block == ENCASED_SHAFT;
    if !encased && block != GIRDER {
        return None;
    }
    let on = |key: &str| host.block_property(pos, key).as_deref() == Some("true");
    let axis = host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)?;
    Some(State { encased, x: on("x"), z: on("z"), top: on("top"), bottom: on("bottom"), axis })
}

/// Writes `state` to the block at `pos` (the block `block`), a property at a time; only the last
/// write tells the neighbours, with `last`, the ones before it with `QUIET`.
fn write(host: &mut dyn Host, pos: Pos, state: &State, quiet: bool) {
    let wanted: Vec<_> = state
        .properties()
        .into_iter()
        .filter(|(key, value)| host.block_property(pos, key).as_deref() != Some(*value))
        .collect();
    let last = wanted.len().saturating_sub(1);
    for (i, (key, value)) in wanted.into_iter().enumerate() {
        let flags = if quiet && i < last { QUIET } else { util::UPDATE_ALL };
        host.set_block_property_with(pos, key, value, flags);
    }
}

/// What the world around `pos` is, for the rules.
fn sides(host: &dyn Host, pos: Pos) -> impl Fn(Dir) -> Side + '_ {
    move |d| {
        let at = util::add(pos, d.normal());
        let block = host.block(at);
        let mut side = Side { air: block == "minecraft:air", ..Side::default() };
        if let Some(girder) = read(host, at) {
            side.girder = Some(girder);
        }
        side.fluid = matches!(block.as_str(), "minecraft:water" | "minecraft:lava");
        let property = |key: &str| host.block_property(at, key);
        side.up = property("up").as_deref() == Some("true");
        side.hanging = property("hanging").map(|v| v == "true");
        side.chain_y = side.girder.is_none()
            && property("axis").as_deref() == Some("y")
            && property("waterlogged").is_some();
        side.face = property("face").and_then(|v| match v.as_str() {
            "floor" => Some(Face::Floor),
            "wall" => Some(Face::Wall),
            "ceiling" => Some(Face::Ceiling),
            _ => None,
        });
        side
    }
}

/// `BlockState.canBeReplaced` of the block at `pos`, as far as the world says: air and fluids.
fn replaceable(host: &dyn Host, pos: Pos) -> bool {
    matches!(host.block(pos).as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "minecraft:water" | "minecraft:lava")
}

/// Puts the block `block` with `state` where `pos` is.
fn place(host: &mut dyn Host, pos: Pos, block: &str, state: &State) -> bool {
    if host.block(pos) != block {
        if !encased::switch_block(host, pos, block, &[]) {
            return false;
        }
    }
    write(host, pos, state, true);
    true
}

fn consume(host: &mut dyn Host, item: &str) {
    if host.acting_player().is_some_and(|p| p.game_mode != util::CREATIVE) {
        host.take_items(item, 1);
    }
}

// --- the hooks ---------------------------------------------------------------------------------

impl Hooks for Girders {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ENCASED_SHAFT).then(|| (ENCASED_SHAFT.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ENCASED_SHAFT {
            event.ticks = true;
        }
    }

    /// `HorizontalAxisKineticBlock`: a shaft along its `axis`, both ends.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ENCASED_SHAFT {
            return;
        }
        let Some(axis) = host.block_property(event.pos, "axis").as_deref().and_then(Axis::from_name) else {
            return;
        };
        let Some((_, changed)) = kinetic::tick(host, event.pos, event.entity, Node::passive(Kind::Shaft(axis))) else {
            return;
        };
        if changed {
            host.block_entity_sync(event.pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ENCASED_SHAFT).then(|| kinetic::update_tag(event.entity))
    }

    /// `ShaftRenderer` (the entity type is `ENCASED_SHAFT`'s).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ENCASED_SHAFT {
            return;
        }
        let Some(axis) = host.block_state_property(event.state, "axis").and_then(Axis::from_name) else {
            return;
        };
        encased::draw_shaft(host, &mut *event.draw, axis, event.pos, kinetic::speed(event.entity));
        event.draw.keep();
    }

    /// `GirderBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if context.item != GIRDER {
            return None;
        }
        let pos = [context.pos.x, context.pos.y, context.pos.z];
        let state = rules::for_placement(util::dir(context.face).axis(), &|d| side_at(level, pos, d));
        apply(level, event.state, &state).map(Some)
    }

    /// `GirderBlock.updateShape`, `GirderEncasedShaftBlock.updateShape`.
    fn update_shape(&self, level: &mut dyn LevelTicksHost, event: &mut UpdateShapeEvent) -> Option<BlockStateId> {
        let level: &dyn LevelHost = &*level;
        let state = girder_state(level, event.state)?;
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let new = rules::update_shape(state, util::dir(event.direction), &|d| side_at(level, pos, d));
        if new == state {
            return Some(event.state);
        }
        apply(level, event.state, &new)
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let item = event.stack.item.to_string();
        let block = host.block(pos);
        let mine = block == GIRDER || block == ENCASED_SHAFT;
        let relevant = (mine && (item == util::WRENCH || item == SHAFT || item == GIRDER))
            || (block == SHAFT && item == GIRDER);
        if !relevant {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        // `IWrenchable.onSneakWrenched`.
        if item == util::WRENCH && player.sneaking {
            return util::wrench(host, pos, |_| {});
        }
        if player.sneaking {
            return Flow::Continue;
        }
        let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
        let hit = rules::hit_from_centre(eye, player.yaw, player.pitch, pos, face);
        match (block.as_str(), item.as_str()) {
            (GIRDER, SHAFT) => {
                let Some(state) = read(host, pos) else {
                    return Flow::Continue;
                };
                let encased = rules::encase_girder(&state, face.axis());
                place(host, pos, ENCASED_SHAFT, &encased);
                host.play_sound("minecraft:block.netherite_block.hit", util::centre(pos), BLOCKS, 0.5, 1.25);
                consume(host, SHAFT);
                Flow::Cancel
            }
            (SHAFT, GIRDER) => {
                let axis = host.block_property(pos, "axis").as_deref().and_then(Axis::from_name);
                let Some(encased) = axis.and_then(rules::encase_shaft) else {
                    return Flow::Continue;
                };
                place(host, pos, ENCASED_SHAFT, &encased);
                consume(host, GIRDER);
                Flow::Cancel
            }
            (_, WRENCH_ITEM) => wrench(host, pos, &block, hit),
            (_, GIRDER) => extend(host, pos, hit),
            _ => Flow::Continue,
        }
    }
}

// The match above compares against this as a pattern.
const WRENCH_ITEM: &str = util::WRENCH;

/// `GirderBlock.useItemOn` and `GirderEncasedShaftBlock.onWrenched` with the Wrench.
fn wrench(host: &mut dyn Host, pos: Pos, block: &str, hit: [f64; 3]) -> Flow {
    let Some(state) = read(host, pos) else {
        return Flow::Continue;
    };
    if block == ENCASED_SHAFT {
        // `IWrenchable.onWrenched`: the girder with the shaft given back.
        place(host, pos, GIRDER, &rules::girder_of(&state));
        host.play_sound("minecraft:block.netherite_block.hit", util::centre(pos), BLOCKS, 0.5, 1.0);
        if let Some(player) = host.acting_player()
            && player.game_mode != util::CREATIVE
            && let Some(rest) = host.give_stack(player.uuid, ItemStack::new(SHAFT, 1))
        {
            util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
        }
        return Flow::Cancel;
    }
    let above = read(host, util::add(pos, [0, 1, 0]));
    let below = read(host, util::add(pos, [0, -1, 0]));
    // `GirderWrenchBehavior.handleClick`: only girders count as the other block (`METAL_GIRDER.has`).
    let girders = |s: Option<State>| s.filter(|s| !s.encased);
    if let Some(change) = rules::wrench_click(&state, girders(above), girders(below), hit) {
        write(host, pos, &change.this, false);
        if let Some((d, other)) = change.other {
            write(host, util::add(pos, d.normal()), &other, false);
        }
    }
    Flow::Cancel
}

/// `GirderPlacementHelper.getOffset(...).placeInWorld`: a girder item clicked on a girder goes at
/// the end of the line.
fn extend(host: &mut dyn Host, pos: Pos, hit: [f64; 3]) -> Flow {
    let Some(state) = read(host, pos) else {
        return Flow::Continue;
    };
    let found = {
        let world = &*host;
        rules::extension(&state, pos, hit, &|p| read(world, p), &|p| replaceable(world, p))
    };
    let Some((at, axis)) = found else {
        return Flow::Continue;
    };
    // `getStateForPlacement` then `withAxis`, then `Block.updateFromNeighbourShapes`.
    let mut new = rules::with_axis(&State::GIRDER, axis);
    {
        let world = &*host;
        new = rules::settle(new, &sides(world, at));
    }
    if !place(host, at, GIRDER, &new) {
        return Flow::Continue;
    }
    host.play_sound("minecraft:block.netherite_block.place", util::centre(at), BLOCKS, 1.0, 0.8);
    consume(host, GIRDER);
    Flow::Cancel
}

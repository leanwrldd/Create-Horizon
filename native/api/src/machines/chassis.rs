//! The Linear Chassis, the Secondary Linear Chassis and the Radial Chassis (`LinearChassisBlock`,
//! `RadialChassisBlock`, `AbstractChassisBlock`, `ChassisBlockEntity`). A structure that gathers
//! one (the piston, the bearing, and every contraption after them) takes the blocks the chassis
//! holds on to: the engine asks `contraption::chassis_hook`, which reads the range saved here.
//! Every rule is in `create_kinetics::rules::chassis`; this reads the world, asks the rules and
//! writes what they say.
//!
//! A slime ball (or Super Glue) on a side makes it sticky, a sneaking empty hand removes the glue
//! (`AbstractChassisBlock.useItemOn`). The Wrench steps the range.
//!
//! Known differences from Create, for the README:
//! - The range is set with the Wrench, not the scroll value box: a click steps it by one, 16 back
//!   to 1, and the new value shows on the action bar. Each chassis keeps its own range
//!   (`BulkScrollValueBehaviour` sets the whole group at once). The range outline
//!   (`ChassisRangeDisplay`) isn't drawn.
//! - The Wrench doesn't turn a chassis (`IWrenchable` default): pick one up and place it again;
//!   it is placed along the look or the neighbour (`LinearChassisBlock.getStateForPlacement`).
//! - No waterlogging, no connected textures (the chassis draws its plain model).

use create_kinetics::rules::chassis::{self as rules, DEFAULT_RANGE, RANGE_KEY};
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::funnel::nearest_looking;
use create_kinetics::{Axis, Dir, Pos};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack, LevelHost, LevelTicksHost,
    RightClickBlockEvent, StateForPlacementEvent, Tag, Text, UpdateShapeEvent, UseItemOnEvent,
};

use crate::util;

struct Chassis;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Chassis)
}

/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(RANGE_KEY.into(), Tag::Int(DEFAULT_RANGE));
    nbt
}

/// The block state at `pos` as the rules read it.
fn state_at(host: &dyn Host, pos: Pos) -> Option<State> {
    let level = host.level(host.dimension())?;
    level.block_state(host, pos).map(|text| State::parse(&text))
}

/// A neighbour as `glueAllowedOnSide` wants it: its name and axis if it is a chassis.
fn neighbour(host: &dyn Host, pos: Pos) -> Option<(String, Axis)> {
    let state = state_at(host, pos)?;
    rules::is_chassis(&state.name).then(|| {
        let axis = rules::axis_of(&state);
        (state.name, axis)
    })
}

/// `ChassisBlockEntity.getRange` of the chassis at `pos`; none if the block there isn't one.
pub fn range_at(host: &dyn Host, pos: Pos) -> Option<i32> {
    if !rules::is_chassis(&host.block(pos)) {
        return None;
    }
    let saved = host
        .mod_block_entity(pos)
        .map(|entity| util::int(&entity.nbt, RANGE_KEY))
        .filter(|range| *range > 0);
    Some(saved.unwrap_or(DEFAULT_RANGE))
}

/// Applies the sides `use_item` chose, with the slime's sound.
fn apply(host: &mut dyn Host, pos: Pos, changes: &[(&'static str, bool)]) {
    for (key, on) in changes {
        host.set_block_property(pos, key, if *on { "true" } else { "false" });
    }
    if !changes.is_empty() {
        host.play_sound("minecraft:block.slime_block.place", util::centre(pos), BLOCKS, 0.5, 1.0);
    }
}

/// `AbstractChassisBlock.useItemOn` for the block at `pos` clicked on `face`. `Some(changed)` when
/// the chassis took the click.
fn glue_click(
    host: &mut dyn Host,
    pos: Pos,
    face: Dir,
    glue_item: bool,
    empty: bool,
    sneaking: bool,
    repeat_guard: Option<u128>,
) -> bool {
    let Some(state) = state_at(host, pos) else {
        return false;
    };
    if !rules::is_chassis(&state.name) {
        return false;
    }
    let allowed = |side: Dir| {
        let other = neighbour(host, util::add(pos, side.normal()));
        rules::glue_allowed(&state, other.as_ref().map(|(name, axis)| (name.as_str(), *axis)))
    };
    let Some(changes) = rules::use_item(&state, face, glue_item, empty, sneaking, &allowed) else {
        return false;
    };
    if let Some(who) = repeat_guard {
        // The off hand's turn of a click that was taken: do nothing, but still take it.
        if !util::first_click(host, who, pos) {
            return true;
        }
    }
    apply(host, pos, &changes);
    true
}

impl Hooks for Chassis {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        rules::is_chassis(event.block).then(|| (event.block.to_owned(), new_entity()))
    }

    /// `LinearChassisBlock.getStateForPlacement` and the pillar placement of the radial one.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if !rules::is_chassis(item) {
            return None;
        }
        let face = util::dir(context.face);
        // The block the new one is placed against, if it is a linear chassis (it has the sticky
        // ends; any axis of one is read from its state).
        let against = [context.pos.x, context.pos.y, context.pos.z];
        let n = face.normal();
        let on = level.block_state([against[0] - n[0], against[1] - n[1], against[2] - n[2]]);
        let placed_on = level.state_with_property(on, "sticky_top", "true").and_then(|_| {
            Axis::ALL
                .into_iter()
                .find(|axis| level.state_with_property(on, "axis", axis.name()) == Some(on))
        });
        let nearest = nearest_looking(context.yaw, context.pitch)[0];
        let axis = rules::placement_axis(item == rules::RADIAL, placed_on, nearest, face, context.sneaking);
        level
            .state_with_property(event.state, "axis", axis.name())
            .map(Some)
    }

    /// `LinearChassisBlock.updateShape`: two chassis of one kind and axis joined lose the glue
    /// between them.
    fn update_shape(&self, level: &mut dyn LevelTicksHost, event: &mut UpdateShapeEvent) -> Option<BlockStateId> {
        let level: &dyn LevelHost = &*level;
        let own = event.state;
        level.state_with_property(own, "sticky_top", "true")?;
        let flag = |key: &str| level.state_with_property(own, key, "true") == Some(own);
        let axis = Axis::ALL
            .into_iter()
            .find(|axis| level.state_with_property(own, "axis", axis.name()) == Some(own))?;
        let state = State::new(rules::LINEAR)
            .with("axis", axis.name())
            .with("sticky_top", if flag("sticky_top") { "true" } else { "false" })
            .with("sticky_bottom", if flag("sticky_bottom") { "true" } else { "false" });
        // The neighbour is the same kind when it is one of this block's own states.
        let mut other = None;
        for a in Axis::ALL {
            for top in ["false", "true"] {
                for bottom in ["false", "true"] {
                    let candidate = level
                        .state_with_property(own, "axis", a.name())
                        .and_then(|s| level.state_with_property(s, "sticky_top", top))
                        .and_then(|s| level.state_with_property(s, "sticky_bottom", bottom));
                    if candidate == Some(event.neighbour) {
                        other = Some(a);
                    }
                }
            }
        }
        let side = util::dir(event.direction);
        let property = rules::update_shape(&state, side, other.map(|axis| (rules::LINEAR, axis)))?;
        level.state_with_property(own, property, "false")
    }

    /// A slime ball or Super Glue on a side, and the Wrench stepping the range.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if !rules::is_chassis(&host.block(pos)) {
            return Flow::Continue;
        }
        let item: &str = &event.stack.item;
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                let Some(current) = range_at(host, pos) else { return };
                let next = rules::step_range(current);
                host.change_mod_block_entity(pos, &mut |entity| {
                    entity.nbt.insert(RANGE_KEY.into(), Tag::Int(next));
                });
                let radial = host.block(pos) == rules::RADIAL;
                let line = format!("{}: {next}", if radial { "Radius when Sticky" } else { "Range of Sticky Sides" });
                host.action_bar(player.uuid, Text::literal(line));
            });
        }
        if item != rules::SLIME_BALL && item != rules::SUPER_GLUE {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        if glue_click(host, pos, face, true, false, player.sneaking, None) {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// A sneaking empty hand takes the glue off a side.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !rules::is_chassis(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !player.sneaking || player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        let held: Option<ItemStack> = host.held_stack(player.uuid);
        if held.is_some_and(|stack| stack.count > 0) {
            return Flow::Continue;
        }
        let Some(face) = Dir::from_normal(event.face) else {
            return Flow::Continue;
        };
        if glue_click(host, pos, face, false, true, true, Some(player.uuid)) {
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

//! The Copper Valve Handle and the dyed valve handles (`create:copper_valve_handle`,
//! `create:<colour>_valve_handle`; `ValveHandleBlock`, `ValveHandleBlockEntity`,
//! `HandCrankBlockEntity`): a hand crank that turns a shaft behind it a set angle at 32 RPM when
//! clicked, for a Fluid Valve to open or close. The numbers are `create_kinetics::rules::fluid_valve`.
//!
//! Known differences from Create: see `rules/fluid_valve.rs` (the angle is stepped by a Wrench click,
//! not a value box). Also:
//! - The handle lines up with a shaft beside it only through `place_block` of the hand crank
//!   (not done here): it faces away from the face clicked (`HandCrankBlock.getStateForPlacement`
//!   without `getPreferredFacing`).
//! - `canSurvive` (the block behind must have a collision shape) is checked every ten ticks.
//! - A handle doesn't take food (`HandCrankBlock.useItemOn` does for the plain crank; the valve
//!   handle's `clicked` doesn't), and has no waterlogging or ponder.

use create_kinetics::generators::crank_speed;
use create_kinetics::rules::fluid_valve::{self as rules, Handle};
use create_kinetics::{Dir, Kind, Node};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, EquipmentSlot, Flow, Hooks, Host, LevelHost,
    RightClickBlockEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use crate::{int, kinetic, util};

const CENTRE: [f32; 3] = [0.5; 3];
/// `HandCrankBlockEntity.write` and `ValveHandleBlockEntity.write`.
const IN_USE: &str = "InUse";
const BACKWARDS: &str = "Backwards";
const TOTAL_USE_TICKS: &str = "TotalUseTicks";
const START_ANGLE: &str = "StartAngle";
const TARGET_ANGLE: &str = "TargetAngle";
/// The scroll value's key (`ScrollValueBehaviour.write`).
const ANGLE: &str = "ScrollValue";
/// Not Create's: the tick the use started, for a client to time the handle's turn by.
const SINCE: &str = "Since";
/// How often the block behind is checked (`HandCrankBlock.canSurvive`).
const SURVIVE: i64 = 10;

/// `DyeColor`, in order.
pub const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
    "cyan", "purple", "blue", "brown", "green", "red", "black",
];

struct Handles;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Handles)
}

/// The colour of a valve handle block (`None` for the copper one), or `None` for another block.
fn colour_of(block: &str) -> Option<Option<&str>> {
    let name = block.strip_prefix("create:")?.strip_suffix("_valve_handle")?;
    if name == "copper" {
        return Some(None);
    }
    COLOURS.iter().find(|c| **c == name).map(|c| Some(*c))
}

fn is_handle(block: &str) -> bool {
    colour_of(block).is_some()
}

/// `DyeColor.getColor(ItemStack)`: the colour of a dye item.
fn dye_colour(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    COLOURS.iter().find(|c| **c == name).copied()
}

fn read(nbt: &Compound) -> Handle {
    let angle = if util::has(nbt, ANGLE) { int(nbt, ANGLE) } else { rules::DEFAULT_ANGLE };
    Handle {
        angle,
        in_use: int(nbt, IN_USE),
        cooldown: 0,
        total_use_ticks: int(nbt, TOTAL_USE_TICKS),
        start_angle: int(nbt, START_ANGLE),
        target_angle: int(nbt, TARGET_ANGLE),
        backwards: int(nbt, BACKWARDS) != 0,
    }
}

/// Writes what differs of `handle` (the cooldown isn't saved).
fn write(nbt: &mut Compound, handle: &Handle) -> bool {
    let mut changed = false;
    let mut put = |key: &str, value: Tag| {
        if util::differs(nbt, key, &value) {
            nbt.insert(key.into(), value);
            changed = true;
        }
    };
    put(ANGLE, Tag::Int(handle.angle));
    put(IN_USE, Tag::Int(handle.in_use));
    put(BACKWARDS, Tag::Byte(i8::from(handle.backwards)));
    put(TOTAL_USE_TICKS, Tag::Int(handle.total_use_ticks));
    put(START_ANGLE, Tag::Int(handle.start_angle));
    put(TARGET_ANGLE, Tag::Int(handle.target_angle));
    changed
}

fn new_entity() -> Compound {
    let mut nbt = kinetic::new_entity();
    write(&mut nbt, &Handle::default());
    nbt
}

/// Cooldowns by position: not saved, as Create's `cooldown` isn't sent.
static COOLDOWNS: std::sync::Mutex<Option<std::collections::HashMap<(String, [i32; 3]), i32>>> =
    std::sync::Mutex::new(None);

fn with_cooldowns<R>(f: impl FnOnce(&mut std::collections::HashMap<(String, [i32; 3]), i32>) -> R) -> R {
    let mut guard = COOLDOWNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(std::collections::HashMap::new))
}

/// `ValveHandleBlock.clicked` for a click on the handle at `pos`.
fn clicked(host: &mut dyn Host, pos: [i32; 3], block: &str, held: Option<&str>, sneaking: bool) {
    // A dye of another colour dyes the handle (the dye isn't used up).
    let colour = colour_of(block).flatten();
    if let Some(dye) = held.and_then(dye_colour)
        && Some(dye) != colour
    {
        let facing = host.block_property(pos, "facing");
        let dimension = host.dimension().to_owned();
        if let Some(state) = host.default_state(&format!("create:{dye}_valve_handle")) {
            host.set_block(dimension.as_str(), pos, state, util::UPDATE_ALL);
            if let Some(facing) = facing {
                host.set_block_property(pos, "facing", &facing);
            }
        }
        return;
    }
    let key = (host.dimension().to_owned(), pos);
    let cooling = with_cooldowns(|all| all.get(&key).copied().unwrap_or(0)) > 0;
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut handle = read(&entity.nbt);
    handle.cooldown = i32::from(cooling);
    if !handle.activate(kinetic::speed(&entity), sneaking) {
        return;
    }
    let since = util::game_time(&*host);
    host.change_mod_block_entity(pos, &mut |entity| {
        write(&mut entity.nbt, &handle);
        entity.nbt.insert(SINCE.into(), Tag::Long(since));
    });
    with_cooldowns(|all| all.insert(key.clone(), handle.cooldown.max(rules::COOLDOWN)));
    host.block_entity_sync(pos);
}

impl Hooks for Handles {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_handle(event.block).then(|| (event.block.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_handle(event.block) {
            event.ticks = true;
        }
    }

    /// `HandCrankBlockEntity.tick`, `ValveHandleBlockEntity.tick` and `HandCrankBlock.canSurvive`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_handle(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return;
        };
        let mut handle = read(&event.entity.nbt);
        let key = (host.dimension().to_owned(), pos);
        handle.cooldown = with_cooldowns(|all| all.get(&key).copied().unwrap_or(0));
        let node = Node {
            generated: crank_speed(handle.in_use, handle.clockwise(), facing),
            capacity: rules::CAPACITY,
            ..Node::passive(Kind::Crank(facing))
        };
        let Some((_, mut changed)) = kinetic::tick(host, pos, event.entity, node) else {
            with_cooldowns(|all| all.remove(&key));
            return;
        };
        handle.tick();
        with_cooldowns(|all| {
            if handle.cooldown > 0 {
                all.insert(key.clone(), handle.cooldown);
            } else {
                all.remove(&key);
            }
        });
        if write(&mut event.entity.nbt, &handle) {
            changed = true;
        }
        if changed {
            host.block_entity_sync(pos);
        }
        // `HandCrankBlock.neighborChanged`: the block it is attached to went.
        if (util::game_time(&*host) + i64::from(pos[0]) + i64::from(pos[2])).rem_euclid(SURVIVE) == 0 {
            let behind = util::add(pos, facing.opposite().normal());
            let dimension = host.dimension().to_owned();
            if host.chunk_is_loaded(&dimension, [behind[0] >> 4, behind[2] >> 4])
                && host.block_collision(behind).is_empty()
            {
                host.destroy_block(pos, None, true);
            }
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_handle(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = kinetic::update_tag(event.entity);
        for key in [IN_USE, BACKWARDS, TOTAL_USE_TICKS, START_ANGLE, TARGET_ANGLE, ANGLE] {
            if let Some(value) = util::get(nbt, key) {
                tag.insert(key.into(), value.clone());
            }
        }
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        Some(tag)
    }

    /// `HandCrankRenderer` for a valve handle (`getRenderedHandle` is the block itself, turned
    /// about its facing axis by `getIndependentAngle`).
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_handle(event.block) {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let handle = read(&event.entity.nbt);
        let since = util::long(&event.entity.nbt, SINCE);
        let draw = &mut *event.draw;
        let elapsed = (draw.game_time - since) as f32 + draw.partial;
        let angle = handle.visual_angle(elapsed, facing.sign());
        draw.push();
        draw.rotate_about(facing.axis().unit(), angle, CENTRE);
        host.draw_block(draw, event.block, &[("facing", facing.name())]);
        draw.pop();
        if handle.in_use == 0 {
            draw.keep();
        }
    }

    /// `HandCrankBlock.getStateForPlacement`, without the shaft beside it Create prefers: facing
    /// away from the face clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if !is_handle(&event.context.item) {
            return None;
        }
        let facing = util::dir(event.context.face);
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `ValveHandleBlock.onBlockActivated`: any click on the handle turns it (or dyes it), whatever is
    /// in hand, unless a sneaking player holds a Wrench.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if !is_handle(&block) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        // `player.mayBuild()`.
        if player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        let held = host.player_equipment(player.uuid, EquipmentSlot::MainHand);
        let item = held.as_ref().map(|s| s.item.to_string());
        // The Wrench steps the angle (and picks the handle up when sneaking).
        if item.as_deref() == Some(util::WRENCH) {
            return Flow::Continue;
        }
        clicked(host, pos, &block, item.as_deref(), player.sneaking);
        Flow::Cancel
    }

    /// The Wrench: sneak-click picks the handle up; a click steps the angle it turns by, standing in
    /// for the value box (`ValveHandleScrollValueBehaviour`, -180 to 180 in steps of 45).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || !is_handle(&host.block(event.pos)) {
            return Flow::Continue;
        }
        let pos = event.pos;
        util::wrench(host, pos, |host| {
            let Some(entity) = host.mod_block_entity(pos) else {
                return;
            };
            let mut handle = read(&entity.nbt);
            handle.angle = rules::next_angle(handle.angle);
            let angle = handle.angle;
            host.change_mod_block_entity(pos, &mut |entity| {
                write(&mut entity.nbt, &handle);
            });
            host.block_entity_sync(pos);
            if let Some(player) = host.acting_player() {
                let direction = if angle < 0 { "counterclockwise" } else { "clockwise" };
                host.action_bar(
                    player.uuid,
                    Text::literal(format!("Rotation when used: {}\u{b0} {direction}", angle.abs())),
                );
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_blocks_are_recognised_by_name() {
        assert_eq!(colour_of("create:copper_valve_handle"), Some(None));
        assert_eq!(colour_of("create:light_blue_valve_handle"), Some(Some("light_blue")));
        assert_eq!(colour_of("create:hand_crank"), None);
        assert_eq!(colour_of("create:chartreuse_valve_handle"), None);
        assert!(is_handle("create:black_valve_handle"));
    }

    #[test]
    fn a_dye_item_names_its_colour() {
        assert_eq!(dye_colour("minecraft:red_dye"), Some("red"));
        assert_eq!(dye_colour("minecraft:light_gray_dye"), Some("light_gray"));
        assert_eq!(dye_colour("minecraft:bone_meal"), None);
    }

    #[test]
    fn a_saved_handle_reads_back() {
        let mut nbt = Compound::new();
        let mut handle = Handle::default();
        handle.angle = -90;
        handle.activate(0.0, true);
        assert!(write(&mut nbt, &handle));
        let back = read(&nbt);
        assert_eq!((back.angle, back.in_use, back.backwards), (-90, handle.in_use, true));
        assert_eq!(back.target_angle, handle.target_angle);
        // Nothing to write the second time.
        assert!(!write(&mut nbt, &handle));
        // A handle with no saved angle reads Create's default.
        assert_eq!(read(&Compound::new()).angle, rules::DEFAULT_ANGLE);
    }
}

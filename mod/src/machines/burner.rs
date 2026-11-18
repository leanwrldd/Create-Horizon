//! The Blaze Burner (`BlazeBurnerBlock`, `BlazeBurnerBlockEntity`, `BlazeBurnerBlockItem`,
//! `LitBlazeBurnerBlock`, `BlazeBurnerRenderer`): `create:blaze_burner` with its `blaze` heat
//! state, fuelled by right-click, `create:lit_blaze_burner` made with flint and steel, and
//! `create:empty_blaze_burner` which captures a Blaze. The fuel and heat rules are
//! `create_kinetics::rules::burner`; the basin reads the heat from the block's state
//! (`machines/basin.rs`).
//!
//! Known differences from Create (for the README):
//! - Regular fuel is anything with a `minecraft:cooking_fuel` component (its time over a normal
//!   furnace), or, where the component can't be read, a short list of vanilla fuels; Create asks
//!   NeoForge's furnace fuel map and the deprecated `blaze_burner_fuel/regular` tag. The Blaze
//!   Cake is the superheated fuel (`create:superheated_blaze_burner_fuels`, 3200 ticks).
//! - Flint and steel, shovels and soul sand do not wear or spend anything, and an egg thrown at
//!   the burner (`BlazeBurnerHandler`) and a splash of water on a lit one are not handled (no
//!   projectile hook).
//! - Goggles on the burner (`BlazeBurnerBlock.useItemOn`), the train hat and the stock keeper's look are missing; the head always turns to the
//!   burner's `facing` rather than to the nearest player, the head's rise over a basin is instant,
//!   and the flame sprite does not scroll (`BlazeBurnerRenderer.renderShared`).
//! - Capturing a Blaze works by right-clicking the mob or a spawner that can spawn Blazes (found
//!   by reading the spawner's saved data for `minecraft:blaze`); no capture particles.
//! - The burner keeps a block entity in every state, `none` included (Create has none then).
//! - The analog output and `getCloneItemStack` are not ported.

use create_kinetics::rules::basin::HeatLevel;
use create_kinetics::rules::burner::{self as rules, Burner, FuelType, SPECIAL_BURN_TIME};
use create_kinetics::Dir;
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, Compound, EquipmentSlot, Flow, Hooks, Host, InteractEntityEvent,
    ItemStack, LevelHost, RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use crate::util;

const ID: &str = "create:blaze_burner";
const LIT: &str = "create:lit_blaze_burner";
const EMPTY_ITEM: &str = "create:empty_blaze_burner";
const BLAZE_CAKE: &str = "create:blaze_cake";
const CREATIVE_CAKE: &str = "create:creative_blaze_cake";
/// `AllEntityTags.BLAZE_BURNER_CAPTURABLE`: `data/create/tags/entity_type/blaze_burner_capturable.json`.
const CAPTURABLE: &str = "minecraft:blaze";

// `AllPartialModels.BLAZE_*` and `BLAZE_BURNER_*`.
const INERT: &str = "create:block/blaze_burner/blaze/inert";
const IDLE: &str = "create:block/blaze_burner/blaze/idle";
const ACTIVE: &str = "create:block/blaze_burner/blaze/active";
const SUPER: &str = "create:block/blaze_burner/blaze/super";
const SUPER_ACTIVE: &str = "create:block/blaze_burner/blaze/super_active";
const RODS: &str = "create:block/blaze_burner/rods_small";
const RODS_2: &str = "create:block/blaze_burner/rods_large";
const SUPER_RODS: &str = "create:block/blaze_burner/superheated_rods_small";
const SUPER_RODS_2: &str = "create:block/blaze_burner/superheated_rods_large";
pub const MODELS: [&str; 9] = [
    INERT, IDLE, ACTIVE, SUPER, SUPER_ACTIVE, RODS, RODS_2, SUPER_RODS, SUPER_RODS_2,
];
const CENTRE: [f32; 3] = [0.5; 3];
/// `LightTexture.FULL_BRIGHT` in the draw's light packing: 15 of sky and of block light.
const FULL_BRIGHT: u32 = 0xF0F0;

// Create's keys (`BlazeBurnerBlockEntity.write`).
const FUEL_LEVEL: &str = "fuelLevel";
const BURN_TIME: &str = "burnTimeRemaining";
const IS_CREATIVE: &str = "isCreative";

struct BlazeBurner;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(BlazeBurner)
}

fn read(nbt: &Compound) -> Burner {
    Burner {
        fuel: FuelType::from_ordinal(util::int(nbt, FUEL_LEVEL)),
        remaining: util::int(nbt, BURN_TIME),
        creative: util::boolean(nbt, IS_CREATIVE),
    }
}

/// Writes what differs: reaching a block entity mutably counts as a change.
fn write(entity: &mut crate::compat::ModBlockEntity, old: Burner, new: Burner) {
    if old.fuel != new.fuel {
        entity.nbt.insert(FUEL_LEVEL.into(), Tag::Int(new.fuel.ordinal()));
    }
    if old.remaining != new.remaining {
        entity.nbt.insert(BURN_TIME.into(), Tag::Int(new.remaining));
    }
    if old.creative != new.creative {
        entity.nbt.insert(IS_CREATIVE.into(), Tag::Byte(i8::from(new.creative)));
    }
}

fn heat_of(host: &dyn Host, pos: [i32; 3]) -> HeatLevel {
    host.block_property(pos, "blaze")
        .as_deref()
        .and_then(HeatLevel::from_name)
        .unwrap_or(HeatLevel::None)
}

fn set_heat(host: &mut dyn Host, pos: [i32; 3], heat: HeatLevel) {
    host.set_block_property(pos, "blaze", heat.name());
}

// --- fuel ------------------------------------------------------------------------------------------------

fn number(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(bits) => f64::from(f32::from_bits(*bits)),
        Tag::Double(bits) => f64::from_bits(*bits),
        _ => return None,
    })
}

/// A furnace's burn time for `stack` from its `minecraft:cooking_fuel` component: a number, or the
/// `minecraft:div` Horizon's items write (`left` is the time in a normal furnace).
fn component_burn_time(host: &dyn Host, stack: &ItemStack) -> Option<i32> {
    let fuel = host.component(stack, "minecraft:cooking_fuel").ok()??;
    let time = fuel.get("burn_time")?;
    number(time)
        .or_else(|| number(time.get("left")?))
        .map(|t| t as i32)
        .filter(|t| *t > 0)
}

/// Vanilla's `FuelValues` for the commonest fuels, for where the component can't be read.
fn vanilla_burn_time(item: &str) -> Option<i32> {
    let name = item.strip_prefix("minecraft:")?;
    Some(match name {
        "coal" | "charcoal" => 1600,
        "coal_block" => 16000,
        "blaze_rod" => 2400,
        "lava_bucket" => 20000,
        "dried_kelp_block" => 4000,
        "stick" => 100,
        "bamboo" => 50,
        n if n.ends_with("_planks") || n.ends_with("_log") || n.ends_with("_wood") => 300,
        _ => return None,
    })
}

/// `BlazeBurnerBlockEntity.tryUpdateFuel`'s lookup: the kind of fuel and how long it burns.
fn fuel_of(host: &dyn Host, stack: &ItemStack) -> Option<(FuelType, i32)> {
    if &*stack.item == BLAZE_CAKE {
        return Some((FuelType::Special, SPECIAL_BURN_TIME));
    }
    component_burn_time(host, stack)
        .or_else(|| vanilla_burn_time(&stack.item))
        .map(|time| (FuelType::Normal, time))
}

/// `stack.getCraftingRemainingItem()` for the fuels that leave something.
fn remainder(item: &str) -> Option<&'static str> {
    (item == "minecraft:lava_bucket").then_some("minecraft:bucket")
}

/// `BlazeBurnerBlock.tryInsert`: a player's use of an item on a burner that has a blaze.
fn try_insert(host: &mut dyn Host, pos: [i32; 3], stack: &ItemStack, creative_player: bool) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let old = read(&entity.nbt);
    let mut burner = old;
    let heat = heat_of(host, pos);
    if &*stack.item == CREATIVE_CAKE {
        let next = burner.apply_creative_fuel(heat);
        host.change_mod_block_entity(pos, &mut |e| write(e, old, burner));
        play_shoot(host, pos);
        set_heat(host, pos, next);
        return true;
    }
    let Some((fuel, time)) = fuel_of(host, stack) else {
        return false;
    };
    // `forceOverflow` for a real player; a creative player does not consume the item.
    if !burner.try_update_fuel(fuel, time, true, false) {
        return false;
    }
    if !creative_player {
        // The stack is plain, or it can't be taken from the hand here.
        if stack.extra.is_some() || host.take_items(&stack.item, 1) != 1 {
            return false;
        }
        if let Some(left) = remainder(&stack.item) {
            host.give_items(left, 1);
        }
    }
    host.change_mod_block_entity(pos, &mut |e| write(e, old, burner));
    play_shoot(host, pos);
    let next = burner.heat_level();
    if next != heat {
        set_heat(host, pos, next);
        host.play_sound(
            "minecraft:entity.blaze.ambient",
            util::centre(pos),
            4,
            0.125 + (util::random() % 1000) as f32 / 8000.0,
            1.0,
        );
    }
    true
}

/// What the Mechanical Arm's `BlazeBurnerPoint.insert` asks of `BlazeBurnerBlock.tryInsert(.., false,
/// false, simulate)` (no player: normal fuel doesn't overflow): `(taken, creative cake, container)`,
/// the container being what the fuel leaves (an empty bucket). Unless `simulate` the fuel is applied.
pub(crate) fn feed_from_arm(
    host: &mut dyn Host,
    pos: [i32; 3],
    stack: &ItemStack,
    simulate: bool,
) -> (bool, bool, Option<&'static str>) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return (false, false, None);
    };
    let old = read(&entity.nbt);
    let mut burner = old;
    let heat = heat_of(host, pos);
    if &*stack.item == CREATIVE_CAKE {
        if !simulate {
            let next = burner.apply_creative_fuel(heat);
            host.change_mod_block_entity(pos, &mut |e| write(e, old, burner));
            play_shoot(host, pos);
            set_heat(host, pos, next);
        }
        return (true, true, None);
    }
    let Some((fuel, time)) = fuel_of(host, stack) else {
        return (false, false, None);
    };
    if !burner.try_update_fuel(fuel, time, false, simulate) {
        return (false, false, None);
    }
    if !simulate {
        host.change_mod_block_entity(pos, &mut |e| write(e, old, burner));
        play_shoot(host, pos);
        let next = burner.heat_level();
        if next != heat {
            set_heat(host, pos, next);
        }
    }
    (true, false, remainder(&stack.item))
}

/// `playSound`: `BLAZE_SHOOT`, quiet and low.
fn play_shoot(host: &mut dyn Host, pos: [i32; 3]) {
    let r = (util::random() % 1000) as f32 / 1000.0;
    host.play_sound(
        "minecraft:entity.blaze.shoot",
        util::centre(pos),
        4,
        0.125 + r * 0.125,
        0.75 - r * 0.25,
    );
}

// --- drawing ---------------------------------------------------------------------------------------------

/// `AngleHelper.horizontalAngle`: the facing's y rotation, negated along x.
fn horizontal_angle(facing: Dir) -> f32 {
    let angle = match facing {
        Dir::South => 0.0,
        Dir::West => 90.0,
        Dir::North => 180.0,
        Dir::East => 270.0,
        _ => return 0.0,
    };
    if facing.axis() == create_kinetics::Axis::X {
        -angle
    } else {
        angle
    }
}

/// `BlazeBurnerRenderer.getBlazeModel`.
fn blaze_model(heat: HeatLevel, block_above: bool) -> &'static str {
    if heat.is_at_least(HeatLevel::Seething) {
        if block_above { SUPER_ACTIVE } else { SUPER }
    } else if heat.is_at_least(HeatLevel::Fading) {
        if block_above && heat.is_at_least(HeatLevel::Kindled) { ACTIVE } else { IDLE }
    } else {
        INERT
    }
}

impl Hooks for BlazeBurner {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in MODELS {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(FUEL_LEVEL.into(), Tag::Int(0));
            nbt.insert(BURN_TIME.into(), Tag::Int(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `BlazeBurnerBlockEntity.tick` on the server: the fuel burns down and the block's heat follows.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let heat = heat_of(host, pos);
        // Create has no block entity while the state is `none`.
        if heat == HeatLevel::None {
            return;
        }
        let old = read(&event.entity.nbt);
        let mut burner = old;
        let wanted = burner.tick();
        write(event.entity, old, burner);
        if let Some(next) = wanted
            && next != heat
        {
            set_heat(host, pos, next);
        }
    }

    /// Clients draw from the block's state alone.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(Compound::new)
    }

    /// `BlazeBurnerRenderer.renderShared`: the head bobs over the cage, the rods circle it.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let heat = host
            .block_state_property(event.state, "blaze")
            .as_deref()
            .and_then(HeatLevel::from_name)
            .unwrap_or(HeatLevel::None);
        if heat == HeatLevel::None {
            return;
        }
        let facing = host
            .block_state_property(event.state, "facing")
            .as_deref()
            .and_then(Dir::from_name)
            .unwrap_or(Dir::South);
        let [x, y, z] = event.pos;
        let draw = &mut *event.draw;
        let (time, above) = if let Some(tick) = crate::float(&event.entity.nbt, crate::ponder::TICK) {
            (tick, util::boolean(&event.entity.nbt, crate::ponder::BASIN_ABOVE))
        } else {
            (draw.game_time as f32 + draw.partial, host.block([x, y + 1, z]) == super::basin::ID)
        };
        // `headAnimation * .175`: the head lifts when a basin is above a burner that heats it.
        let animation = if above && heat.is_at_least(HeatLevel::Fading) { 0.175 } else { 0.0 };
        let block_above = animation > 0.125;
        let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ z.wrapping_mul(83_492_791))
            .unsigned_abs() as f32;
        let render_tick = time + (hash % 13.0) * 16.0;
        let offset_mult = if heat.is_at_least(HeatLevel::Fading) { 64.0 } else { 16.0 };
        let phase = (render_tick / 16.0) % (2.0 * std::f32::consts::PI);
        let offset = phase.sin() / offset_mult;
        let offset1 = (phase + std::f32::consts::PI).sin() / offset_mult;
        let offset2 = (phase + std::f32::consts::FRAC_PI_2).sin() / offset_mult;
        let head_y = offset - animation * 0.75;
        let angle = (horizontal_angle(facing) + 180.0) % 360.0;

        let light = draw.light;
        draw.light = FULL_BRIGHT;
        draw.push();
        draw.translate([0.0, head_y, 0.0]);
        draw.rotate_about([0.0, 1.0, 0.0], angle, CENTRE);
        host.draw_model(draw, blaze_model(heat, block_above), 0xFF_FFFF);
        draw.pop();
        if heat.is_at_least(HeatLevel::Fading) {
            let (small, large) = if heat == HeatLevel::Seething {
                (SUPER_RODS, SUPER_RODS_2)
            } else {
                (RODS, RODS_2)
            };
            draw.push();
            draw.translate([0.0, offset1 + animation + 0.125, 0.0]);
            host.draw_model(draw, small, 0xFF_FFFF);
            draw.pop();
            draw.push();
            draw.translate([0.0, offset2 + animation - 3.0 / 16.0, 0.0]);
            host.draw_model(draw, large, 0xFF_FFFF);
            draw.pop();
        }
        draw.light = light;
        // The bobbing reads the game time: nothing here is kept.
    }

    /// `BlazeBurnerBlock.getStateForPlacement`: a burner item with a blaze starts smouldering,
    /// the empty one has none, and it faces the player.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<crate::compat::BlockStateId>> {
        let context = event.context;
        let heat = match context.item.as_str() {
            ID => HeatLevel::Smouldering,
            EMPTY_ITEM => HeatLevel::None,
            _ => return None,
        };
        let facing = create_kinetics::looking(context.yaw, 0.0).opposite();
        let state = level.state_with_property(event.state, "facing", facing.name())?;
        level.state_with_property(state, "blaze", heat.name()).map(Some)
    }

    /// `BlazeBurnerBlock.useItemOn` and `LitBlazeBurnerBlock.useItemOn`.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if block != ID && block != LIT {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let main = host.player_equipment(uuid, EquipmentSlot::MainHand);
        // Sneaking with anything in a hand skips the block's own use.
        if player.sneaking
            && (main.is_some() || host.player_equipment(uuid, EquipmentSlot::OffHand).is_some())
        {
            return Flow::Continue;
        }
        let Some(stack) = main else {
            return Flow::Continue;
        };
        // The off hand's turn of a click the burner took (a stack of fuel is used up once more).
        if util::click_repeated(host, uuid, pos) {
            return Flow::Cancel;
        }
        let item = &*stack.item;
        let dimension = host.dimension();
        if block == LIT {
            if item.ends_with("_shovel") {
                // `EXTINGUISH_FLAME_ACTION`: back to an empty burner (no wear on the shovel).
                util::click_taken(host, uuid, pos);
                host.play_sound("minecraft:block.fire.extinguish", util::centre(pos), 4, 0.5, 2.0);
                if let Some(state) = host.default_state(ID) {
                    host.set_block(dimension, pos, state, util::UPDATE_ALL);
                }
                return Flow::Cancel;
            }
            if host.block_property(pos, "flame_type").as_deref() == Some("regular")
                && matches!(item, "minecraft:soul_sand" | "minecraft:soul_soil")
            {
                util::click_taken(host, uuid, pos);
                host.play_sound("minecraft:block.soul_sand.place", util::centre(pos), 4, 1.0, 1.0);
                host.set_block_property(pos, "flame_type", "soul");
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        let heat = heat_of(host, pos);
        if heat == HeatLevel::None {
            if item == "minecraft:flint_and_steel" {
                util::click_taken(host, uuid, pos);
                host.play_sound("minecraft:item.flintandsteel.use", util::centre(pos), 4, 1.0, 1.0);
                if let Some(state) = host.default_state(LIT) {
                    host.set_block(dimension, pos, state, util::UPDATE_ALL);
                }
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        if try_insert(host, pos, &stack, player.game_mode == util::CREATIVE) {
            util::click_taken(host, uuid, pos);
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    /// `BlazeBurnerBlockItem.useOn` on a spawner that can spawn Blazes.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if &*event.stack.item == util::WRENCH && (block == ID || block == LIT) {
            return util::wrench(host, pos, |_| {});
        }
        if &*event.stack.item != EMPTY_ITEM || block != "minecraft:spawner" {
            return Flow::Continue;
        }
        let dimension = host.dimension();
        let capturable = host
            .block_entity_nbt(dimension, pos)
            .is_some_and(|nbt| nbt.contains(CAPTURABLE));
        if !capturable {
            return Flow::Continue;
        }
        give_burner(host, pos);
        Flow::Cancel
    }

    /// `BlazeBurnerBlockItem.interactLivingEntity`: right-click a Blaze with the empty burner.
    fn interact_entity(&self, host: &mut dyn Host, event: &mut InteractEntityEvent<'_>) -> Flow {
        if event.kind != CAPTURABLE || event.stack.is_none_or(|s| &*s.item != EMPTY_ITEM) {
            return Flow::Continue;
        }
        let Ok(id) = u32::try_from(event.entity) else {
            return Flow::Continue;
        };
        let dimension = host.dimension();
        let at = host
            .entity(id)
            .and_then(|entity| host.entity_position(entity))
            .unwrap_or([0.0; 3]);
        if !host.remove_mob(dimension, id) {
            return Flow::Continue;
        }
        capture_sounds(host, at);
        give_burner_item(host);
        Flow::Cancel
    }
}

/// `spawnCaptureEffects` on the server: `BLAZE_HURT` and `FIRE_EXTINGUISH`.
fn capture_sounds(host: &mut dyn Host, at: [f64; 3]) {
    host.play_sound("minecraft:entity.blaze.hurt", at, 5, 0.25, 0.75);
    host.play_sound("minecraft:block.fire.extinguish", at, 5, 0.5, 0.75);
}

fn give_burner(host: &mut dyn Host, pos: [i32; 3]) {
    capture_sounds(host, util::centre(pos));
    give_burner_item(host);
}

/// `giveBurnerItemTo`: one empty burner is spent (not in creative) and a Blaze Burner comes back.
fn give_burner_item(host: &mut dyn Host) {
    let Some(player) = host.acting_player() else {
        return;
    };
    if player.game_mode != util::CREATIVE {
        host.take_items(EMPTY_ITEM, 1);
    }
    let filled = ItemStack::new(ID, 1);
    if let Some(rest) = host.give_stack(player.uuid, filled) {
        util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_turn_to_the_facing_plus_half_a_turn() {
        assert_eq!((horizontal_angle(Dir::South) + 180.0) % 360.0, 180.0);
        assert_eq!((horizontal_angle(Dir::North) + 180.0) % 360.0, 0.0);
        assert_eq!((horizontal_angle(Dir::West) + 180.0) % 360.0, 90.0);
        assert_eq!((horizontal_angle(Dir::East) + 180.0) % 360.0, -90.0);
    }

    #[test]
    fn the_blaze_model_follows_the_heat() {
        assert_eq!(blaze_model(HeatLevel::Smouldering, true), INERT);
        assert_eq!(blaze_model(HeatLevel::Fading, true), IDLE);
        assert_eq!(blaze_model(HeatLevel::Kindled, true), ACTIVE);
        assert_eq!(blaze_model(HeatLevel::Kindled, false), IDLE);
        assert_eq!(blaze_model(HeatLevel::Seething, true), SUPER_ACTIVE);
        assert_eq!(blaze_model(HeatLevel::Seething, false), SUPER);
    }

    #[test]
    fn vanilla_fuels_have_their_times() {
        assert_eq!(vanilla_burn_time("minecraft:coal"), Some(1600));
        assert_eq!(vanilla_burn_time("minecraft:oak_planks"), Some(300));
        assert_eq!(vanilla_burn_time("minecraft:stone"), None);
        assert_eq!(vanilla_burn_time("create:cogwheel"), None);
    }

    #[test]
    fn burners_round_trip_through_their_keys() {
        let mut nbt = Compound::new();
        nbt.insert(FUEL_LEVEL.into(), Tag::Int(2));
        nbt.insert(BURN_TIME.into(), Tag::Int(1234));
        nbt.insert(IS_CREATIVE.into(), Tag::Byte(1));
        assert_eq!(
            read(&nbt),
            Burner {
                fuel: FuelType::Special,
                remaining: 1234,
                creative: true
            }
        );
        assert_eq!(rules::MAX_HEAT_CAPACITY, 10_000);
    }
}

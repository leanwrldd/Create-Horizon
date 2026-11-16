//! What a Deployer does in the world at the end of its stroke: `DeployerHandler.activate`
//! (`activateInner`, `tryHarvestBlock`), by Horizon's fake player and break helpers. The stroke
//! itself is `deployer.rs`; the recipes are `deployer_apply.rs`; the numbers are
//! `create_kinetics::rules::deployer`.
//!
//! In `USE` mode (a right click): the entity in the cell two blocks in front is used
//! (`Player.interactOn` through `Host::use_as_player`), or the block there (a manual
//! `create:item_application` recipe first, then `useItemOn`: the block reacts, else the held item
//! is placed or used, else used in the air). In `PUNCH` mode (a left click): the entity is
//! attacked (`hurt_entity`), or the block mined a stage at a time by the held tool's speed.
//!
//! Known differences from Create:
//! - Horizon has no fake player entity. The Deployer's hand is a stack that `use_as_player`
//!   borrows for a click, so there is no real inventory (just the held stack and an overflow of
//!   stacks that didn't fit), no `FakePlayer` attributes and no protection by `mayInteract` or by
//!   the owner. A mob attacked doesn't retaliate (`ignoreDeployerAttacks`) because no entity
//!   attacks it.
//! - An attack deals `1 +` the held item's attack damage (`attack`) and no more (no enchantment,
//!   sweep, critical hit or knockback), as `damage_type` `create:deployer` with no attacker. A
//!   kill's drops are taken into the hand from the items that appear next to the mob in the same
//!   tick (`LivingDropsEvent`), and none of its experience drops.
//! - Mining speed is the tool's rules and the block's hardness, with no enchantment and no `/ 5`
//!   for being off the ground. `LeftClickBlock` and `BlockState.attack` (a note block's note, a
//!   dragon egg's hop) are not run, and the hit sound (`SoundType.getHitSound`) isn't played.
//! - `MobBucketItem.checkExtraContent`, `FLINT_AND_STEEL`'s fire repositioning, the villager and
//!   player food handling of `activateInner` and the bucket's fake wall (`ItemUseWorld`: a bucket
//!   uses the real ray, so fluid a few blocks beyond the cell may be taken or put down) are not
//!   ported. An ender pearl is not thrown (`item == ENDER_PEARL` returns after `useOn`).
//! - Create's Deployer clicks on the face the ray meets, and so does this one, but the ray is
//!   made from the block's collision boxes (`block_collision`), not its outline.

use create_kinetics::rules::deployer::{self as rules, Mode, Punch};
use create_kinetics::rules::drill;
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Hooks, Host, ItemStack, NativeFakeUse, NativeUseTarget, RandomSource};

use super::deployer_apply as apply;
use crate::util;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// The damage type of a Deployer's attacks (Create's fake player attacks as a player;
/// `data/create/damage_type/deployer.json` is written by `tools/content/deployer.py`).
pub(super) const DAMAGE_TYPE: &str = "create:deployer";
/// The held stack, then the overflow (`DeployerBlockEntity.overflowItems`, up to eleven stacks).
pub(super) type Items = Vec<Option<ItemStack>>;
pub(super) const SLOTS: usize = 1 + rules::OVERFLOW;
/// `SoundSource` ordinals.
const BLOCKS: usize = 4;

/// `blockBreakingProgress`: the block being mined and how far the punches have got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Crack {
    pub pos: Pos,
    pub progress: f32,
}

fn empty_stack() -> ItemStack {
    ItemStack { item: "minecraft:air".into(), count: 0, damage: 0, extra: None }
}

fn add(a: Pos, b: Pos, times: i32) -> Pos {
    [a[0] + b[0] * times, a[1] + b[1] * times, a[2] + b[2] * times]
}

// --- the hand's inventory -------------------------------------------------------------------------

/// `Inventory.placeItemBackInInventory` for the fake player: onto a stack of the same item, else
/// into the first free slot (the hand first); what has no room is dropped at the Deployer.
pub(super) fn place_back(host: &mut dyn Host, pos: Pos, items: &mut Items, mut stack: ItemStack) {
    if items.len() < SLOTS {
        items.resize(SLOTS, None);
    }
    let max = host.max_stack_size(&stack.item).unwrap_or(64);
    for have in items.iter_mut().flatten() {
        if stack.count == 0 {
            return;
        }
        if !util::same_item(have, &stack) {
            continue;
        }
        let moved = max.saturating_sub(have.count).min(stack.count);
        have.count += moved;
        stack.count -= moved;
    }
    if stack.count == 0 {
        return;
    }
    if let Some(slot) = items.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(stack);
        return;
    }
    util::spawn_item(host, util::centre(pos), &stack, [0.0; 3], 10);
}

/// `heldItem.hurtAndBreak(amount, player, MAINHAND)`: the held item wears, and breaks when it is
/// worn out. An item with no durability doesn't wear.
pub(super) fn wear_held(host: &mut dyn Host, pos: Pos, items: &mut Items, amount: u32) {
    if amount == 0 {
        return;
    }
    let Some(Some(stack)) = items.first_mut() else {
        return;
    };
    let max = rules::max_damage(&stack.item);
    if max == 0 {
        return;
    }
    match rules::worn(stack.damage, max, amount) {
        Some(damage) => stack.damage = damage,
        None => {
            items[0] = None;
            let (sound, volume, pitch) = rules::BREAK_SOUND;
            host.play_sound(sound, util::centre(pos), BLOCKS, volume, pitch);
        }
    }
}

/// What a recipe takes of the held item, unless it is kept (`BeltDeployerCallbacks.activate`,
/// `ManualApplicationRecipe`): a tool wears by one, anything else loses one.
pub(super) fn use_up(host: &mut dyn Host, pos: Pos, items: &mut Items, keep_held: bool) {
    if keep_held {
        return;
    }
    let Some(Some(stack)) = items.first_mut() else {
        return;
    };
    if rules::max_damage(&stack.item) > 0 {
        wear_held(host, pos, items, 1);
        return;
    }
    stack.count -= 1;
    if stack.count == 0 {
        items[0] = None;
    }
}

/// What came of a use of the held stack: the hand takes what it holds afterwards, and what else
/// ended up in the fake player's inventory goes back into the hand's inventory.
fn take_back(host: &mut dyn Host, pos: Pos, items: &mut Items, used: NativeFakeUse) {
    if items.len() < SLOTS {
        items.resize(SLOTS, None);
    }
    items[0] = used.held;
    for stack in used.other {
        place_back(host, pos, items, stack);
    }
}

// --- the stroke's work ----------------------------------------------------------------------------

/// `DeployerHandler.activate` for the Deployer at `pos` facing `facing`, whose hand is `items`.
pub(super) fn activate(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    mode: Mode,
    items: &mut Items,
    crack: &mut Option<Crack>,
) {
    let normal = facing.normal();
    let clicked = add(pos, normal, 2);
    // `rayOrigin = vec.add(extensionVector.scale(3 / 2f + 1 / 64f))`, where the fake player stands
    // (`player.setPos`), with its eyes on its feet (`deployerHasEyesOnHisFeet`).
    let centre = util::centre(pos);
    let reach = 3.0 / 2.0 + 1.0 / 64.0;
    let origin = [0, 1, 2].map(|i| centre[i] + f64::from(normal[i]) * reach);
    let (yaw, pitch) = (rules::fake_yaw(facing), rules::fake_pitch(facing));

    // Check for entities.
    let dimension = host.dimension();
    let cell = cell_entities(host, clicked);
    if !cell.is_empty() {
        let at = host
            .level_random(dimension)
            .and_then(|random| random.next_int_bound(cell.len() as i32).ok())
            .unwrap_or(0) as usize;
        let (id, bounds) = cell[at];
        let success = match mode {
            Mode::Use => use_on_entity(host, pos, items, id, bounds, origin, yaw, pitch),
            Mode::Punch => {
                attack(host, pos, items, id, bounds);
                true
            }
        };
        if success {
            return;
        }
    }

    match mode {
        Mode::Punch => punch_block(host, pos, facing, clicked, items, crack),
        Mode::Use => use_block(host, pos, facing, clicked, items, origin, yaw, pitch),
    }
}

/// `level.getEntitiesOfClass(Entity.class, new AABB(clickedPos))`: the entities whose box meets
/// the cell, with their boxes. (`entities_in` looks at feet, so a wider net is cast.)
fn cell_entities(host: &mut dyn Host, cell: Pos) -> Vec<(u32, [[f64; 3]; 2])> {
    let dimension = host.dimension();
    let [x, y, z] = cell.map(f64::from);
    let mut found = Vec::new();
    for id in host.entities_in(dimension, [x - 1.0, y - 3.0, z - 1.0], [x + 2.0, y + 1.0, z + 2.0]) {
        let Some(entity) = host.entity(id) else {
            continue;
        };
        let Some(bounds) = host.entity_bounding_box(entity) else {
            continue;
        };
        // `AABB.intersects`: strictly.
        let meets = (0..3).all(|i| {
            let lo = f64::from(cell[i]);
            bounds[0][i] < lo + 1.0 && bounds[1][i] > lo
        });
        if meets {
            found.push((id, bounds));
        }
    }
    found
}

fn use_on_entity(
    host: &mut dyn Host,
    pos: Pos,
    items: &mut Items,
    id: u32,
    bounds: [[f64; 3]; 2],
    origin: [f64; 3],
    yaw: f32,
    pitch: f32,
) -> bool {
    let held = items.first().cloned().flatten();
    // `Player.interactOn` at the middle of the entity, measured from its feet.
    let location = [0.0, (bounds[1][1] - bounds[0][1]) / 2.0, 0.0];
    let target = NativeUseTarget::Entity { id: id as i32, location };
    let Some(used) = host.use_as_player(held.unwrap_or_else(empty_stack), origin, yaw, pitch, target, false)
    else {
        return false;
    };
    let consumed = used.consumed;
    take_back(host, pos, items, used);
    consumed
}

/// `player.attack(entity)`: what the held item deals, its wear, and the drops of a kill taken
/// into the hand. Always counts as a success, as in Create.
fn attack(host: &mut dyn Host, pos: Pos, items: &mut Items, id: u32, bounds: [[f64; 3]; 2]) {
    let dimension = host.dimension();
    let held = items.first().cloned().flatten();
    let damage = rules::attack_damage(held.as_ref().map(|stack| &*stack.item));
    let (lo, hi) = (bounds[0].map(|v| v - 2.0), bounds[1].map(|v| v + 2.0));
    let before = items_near(host, lo, hi);
    let hurt = host.hurt_entity(dimension, id, damage, DAMAGE_TYPE, None);
    if hurt && let Some(held) = &held {
        wear_held(host, pos, items, rules::wear_per_attack(&held.item));
    }
    // `LivingDropsEvent`: what the kill dropped goes into the hand's inventory instead.
    capture_drops(host, pos, items, lo, hi, &before);
}

/// The ids of the item entities in the box.
fn items_near(host: &dyn Host, lo: [f64; 3], hi: [f64; 3]) -> Vec<u32> {
    host.item_entities(host.dimension(), lo, hi).iter().map(|item| item.id).collect()
}

/// `Entity.captureDrops` and `DeployerHandler.safeOnUse`: the item entities that appeared in the
/// box since `before` (a mob's loot, a bush's berries, a composter's bone meal) are taken out of
/// the world and into the hand's inventory.
fn capture_drops(host: &mut dyn Host, pos: Pos, items: &mut Items, lo: [f64; 3], hi: [f64; 3], before: &[u32]) {
    let dimension = host.dimension();
    for item in host.item_entities(dimension, lo, hi) {
        if before.contains(&item.id) {
            continue;
        }
        let mut gone = item.stack.clone();
        gone.count = 0;
        host.set_item_entity(dimension, item.id, gone);
        place_back(host, pos, items, item.stack);
    }
}

fn use_block(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    clicked: Pos,
    items: &mut Items,
    origin: [f64; 3],
    yaw: f32,
    pitch: f32,
) {
    let held = items.first().cloned().flatten();
    let block = host.block(clicked);
    // `RightClickBlock`: a recipe for the block and the held item applies first.
    if let Some(held) = &held
        && !drill::is_air(&block)
        && let Some(manual) = apply::manual_for(host, &block, held)
    {
        apply::apply_manual(host, clicked, &manual);
        use_up(host, pos, items, manual.keep_held);
        return;
    }
    if held.as_ref().is_some_and(|stack| &*stack.item == "minecraft:ender_pearl") {
        return;
    }
    // The ray through the cell, from its near face: where it hits is where the click lands.
    let boxes = host.block_collision(clicked);
    let hit = rules::front_hit(facing, &boxes);
    let mut point = util::centre(clicked);
    let axis = facing.axis().index();
    point[axis] = f64::from(clicked[axis]) + hit.along;
    let target = NativeUseTarget::Block { pos: clicked, face: util::direction(facing.opposite()), point };
    let [x, y, z] = clicked.map(f64::from);
    let (lo, hi) = ([x - 1.0, y - 1.0, z - 1.0], [x + 2.0, y + 2.0, z + 2.0]);
    let before = items_near(&*host, lo, hi);
    if let Some(used) = host.use_as_player(held.unwrap_or_else(empty_stack), origin, yaw, pitch, target, false) {
        let consumed = used.consumed;
        take_back(host, pos, items, used);
        if consumed {
            capture_drops(host, pos, items, lo, hi, &before);
        }
    }
}

/// The left click on a block: put out a fire in front, then mine with the held tool.
fn punch_block(
    host: &mut dyn Host,
    pos: Pos,
    facing: Dir,
    clicked: Pos,
    items: &mut Items,
    crack: &mut Option<Crack>,
) {
    let dimension = host.dimension();
    let block = host.block(clicked);
    // `clickedState.getShape().isEmpty()`: air and fluids have no shape.
    if drill::is_air(&block) || drill::is_liquid(&block) {
        *crack = None;
        return;
    }
    // `BlockHelper.extinguishFire`: fire on the face the ray met.
    let beside = add(clicked, facing.opposite().normal(), 1);
    if host.block(beside) == "minecraft:fire" {
        if let Some(air) = host.default_state("minecraft:air") {
            host.set_block(dimension, beside, air, util::UPDATE_ALL);
        }
        host.play_sound("minecraft:block.fire.extinguish", util::centre(beside), BLOCKS, 0.5, 2.6);
        return;
    }
    let Some(held) = items.first().cloned().flatten() else {
        return;
    };
    let id = drill::breaker_id(pos);
    let hardness = drill::hardness(&block);
    let in_tag = |tag: &str| host.block_name_in_tag(&block, tag).unwrap_or(false);
    let speed = rules::mining_speed(Some(&*held.item), &block, in_tag);
    let harvests = rules::has_correct_tool(Some(&*held.item), &block, in_tag);
    let destroy = rules::destroy_progress(speed, hardness, harvests);
    let before = crack.map_or(0.0, |crack| crack.progress);
    match rules::punch(destroy, before) {
        Punch::Break => {
            // `tryHarvestBlock`: the block's drops, rolled for the tool, go into the hand's
            // inventory when the tool is right for them.
            let drops = host.destroy_block(clicked, Some(&held), true).unwrap_or_default();
            if harvests {
                for stack in drops {
                    place_back(host, pos, items, stack);
                }
            }
            wear_held(host, pos, items, rules::wear_per_block(&held.item, hardness));
            host.set_break_progress(id, clicked, -1);
            *crack = None;
        }
        Punch::Reset => *crack = None,
        Punch::Crack { progress, stage } => {
            if let Some(stage) = stage {
                host.set_break_progress(id, clicked, stage);
            }
            *crack = Some(Crack { pos: clicked, progress });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world that knows stack sizes and counts the sounds made.
    #[derive(Default)]
    struct World {
        sounds: Vec<String>,
    }

    impl Host for World {
        fn max_stack_size(&self, item: &str) -> Option<u32> {
            Some(if item.ends_with("_axe") { 1 } else { 64 })
        }
        fn play_sound(&mut self, sound: &str, _: [f64; 3], _: usize, _: f32, _: f32) -> bool {
            self.sounds.push(sound.to_owned());
            true
        }
    }

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack { item: item.into(), count, damage: 0, extra: None }
    }

    #[test]
    fn what_comes_back_joins_the_hand_or_goes_to_the_overflow() {
        let mut host = World::default();
        let mut items: Items = vec![None; SLOTS];
        // Into the empty hand first.
        place_back(&mut host, [0; 3], &mut items, stack("minecraft:cobblestone", 10));
        assert_eq!(items[0].as_ref().map(|s| s.count), Some(10));
        // Onto the same item in the hand.
        place_back(&mut host, [0; 3], &mut items, stack("minecraft:cobblestone", 60));
        assert_eq!(items[0].as_ref().map(|s| s.count), Some(64));
        assert_eq!(items[1].as_ref().map(|s| (&*s.item, s.count)), Some(("minecraft:cobblestone", 6)));
        // Another item takes the next free slot.
        place_back(&mut host, [0; 3], &mut items, stack("minecraft:dirt", 3));
        assert_eq!(items[2].as_ref().map(|s| &*s.item), Some("minecraft:dirt"));
        // And joins the dirt, not a new slot.
        place_back(&mut host, [0; 3], &mut items, stack("minecraft:dirt", 3));
        assert_eq!(items[2].as_ref().map(|s| s.count), Some(6));
        assert!(items[3].is_none());
    }

    #[test]
    fn a_tool_wears_and_breaks_a_plain_item_is_used_up() {
        let mut host = World::default();
        let mut items: Items = vec![None; SLOTS];
        items[0] = Some(stack("minecraft:iron_axe", 1));
        // `max_damage` of an iron axe is 250.
        wear_held(&mut host, [0; 3], &mut items, 1);
        assert_eq!(items[0].as_ref().map(|s| s.damage), Some(1));
        items[0].as_mut().unwrap().damage = 249;
        wear_held(&mut host, [0; 3], &mut items, 1);
        assert!(items[0].is_none());
        assert_eq!(host.sounds, ["minecraft:entity.item.break"]);

        // A stick has no durability: `use_up` takes one.
        items[0] = Some(stack("minecraft:stick", 2));
        use_up(&mut host, [0; 3], &mut items, false);
        assert_eq!(items[0].as_ref().map(|s| s.count), Some(1));
        use_up(&mut host, [0; 3], &mut items, true);
        assert_eq!(items[0].as_ref().map(|s| s.count), Some(1));
        use_up(&mut host, [0; 3], &mut items, false);
        assert!(items[0].is_none());
        // A tool kept by its recipe doesn't wear (`keep_held_item`).
        items[0] = Some(stack("minecraft:iron_axe", 1));
        use_up(&mut host, [0; 3], &mut items, true);
        assert_eq!(items[0].as_ref().map(|s| s.damage), Some(0));
        use_up(&mut host, [0; 3], &mut items, false);
        assert_eq!(items[0].as_ref().map(|s| s.damage), Some(1));
    }

    #[test]
    fn what_the_fake_player_uses_comes_back_into_the_hand() {
        let mut host = World::default();
        let mut items: Items = vec![None; SLOTS];
        items[0] = Some(stack("minecraft:bucket", 1));
        let mut used = NativeFakeUse::default();
        used.consumed = true;
        used.held = Some(stack("minecraft:water_bucket", 1));
        used.other = vec![stack("minecraft:glass_bottle", 1)];
        take_back(&mut host, [0; 3], &mut items, used);
        assert_eq!(items[0].as_ref().map(|s| &*s.item), Some("minecraft:water_bucket"));
        assert_eq!(items[1].as_ref().map(|s| &*s.item), Some("minecraft:glass_bottle"));
    }
}

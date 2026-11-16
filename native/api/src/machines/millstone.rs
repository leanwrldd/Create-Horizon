//! The Millstone (`MillstoneBlock`, `MillstoneBlockEntity`, `MillstoneRenderer`): grinds the item
//! in its one input slot into its nine output slots with Create's `create:milling` recipes,
//! turning an inner cog. The rules (timer, speed, slot arithmetic) are
//! `create_kinetics::rules::millstone`; this is the glue.
//!
//! Known differences from Create:
//! - Items lying on top are pulled in by a scan every few ticks, not when they land
//!   (`updateEntityAfterFallOn`).
//! - The slots are the block entity's own item slots (`Items`: 0 is the input, 1 to 9 the
//!   outputs), saved as Horizon saves them, not Create's `InputInventory` / `OutputInventory`
//!   compounds. `Timer` is Create's key.
//! - No grinding particles or sound, and no `getCraftingRemainingItem` leftovers.

use create_kinetics::rules::millstone::{
    self as rules, INPUT_SLOTS, OUTPUT_SLOTS, Step, TOP_SCAN,
};
use create_kinetics::{Axis, Kind, Node};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, ContainerAcceptsEvent,
    EquipmentSlot, Flow, Hooks, Host, ItemStack, ModBlockEntity, ModContainer,
    RightClickBlockEvent, StandaloneModelsEvent, Tag, UseItemOnEvent,
};

use crate::{kinetic, recipes, util};

const ID: &str = "create:millstone";
/// `AllRecipeTypes.MILLING`.
const RECIPE_TYPE: &str = "create:milling";
/// `AllPartialModels.MILLSTONE_COG`.
const INNER: &str = "create:block/millstone/inner";
const TIMER: &str = "Timer";
const SLOTS: usize = INPUT_SLOTS + OUTPUT_SLOTS;
const CENTRE: [f32; 3] = [0.5; 3];

struct Millstone;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Millstone)
}

/// The millstone on its network: `MillstoneBlock` is a small cogwheel (`ICogWheel`) turning about
/// Y with a shaft towards down only (`hasShaftTowards`).
fn node() -> Node {
    Node {
        impact: rules::IMPACT,
        // A small cog on Y with only its bottom shaft end, as the encased cogwheels are built.
        ..Node::passive(Kind::encased_cog(Axis::Y, false, false, true))
    }
}

/// One container for the whole block, as `MillstoneInventoryHandler` (a `CombinedInvWrapper`)
/// is the same from every side: put into the input, take out of the outputs.
fn container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| all.clone()),
        inputs: (0..INPUT_SLOTS).collect(),
        outputs: (INPUT_SLOTS..SLOTS).collect(),
        menu: None,
    }
}

fn max_stack(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.max_stack_size(&stack.item).unwrap_or(64)
}

/// The slots, padded to [`SLOTS`] (a fresh entity may hold none yet).
fn slots_of(entity: &ModBlockEntity) -> Vec<Option<ItemStack>> {
    let mut slots = entity.items.clone();
    slots.resize(SLOTS, None);
    slots
}

/// Writes the slots back only when they differ: any `&mut` reach into the entity counts as a change.
fn store(entity: &mut ModBlockEntity, slots: Vec<Option<ItemStack>>) -> bool {
    if slots == entity.items {
        return false;
    }
    entity.items = slots;
    true
}

fn set_count(stack: &mut ItemStack, count: u32) {
    stack.count = count;
}

/// Puts `stack` into the input slot as far as a recipe takes it and it fits
/// (`MillstoneInventoryHandler.insertItem`). Answers what is left.
fn insert_input(host: &dyn Host, slots: &mut [Option<ItemStack>], stack: ItemStack) -> ItemStack {
    if recipes::for_stack(host, RECIPE_TYPE, &stack).is_none() {
        return stack;
    }
    let limit = rules::SLOT_LIMIT.min(max_stack(host, &stack));
    let mut rest = stack.clone();
    match &mut slots[0] {
        Some(have) => {
            if !util::same_item(have, &stack) {
                return stack;
            }
            let moved = limit.saturating_sub(have.count).min(stack.count);
            have.count += moved;
            rest.count -= moved;
        }
        empty => {
            let moved = limit.min(stack.count);
            let mut put = stack;
            put.count = moved;
            rest.count -= moved;
            *empty = Some(put);
        }
    }
    rest
}

/// `MillstoneBlock.updateEntityAfterFallOn`, as a scan of what lies on top.
fn take_from_top(host: &mut dyn Host, pos: [i32; 3], slots: &mut [Option<ItemStack>]) -> bool {
    let [x, y, z] = pos.map(f64::from);
    let dimension = host.dimension();
    let mut changed = false;
    for item in host.item_entities(dimension, [x, y + 0.9, z], [x + 1.0, y + 1.5, z + 1.0]) {
        let [px, _, pz] = item.position;
        if !(x..x + 1.0).contains(&px) || !(z..z + 1.0).contains(&pz) {
            continue;
        }
        let on_ground = host
            .entity(item.id)
            .and_then(|entity| host.entity_on_ground(entity))
            .unwrap_or(false);
        if !on_ground {
            continue;
        }
        let rest = insert_input(host, slots, item.stack.clone());
        if rest.count != item.stack.count {
            host.set_item_entity(dimension, item.id, rest);
            changed = true;
        }
    }
    changed
}

/// `MillstoneBlockEntity.process`: one input item becomes one roll of the recipe's results.
fn process(host: &mut dyn Host, slots: &mut [Option<ItemStack>]) -> bool {
    let Some(input) = slots[0].clone() else {
        return false;
    };
    let Some(recipe) = recipes::for_stack(host, RECIPE_TYPE, &input) else {
        return false;
    };
    if input.count <= 1 {
        slots[0] = None;
    } else if let Some(have) = &mut slots[0] {
        have.count -= 1;
    }
    let dimension = host.dimension();
    for made in recipes::roll(host, dimension, &recipe.results) {
        let limit = rules::SLOT_LIMIT.min(max_stack(host, &made));
        // `ItemHandlerHelper.insertItemStacked`: what no slot takes is lost, as in Create.
        let _ = rules::insert_stacked(
            &mut slots[INPUT_SLOTS..],
            made,
            |s| s.count,
            set_count,
            util::same_item,
            limit,
        );
    }
    true
}

impl Hooks for Millstone {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(INNER.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = kinetic::new_entity();
            nbt.insert(TIMER.into(), Tag::Int(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(container)
    }

    /// `MillstoneInventoryHandler.isItemValid`: only what a recipe takes goes in.
    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        if event.slot >= INPUT_SLOTS || recipes::for_stack(host, RECIPE_TYPE, event.stack).is_none()
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node()) else {
            return; // The network jammed and the block broke.
        };
        let mut sync = changed;
        let mut slots = slots_of(event.entity);

        if util::game_time(host).rem_euclid(TOP_SCAN) == 0 && take_from_top(host, pos, &mut slots) {
            sync = true;
        }

        let output_full = slots[INPUT_SLOTS..].iter().flatten().any(|stack| {
            rules::slot_full(stack.count, max_stack(host, stack))
        });
        let before = util::int(&event.entity.nbt, TIMER);
        let mut timer = before;
        match rules::step(&mut timer, rpm, output_full, slots[0].is_none()) {
            Step::Process => {
                // Nothing to grind any more: the timer just stays run down, as Create's does.
                process(host, &mut slots);
                sync = true;
            }
            Step::Lookup => {
                let time = recipes::for_stack(host, RECIPE_TYPE, slots[0].as_ref().expect("input"))
                    .map(|recipe| recipe.processing_time);
                timer = rules::timer_for(time);
                sync = true;
            }
            Step::Stopped | Step::Counting | Step::Idle => {}
        }
        if timer != before {
            event.entity.nbt.insert(TIMER.into(), Tag::Int(timer));
        }
        if store(event.entity, slots) {
            sync = true;
        }
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        (event.entity.id == ID).then(|| {
            let mut tag = kinetic::update_tag(event.entity);
            tag.insert(TIMER.into(), Tag::Int(util::int(&event.entity.nbt, TIMER)));
            tag
        })
    }

    /// `MillstoneRenderer`: the casing is in the chunk mesh; the inner cog turns about Y.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let rpm = kinetic::speed(event.entity);
        // `KineticBlockEntityVisual.rotationOffset`: a cog is turned 22.5 degrees where the
        // other two coordinates add up even, so neighbours mesh.
        let [x, _, z] = event.pos;
        let phase = if (x + z).rem_euclid(2) == 0 { 22.5 } else { 0.0 };
        let draw = &mut *event.draw;
        draw.spin(Axis::Y.unit(), CENTRE, rpm * 0.3, phase);
        host.draw_model(draw, INNER, 0xFF_FFFF);
        draw.still();
        draw.keep();
    }

    /// `MillstoneBlock.useItemOn`: an empty hand takes the outputs, or, with none, the input.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
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
            && (main.is_some()
                || host
                    .player_equipment(uuid, EquipmentSlot::OffHand)
                    .is_some())
        {
            return Flow::Continue;
        }
        // `PASS_TO_DEFAULT_BLOCK_INTERACTION` for anything in the hand.
        if main.is_some() {
            return Flow::Continue;
        }
        // The client may send the off hand's click for the same use: once is enough.
        if !util::first_click(host, uuid, pos) {
            return Flow::Cancel;
        }

        let mut taken: Vec<ItemStack> = Vec::new();
        host.change_mod_block_entity(pos, &mut |entity| {
            let mut slots = slots_of(entity);
            let outputs: Vec<ItemStack> = slots[INPUT_SLOTS..].iter_mut().filter_map(Option::take).collect();
            if outputs.is_empty() {
                taken = slots[..INPUT_SLOTS].iter_mut().filter_map(Option::take).collect();
            } else {
                taken = outputs;
            }
            store(entity, slots);
        });
        for stack in taken {
            // `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
            if let Some(rest) = host.give_stack(uuid, stack) {
                util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
            }
        }
        host.block_entity_sync(pos);
        Flow::Cancel
    }

    /// The Wrench: sneak-click picks the millstone up (`IWrenchable`; it has nothing to turn).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}


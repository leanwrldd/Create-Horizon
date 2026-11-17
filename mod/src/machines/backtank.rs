//! The Copper and Netherite Backtanks (`BacktankItem`, `BacktankBlock`, `BacktankBlockEntity`,
//! `BacktankUtil`): a chestplate that holds air, and the same item placed as a block that fills with
//! air from rotation. The air is the item component `create:backtank_air`; what a worn tank does with
//! it is `diving.rs`. The numbers are `create_kinetics::rules::backtank`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - The worn tank is drawn as an iron (copper) or netherite chestplate: the items are made from
//!   those templates, with Create's armour values, and Create's own armour layers (and the backpack
//!   model `BacktankArmorLayer`) are not in the port. A copper tank has no equip sound of its own.
//! - No Capacity enchantment (`AllEnchantments.CAPACITY`: a tank holds 900 air, 300 more a level):
//!   the port has no enchantment of its own, so a tank holds 900.
//! - The air shows as the item's bar (13 pixels, `BacktankItem.getBarWidth`); no air overlay
//!   (`RemainingAirOverlay`), no first-person netherite model, no comparator output, no custom name,
//!   no waterlogging (a placed tank under water still fills), and no air particles or fill effect.
//! - A placed tank keeps its air in its block entity (`Air`, `Timer`) and gives it back as the
//!   component when broken by a player, an explosion or a piston: the drop gets the air from the block
//!   entity (untried when the block is broken by something other than a player).
//! - Create's separate `copper_backtank_placeable` item doesn't exist: the worn item itself places
//!   the block when used on a block (the content item's `place_block`), and equips when used in
//!   the air, as `BacktankItem.useOn` does.

use create_kinetics::rules::backtank::{self as rules, Fill};
use create_kinetics::{Dir, Kind, Node};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, ComponentTypesEvent, Compound, Hooks, Host, ItemBarEvent,
    ItemStack, LevelHost, LootDropsEvent, PlaceBlockEvent, StandaloneModelsEvent, StateForPlacementEvent,
    Tag,
};

use crate::{kinetic, util};

const AIR: &str = "Air";
const TIMER: &str = "Timer";

struct Backtanks;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Backtanks)
}

/// `BacktankUtil.getAir` of a stack.
pub(crate) fn air_of(stack: &ItemStack) -> i32 {
    match stack.data().and_then(|d| d.other(rules::COMPONENT)) {
        Some(Tag::Int(air)) => rules::air_of(*air, rules::max_air(0)),
        _ => 0,
    }
}

fn is_block(id: &str) -> bool {
    rules::is_backtank(id)
}

fn cogs(block: &str) -> String {
    format!("create:block/{}/block_cogs", block.trim_start_matches("create:"))
}

fn shaft(block: &str) -> String {
    format!("create:block/{}/block_shaft_input", block.trim_start_matches("create:"))
}

/// A state's facing as the rotation (counterclockwise about +y) that the model's block state gives.
fn turn(facing: &str) -> f32 {
    match facing {
        "east" => 270.0,
        "south" => 180.0,
        "west" => 90.0,
        _ => 0.0,
    }
}

impl Hooks for Backtanks {
    /// `AllDataComponents.BACKTANK_AIR`.
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == rules::COMPONENT) {
            event.types.push(rules::COMPONENT.to_owned());
        }
    }

    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for block in [rules::COPPER, rules::NETHERITE] {
            event.models.push(cogs(block));
            event.models.push(shaft(block));
        }
    }

    /// `BacktankItem.getBarWidth` and `getBarColor`: always shown.
    fn item_bar(&self, _: &dyn Host, event: &mut ItemBarEvent<'_>) {
        if !rules::is_backtank(&event.stack.item) {
            return;
        }
        *event.bar = Some((rules::bar_width(air_of(event.stack), rules::max_air(0)), rules::BAR_COLOR));
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_block(event.block).then(|| {
            let mut nbt = kinetic::new_entity();
            nbt.insert(AIR.into(), Tag::Int(0));
            nbt.insert(TIMER.into(), Tag::Int(0));
            (event.block.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_block(event.block) {
            event.ticks = true;
        }
    }

    /// `BacktankBlockEntity.tick`: air in while the shaft turns.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_block(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        // `hasShaftTowards`: the shaft comes out of the top.
        let node = Node { impact: rules::IMPACT, ..Node::passive(Kind::shafts(&[Dir::Up])) };
        let Some((rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        if changed {
            host.block_entity_sync(pos);
        }
        let air = util::int(&event.entity.nbt, AIR);
        let timer = util::int(&event.entity.nbt, TIMER);
        let Fill { air: new_air, timer: new_timer } = rules::fill_tick(rpm, air, timer, rules::max_air(0));
        if new_air != air || new_timer != timer {
            event.entity.nbt.insert(AIR.into(), Tag::Int(new_air));
            event.entity.nbt.insert(TIMER.into(), Tag::Int(new_timer));
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        is_block(&event.entity.id).then(|| kinetic::update_tag(event.entity))
    }

    /// `BacktankRenderer.renderSafe`: the shaft out of the top and the cogs turning at a quarter of the speed.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_block(event.block) {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing") else {
            return;
        };
        let rpm = kinetic::speed(event.entity);
        let draw = &mut *event.draw;
        kinetic::draw_shaft_half(host, draw, Dir::Up, rpm);
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], turn(facing), [0.5; 3]);
        // The cogs turn about the east-west axis through (0.5, 6.5/16, 11/16).
        draw.spin([1.0, 0.0, 0.0], [0.5, 6.5 / 16.0, 11.0 / 16.0], rpm / 4.0, 0.0);
        host.draw_model(draw, &cogs(event.block), 0xFF_FFFF);
        draw.still();
        draw.pop();
        // Not kept: the cogs turn by the game time.
    }

    /// `HorizontalKineticBlock.getStateForPlacement`: the tank faces the player.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if !is_block(&event.context.item) {
            return None;
        }
        let facing = create_kinetics::looking(event.context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `BacktankBlock.setPlacedBy`: the air of the item goes into the block.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !rules::is_backtank(&event.stack.item) {
            return;
        }
        let air = air_of(event.stack);
        host.change_mod_block_entity(event.pos, &mut |entity| {
            entity.nbt.insert(AIR.into(), Tag::Int(air));
        });
    }

    /// The loot table's `copy_components` of the air: the drop carries what the block held.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let Some((pos, _)) = event.context.block else {
            return;
        };
        for drop in event.drops.iter_mut() {
            if !rules::is_backtank(&drop.item) {
                continue;
            }
            if let Some(entity) = host.mod_block_entity(pos) {
                let air = util::int(&entity.nbt, AIR);
                if air > 0 {
                    let _ = host.set_component(drop, rules::COMPONENT, Tag::Int(air));
                }
            }
        }
    }
}

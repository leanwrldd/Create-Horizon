//! The Mechanical Crafter (`MechanicalCrafterBlock`, `MechanicalCrafterBlockEntity`,
//! `ConnectedInputHandler`, `RecipeGridHandler`, `EdgeInteractionBehaviour`,
//! `MechanicalCrafterRenderer`): a cogwheel-driven casing that holds one item, joins into grids
//! with the Wrench, passes its items along the arrows of the crafters pointing at each other to
//! the last one, which crafts what the grid makes (a crafting or a `create:mechanical_crafting`
//! recipe) and hands the result to what it points at. The rules are
//! `create_kinetics::rules::crafter`, `crafter_grid` and `crafter_recipes`; the world reading is
//! `crafter_world.rs`, the picture `crafter_draw.rs`, the recipes' data `crafter_recipes.rs`.
//!
//! Known differences from Create, for the README:
//! - Vanilla crafting recipes are baked into the content (Horizon gives a mod none to read):
//!   shaped and shapeless ones only, from the prepared game data, and a data pack's changes to
//!   them aren't seen (`crafter_recipes.rs`). `create:mechanical_crafting` is read as it is.
//! - The inventory of a grid (`getItemHandler`, a `CombinedInvWrapper`) is the one hands, funnels,
//!   chutes, Deployers and arms fill and the arm empties, one item to each crafter in reading
//!   order. A hopper or a dropper is Horizon's own container call on one crafter's one slot, so
//!   it fills only the crafter it is at. An item with components (a named or enchanted item)
//!   can't be taken from the hand by a click, so such a stack isn't inserted by hand.
//! - A grid starts the moment its last crafter is filled by hand, funnel, chute, arm or Deployer;
//!   an item a hopper put in is noticed within 4 ticks (Create's `whenContentsChanged`), and
//!   redstone is read every tick.
//! - Joining uses Create's strip along the edge of a face (`EdgeInteractionHandler`) by the
//!   player's eye ray and the blocks' collision boxes for `hasBlockSolidSide`; there are no
//!   connected textures (`CrafterCTBehaviour`) and no outlines.
//! - Breaking a crafter ejects its grid and parts its group, but a crafter destroyed by an
//!   explosion or another mod loses what it held. No advancements (`CRAFTER`, `CRAFTER_LAZY`),
//!   no crit particles, no item particles while crafting, and the "was powered" memory is
//!   saved (`Powered`), where Create's isn't.
//! - Placing one against another copies its facing and pointing as Create does, but not
//!   `getPreferredHorizontalFacing`; no `onRemove` for a crafter moved by a contraption, and
//!   no `TransformableBlockEntity`.
//! - The picture counts from the last update rather than ticking on the client, so the
//!   movement may jump by a tick when the speed changes in the middle of a step.
//! - Saved: Create's `GroupedItems`, `Phase`, `CountDown`, `Cover` and `ConnectedInput`, and
//!   `Powered`, `BeforeCraft`, `Anchor` and `Since` of this port (`crafter_world.rs`).

use create_kinetics::rules::crafter::{self as rules, Pointing, Use, UseCase};
use create_kinetics::rules::crafter_grid::{Crafter, Phase};
use create_kinetics::rules::redstone_link::view_vector;
use create_kinetics::{Dir, Kind, Node, Pos};
use crate::compat::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, ContainerAcceptsEvent,
    DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack, LevelHost, ModBlockEntity,
    ModContainer, RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag,
    UseItemOnEvent,
};

use super::crafter_draw as draw;
use super::crafter_world::{
    self as world, ANCHOR, BEFORE, COVER, COVERED, GROUPED, GameEnv, ID, PHASE, SINCE,
};
use crate::{kinetic, util};

/// `Level.UPDATE_CLIENTS`-free: the table of slots is the one block entity's.
const MECHANICAL_ARM: &str = "create:mechanical_arm";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

struct Crafters;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Crafters)
}

/// One slot, put into by hoppers and never taken from (`forbidExtraction`).
fn container() -> ModContainer {
    ModContainer {
        slots: 1,
        faces: [(); 6].map(|()| vec![0]),
        inputs: vec![0],
        outputs: Vec::new(),
        menu: None,
    }
}

/// Which tick of every few this crafter polls on, so a wall of them doesn't all at once.
fn offset(pos: Pos) -> i64 {
    i64::from(pos[0].wrapping_mul(3) + pos[1].wrapping_mul(5) + pos[2].wrapping_mul(7))
}

/// How often an idle crafter holding an item looks whether the grid is full, in ticks.
const IDLE_CHECK: i64 = 4;
/// `setLazyTickRate(20)`.
const LAZY: i64 = 20;

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    // `CStress.setImpact(2.0)`; a small cogwheel on the facing axis, with no shaft ends
    // (`ICogWheel`, `KineticBlock.hasShaftTowards`).
    let node = Node {
        impact: rules::IMPACT,
        ..Node::passive(Kind::encased_cog(facing.axis(), false, false, false))
    };
    let Some((_rpm, changed)) = kinetic::tick(host, pos, event.entity, node) else {
        return; // The network jammed and the block broke.
    };
    let mut sync = changed;
    let now = util::game_time(&*host);

    // `SmartInventory(1, be, 1, false)` holds one item: whatever a hopper stacked is thrown out.
    if let Some(Some(stack)) = event.entity.items.first()
        && stack.count > 1
    {
        let mut extra = stack.clone();
        extra.count -= 1;
        if let Some(Some(slot)) = event.entity.items.first_mut() {
            slot.count = 1;
        }
        let front = facing.normal();
        let at = [0, 1, 2].map(|i| f64::from(pos[i]) + 0.5 + f64::from(front[i]) * 0.75);
        util::spawn_item(host, at, &extra, [0.0; 3], 10);
        sync = true;
    }

    let dimension = host.dimension();
    let powered = host.neighbor_signal(dimension, pos) > 0;
    let Some(own) = world::read(&*host, pos, event.entity) else {
        return;
    };
    let idle_check = own.phase == Phase::Idle
        && own.slot.is_some()
        && (now + offset(pos)).rem_euclid(IDLE_CHECK) == 0;
    let insert_check = own.phase == Phase::Inserting && (now + offset(pos)).rem_euclid(LAZY) == 0;
    let working = matches!(own.phase, Phase::Assembling | Phase::Exporting | Phase::Crafting)
        && own.speed != 0.0
        || own.phase == Phase::Inserting;
    let edge = own.was_powered != powered;
    let gather = working || edge || idle_check;

    let mut snapshot = if gather {
        match world::gather(&*host, pos, event.entity) {
            Some(snapshot) => snapshot,
            None => return,
        }
    } else if own.phase == Phase::Idle {
        // Nothing to do but remember the redstone.
        if sync {
            host.block_entity_sync(pos);
        }
        return;
    } else {
        world::alone(pos, own)
    };

    let mut env = GameEnv::new(host, &snapshot);
    let mut effects = snapshot.world.tick(pos, powered, &mut env);
    if idle_check || insert_check {
        effects.extend(snapshot.world.lazy_tick(pos, &mut env));
    }
    if world::own_visible(&snapshot, pos) {
        sync = true;
    }
    world::commit(host, &snapshot, Some((pos, &mut *event.entity)));
    world::apply_effects(host, &snapshot.world, effects);
    if sync {
        host.block_entity_sync(pos);
    }
}

/// What a client needs to draw the crafter.
fn update_tag(entity: &ModBlockEntity) -> Compound {
    let nbt = &entity.nbt;
    let mut tag = kinetic::update_tag(entity);
    if let Some(phase) = util::get(nbt, PHASE) {
        tag.insert(PHASE.into(), phase.clone());
    }
    tag.insert(COVERED.into(), Tag::Byte(i8::from(util::boolean(nbt, COVERED))));
    let anchor = if util::has(nbt, ANCHOR) { util::int(nbt, ANCHOR) } else { util::int(nbt, world::COUNT_DOWN) };
    tag.insert(ANCHOR.into(), Tag::Int(anchor));
    tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
    for key in [GROUPED, BEFORE] {
        if let Some(grid) = util::get(nbt, key) {
            tag.insert(key.into(), grid.clone());
        }
    }
    let slot = entity.items.first().cloned().flatten();
    let inventory = slot.as_ref().map_or_else(|| Tag::Compound(Compound::new()), util::stack_tag);
    tag.insert(draw::INVENTORY.into(), inventory);
    tag
}

/// The player's own stack in a hand, or none.
fn hand(host: &dyn Host, uuid: u128, slot: EquipmentSlot) -> Option<ItemStack> {
    host.player_equipment(uuid, slot)
}

/// `Inventory.placeItemBackInInventory`: what does not fit drops at the player.
fn give(host: &mut dyn Host, uuid: u128, at: [f64; 3], stack: ItemStack) {
    if let Some(rest) = host.give_stack(uuid, stack) {
        util::spawn_item(host, at, &rest, [0.0; 3], 0);
    }
}

/// `MechanicalCrafterBlock.useItemOn`, for a hand holding `stack` (none: empty).
fn click(host: &mut dyn Host, pos: Pos, event_face: [i32; 3]) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == util::SPECTATOR {
        return Flow::Continue;
    }
    let uuid = player.uuid;
    let main = hand(&*host, uuid, EquipmentSlot::MainHand);
    let off = hand(&*host, uuid, EquipmentSlot::OffHand);
    // The Wrench joins and turns (`use_item_on`); the arm selects the block (its own module).
    if main.as_ref().is_some_and(|s| &*s.item == util::WRENCH || &*s.item == MECHANICAL_ARM) {
        return Flow::Continue;
    }
    // Sneaking with anything in a hand skips the block's own use.
    if player.sneaking && (main.is_some() || off.is_some()) {
        return Flow::Continue;
    }
    let (Some(entity), Some(facing)) = (host.mod_block_entity(pos), util::facing(&*host, pos, "facing")) else {
        return Flow::Continue;
    };
    let slot = entity.items.first().cloned().flatten();
    let case = |stack: Option<&ItemStack>, is_main: bool| UseCase {
        front: Dir::from_normal(event_face) == Some(facing),
        idle: Phase::from_name(&match util::get(&entity.nbt, PHASE) {
            Some(Tag::String(s)) => s.to_string_lossy().to_string(),
            _ => String::new(),
        }) == Phase::Idle,
        empty_main_hand: stack.is_none() && is_main,
        wrench: false,
        cover_item: stack.is_some_and(|s| &*s.item == COVER),
        arm: false,
        covered: util::boolean(&entity.nbt, COVERED),
        slot_empty: slot.is_none(),
        same_as_slot: matches!((stack, &slot), (Some(s), Some(have)) if util::same_item(s, have)),
    };
    // The main hand first; when it does nothing and is empty, the off hand's turn.
    let mut used = rules::use_item_on(case(main.as_ref(), true));
    let mut held = main;
    if used == Use::Pass && held.is_none() && off.is_some() {
        used = rules::use_item_on(case(off.as_ref(), false));
        held = off;
    }
    if used == Use::Pass {
        return Flow::Continue;
    }
    // Once a click: the off hand's turn would act again.
    if !util::first_click(&*host, uuid, pos) {
        return Flow::Cancel;
    }
    let creative = player.game_mode == util::CREATIVE;
    match used {
        Use::Pass => {}
        Use::EjectGrid => world::eject_whole_grid(host, pos),
        Use::PlaceCover => {
            world::set_cover(host, pos, true);
            if !creative {
                host.take_items(COVER, 1);
            }
        }
        Use::Insert => {
            if let Some(stack) = held {
                // What goes in is worked out first, so that exactly that is taken from the hand
                // (a stack with components can't be taken by a click: see the module's list).
                let rest = world::handler_insert(host, pos, &stack, true);
                let going = stack.count - rest.count;
                if going > 0 && stack.extra.is_none() {
                    let took = host.take_items(&stack.item, going);
                    if took > 0 {
                        let mut part = stack;
                        part.count = took;
                        world::handler_insert(host, pos, &part, false);
                    }
                }
            }
        }
        Use::Uncover => {
            world::set_cover(host, pos, false);
            if !creative {
                give(host, uuid, player.position, ItemStack::new(COVER, 1));
            }
        }
        Use::TakeItem => {
            if let Some(stack) = slot {
                give(host, uuid, player.position, stack);
                world::take_slot(host, pos);
            }
        }
    }
    Flow::Cancel
}

/// `EdgeInteractionHandler.onBlockActivated` with the Wrench: the direction of the strip the
/// player's eye ray meets on the face clicked, if it joins a neighbour; joins or parts it.
fn edge_click(host: &mut dyn Host, pos: Pos, face: Dir) -> bool {
    let (Some(player), Some(facing)) = (host.acting_player(), util::facing(&*host, pos, "facing")) else {
        return false;
    };
    let eye = [
        player.position[0],
        player.position[1] + f64::from(player.eye_height),
        player.position[2],
    ];
    let Some(hit) = rules::face_hit(pos, face, eye, view_vector(player.yaw, player.pitch)) else {
        return false;
    };
    let solid = |host: &dyn Host, at: Pos| rules::face_covered(&host.block_collision(at), face.opposite());
    // `getConnectiveSides`: nothing if the face itself is covered.
    if solid(&*host, util::add(pos, face.normal())) {
        return false;
    }
    let mut candidates = Vec::new();
    for direction in Dir::ALL {
        if direction.axis() == face.axis() {
            continue;
        }
        let neighbour = util::add(pos, direction.normal());
        if solid(&*host, util::add(neighbour, face.normal())) {
            continue;
        }
        let beside = (host.block(neighbour) == ID).then(|| util::facing(&*host, neighbour, "facing")).flatten();
        if rules::should_connect(facing, face, direction, beside) {
            candidates.push(direction);
        }
    }
    let Some(direction) = rules::activated_direction(pos, &candidates, hit) else {
        return false;
    };
    world::toggle_connection(host, pos, util::add(pos, direction.normal()));
    host.play_sound("minecraft:entity.item_frame.add_item", util::centre(pos), BLOCKS, 0.25, 0.1);
    true
}

impl Hooks for Crafters {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [draw::LID, draw::ARROW, draw::BELT_FRAME, draw::BELT, draw::COG] {
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
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
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

    /// `Inventory.insertItem`: nothing while the crafter works or is covered, and only into an
    /// empty slot (it holds one item).
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        let idle = !matches!(
            util::get(&event.entity.nbt, PHASE),
            Some(Tag::String(s)) if s.to_string_lossy() != "IDLE"
        );
        let full = event.entity.items.first().is_some_and(Option::is_some);
        if !idle || util::boolean(&event.entity.nbt, COVERED) || full {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(|| update_tag(event.entity))
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block == ID {
            draw::submit(host, event);
        }
    }

    /// `MechanicalCrafterBlock.getStateForPlacement`: against another crafter (not sneaking) it
    /// takes that one's facing and a pointing for the face clicked; else it faces the player and
    /// points towards the face clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let clicked = util::dir(context.face);
        let towards_player = create_kinetics::looking(context.yaw, 0.0).opposite();
        // The state of every crafter, to tell which one the block clicked on is.
        let state = |facing: Dir, pointing: Pointing| {
            let facing = level.state_with_property(event.state, "facing", facing.name())?;
            level.state_with_property(facing, "pointing", pointing.name())
        };
        let placed_on = level.block_state(util::add([context.pos.x, context.pos.y, context.pos.z], clicked.opposite().normal()));
        let behind = [Dir::North, Dir::South, Dir::West, Dir::East].into_iter().find(|facing| {
            Pointing::ALL.iter().any(|p| state(*facing, *p) == Some(placed_on))
        });
        let (facing, pointing) = rules::placement(clicked, towards_player, context.sneaking, behind);
        state(facing, pointing).map(Some)
    }

    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if host.block(event.pos) != ID {
            return Flow::Continue;
        }
        click(host, event.pos, event.face)
    }

    /// The Wrench: a click on the edge of a face joins or parts the neighbour there
    /// (`EdgeInteractionHandler`); on the front it turns the pointing (`onWrenched`); sneaking
    /// picks the crafter up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, face) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let Some(face) = face else { return };
            if edge_click(host, pos, face) {
                return;
            }
            let Some(facing) = util::facing(&*host, pos, "facing") else { return };
            let pointing = host.block_property(pos, "pointing").as_deref().and_then(Pointing::from_name);
            if face == facing && let Some(pointing) = pointing {
                host.set_block_property(pos, "pointing", pointing.cycle().name());
            }
        })
    }

    /// `MechanicalCrafterBlock.onRemove`: the cover pops out, the grid is thrown out, and the
    /// group parts around the crafter.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return Flow::Continue;
        };
        let covered = host.mod_block_entity(pos).is_some_and(|e| util::boolean(&e.nbt, COVERED));
        if covered {
            util::spawn_item(host, util::centre(pos), &ItemStack::new(COVER, 1), [0.0; 3], 10);
        }
        world::eject_whole_grid(host, pos);
        world::part_from_neighbours(host, pos, facing);
        Flow::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_is_spread_over_the_wall() {
        assert_ne!(offset([0, 0, 0]), offset([1, 0, 0]));
        assert_eq!(offset([0, 0, 0]), 0);
    }

    #[test]
    fn the_container_has_one_slot_hoppers_fill_and_none_empty() {
        let c = container();
        assert_eq!(c.slots, 1);
        assert_eq!(c.inputs, vec![0]);
        assert!(c.outputs.is_empty());
        assert!(c.faces.iter().all(|f| f == &vec![0]));
    }

    #[test]
    fn a_fresh_crafter_is_idle_and_its_ready_to_be_powered() {
        let c: Crafter<ItemStack> = Crafter::new(Dir::North, Pointing::Up);
        assert_eq!(c.phase, Phase::Idle);
        assert!(c.was_powered);
    }
}

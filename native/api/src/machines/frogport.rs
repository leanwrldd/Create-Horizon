//! The Package Frogport (`create:package_frogport`) (`FrogportBlock`, `FrogportBlockEntity`,
//! `PackagePortBlockEntity`, `PackagePortTarget.ChainConveyorFrogportTarget`,
//! `FrogportRenderer`, `PackagePortAutomationInventoryWrapper`): a frog that works a Chain
//! Conveyor. It holds up to 18 packages. A package on the chain whose address matches the
//! frog's is caught with its tongue and kept; a package in the frog that doesn't match its
//! address is sent out onto the chain. The rules are `create_kinetics::rules::frogport` (what
//! to pull, the animation) and `rules::chain_conveyor` (where it attaches); the chain side is
//! `chain_conveyor.rs`.
//!
//! How it works here:
//! - Placed within 5 blocks of a Chain Conveyor's wheel or chain (at or below the point it works
//!   at), it attaches to the nearest point by itself. The Wrench on it picks the nearest point
//!   again. A renamed Name Tag on it sets its address (the tag's name); a Name Tag without a name
//!   clears it. Goggles on it put the goggles on its head.
//! - Its inventory is the block's container, so hoppers, funnels and chutes reach it from any
//!   side; the frog also pushes the packages that match its address into the inventory below it
//!   (a chest, a Packager) and pulls the ones that don't from its own inventory and from the one
//!   below, every 10 ticks, and when a Packager below it finishes a box.
//! - A package right-clicked on it goes into its inventory (to be sent); an empty hand shows its
//!   address, target and packages; a sneaking empty hand takes the packages out.
//!
//! Known differences from Create: see `rules/frogport.rs`. One more: Create's automation wrapper
//! lets a hopper take only packages that match the port's address; here the container is
//! plain (any package can be taken out, only a non-matching one can be put in).

use create_kinetics::rules::chain_conveyor::{self as chain, Target};
use create_kinetics::rules::frogport::{self as rules, Effect, Port};
use create_kinetics::rules::package as package_rules;
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, ContainerAcceptsEvent,
    DisplayContext, Flow, Hooks, Host, ItemStack, ModBlockEntity, ModContainer, PlaceBlockEvent,
    RightClickBlockEvent, StandaloneModelsEvent, Tag, Text, UseItemOnEvent,
};

use super::chute_inv as inv;
use super::package as items;
use super::packager_inv as target_inv;
use crate::util;

pub const ID: &str = "create:package_frogport";

const BODY: &str = "create:block/package_frogport/body";
const HEAD: &str = "create:block/package_frogport/head";
const HEAD_GOGGLES: &str = "create:block/package_frogport/head_goggles";
const TONGUE: &str = "create:block/package_frogport/tongue";

const TARGET: &str = "Target";
const ADDRESS: &str = "AddressFilter";
const ACCEPTS: &str = "AcceptsPackages";
const YAW: &str = "PlacedYaw";
const GOGGLES: &str = "Goggles";
const FAILED: &str = "FailedLastExport";
const START: &str = "AnimStart";
const DEPOSIT: &str = "Deposit";
const ANIMATED: &str = "AnimatedPackage";

struct Frogports;

pub fn hooks() -> Box<dyn Hooks> {
    // A Packager below a Frogport wakes it when it finishes a box (`wakeTheFrogs`).
    let _ = super::packager::WAKE_FROGS.set(wake);
    Box::new(Frogports)
}

// --- the saved form -----------------------------------------------------------------------------

/// `ChainConveyorFrogportTarget.CODEC`.
fn target_tag(target: &Target) -> Tag {
    let mut tag = Compound::new();
    tag.insert("type".into(), Tag::String("create:chain_conveyor".into()));
    tag.insert("relative_pos".into(), Tag::IntArray(target.relative_pos.to_vec()));
    tag.insert("chain_pos".into(), Tag::Float(target.chain_pos.to_bits()));
    if let Some(connection) = target.connection {
        tag.insert("connection".into(), Tag::IntArray(connection.to_vec()));
    }
    tag.insert("flipped".into(), Tag::Byte(i8::from(target.flipped)));
    Tag::Compound(tag)
}

fn pos_of(tag: Option<&Tag>) -> Option<Pos> {
    match tag {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// The target a port's data holds, if any.
pub fn target_of(nbt: &Compound) -> Option<Target> {
    let tag = util::get(nbt, TARGET)?;
    Some(Target {
        relative_pos: pos_of(tag.get("relative_pos"))?,
        chain_pos: match tag.get("chain_pos") {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            _ => 0.0,
        },
        connection: pos_of(tag.get("connection")),
        flipped: matches!(tag.get("flipped"), Some(Tag::Byte(1))),
    })
}

/// `PackagePortBlockEntity.getFilterString`: the address filter, or none when the port doesn't
/// accept packages.
pub fn filter_of(nbt: &Compound) -> Option<String> {
    let accepts = !util::has(nbt, ACCEPTS) || util::boolean(nbt, ACCEPTS);
    accepts.then(|| match util::get(nbt, ADDRESS) {
        Some(Tag::String(text)) => text.to_string_lossy(),
        _ => String::new(),
    })
}

/// The port as the rules see it: the container's slots and the animation.
fn read(entity: &ModBlockEntity) -> Port<ItemStack> {
    let mut port = Port::default();
    for (index, slot) in entity.items.iter().enumerate().take(rules::SLOTS) {
        port.slots[index].clone_from(slot);
    }
    let nbt = &entity.nbt;
    if util::has(nbt, START) && util::long(nbt, START) >= 0 {
        port.animation = Some(rules::Animation {
            start: util::long(nbt, START),
            depositing: util::boolean(nbt, DEPOSIT),
            item: util::get(nbt, ANIMATED).and_then(util::stack_of),
        });
    }
    port.failed_last_export = util::boolean(nbt, FAILED);
    port
}

/// Writes the port back, only what differs. Whether anything did.
fn store(entity: &mut ModBlockEntity, port: &Port<ItemStack>) -> bool {
    let mut changed = false;
    if entity.items.len() != rules::SLOTS || entity.items.iter().zip(&port.slots).any(|(a, b)| a != b) {
        entity.items = port.slots.clone();
        changed = true;
    }
    let (start, deposit, animated) = match &port.animation {
        Some(a) => (a.start, a.depositing, a.item.as_ref().map_or_else(|| Tag::Compound(Compound::new()), util::stack_tag)),
        None => (-1, false, Tag::Compound(Compound::new())),
    };
    let wanted = [
        (START, Tag::Long(start)),
        (DEPOSIT, Tag::Byte(i8::from(deposit))),
        (ANIMATED, animated),
        (FAILED, Tag::Byte(i8::from(port.failed_last_export))),
    ];
    for (key, value) in wanted {
        if util::differs(&entity.nbt, key, &value) {
            util::put(&mut entity.nbt, key, value);
            changed = true;
        }
    }
    changed
}

fn matches_own(nbt: &Compound, stack: &ItemStack) -> Option<bool> {
    filter_of(nbt).map(|filter| package_rules::match_address(&items::address(stack), &filter))
}

fn conveyor_of(pos: Pos, target: &Target) -> Pos {
    util::add(pos, target.relative_pos)
}

// --- what the chain and the packager ask -----------------------------------------------------------

/// `ChainConveyorBlockEntity.exportToPort`: the frog at `pos` catches `item` off the chain, if it
/// isn't busy or full. Whether it did.
pub fn catch(host: &mut dyn Host, pos: Pos, item: &ItemStack) -> bool {
    if host.block(pos) != ID || !items::is_package(item) {
        return false;
    }
    let now = util::game_time(&*host);
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    let mut port = read(&entity);
    if port.animating(now) || port.is_backed_up() {
        return false;
    }
    port.start_animation(item.clone(), false, now);
    host.change_mod_block_entity(pos, &mut |e| {
        store(e, &port);
    });
    host.block_entity_sync(pos);
    host.play_sound("create:frogport_open", util::centre(pos), 4, 1.0, 1.0);
    host.play_sound("create:frogport_catch", util::centre(pos), 4, 1.0, 1.0);
    true
}

/// `PackagerBlockEntity.wakeTheFrogs`: the frog above a packager pulls at once.
fn wake(host: &mut dyn Host, pos: Pos) {
    if host.block(pos) != ID {
        return;
    }
    let now = util::game_time(&*host);
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut port = read(&entity);
    if port.animating(now) {
        return;
    }
    if try_pull(host, pos, &entity, &mut port, now) {
        host.change_mod_block_entity(pos, &mut |e| {
            store(e, &port);
        });
        host.block_entity_sync(pos);
    }
}

// --- working ------------------------------------------------------------------------------------------

/// `tryPullingFromOwnAndAdjacentInventories`: starts sending the first package that should go.
fn try_pull(host: &mut dyn Host, pos: Pos, entity: &ModBlockEntity, port: &mut Port<ItemStack>, now: i64) -> bool {
    let Some(target) = target_of(&entity.nbt) else {
        return false;
    };
    let conveyor = conveyor_of(pos, &target);
    let sample = ItemStack { item: package_rules::default_box().into(), count: 1, damage: 0, extra: None };
    if !chain_conveyor_accepts(host, conveyor, &target, &sample) {
        return false;
    }
    let nbt = &entity.nbt;
    // Its own inventory.
    let own = port.first_slot(&mut |stack| rules::pulls(items::is_package(stack), matches_own(nbt, stack), false));
    if let Some(slot) = own {
        let boxed = port.slots[slot].take().expect("the slot held the box");
        port.start_animation(boxed, true, now);
        return true;
    }
    // The inventory below.
    let below = util::add(pos, Dir::Down.normal());
    let from_packager = target_inv::is_packager(&host.block(below));
    let Some(mut held) = (if from_packager { inv::items_of(host, below) } else { target_inv::read(host, below) }) else {
        return false;
    };
    held.sort_by_key(|(slot, _)| *slot);
    for (slot, stack) in held {
        if !rules::pulls(items::is_package(&stack), matches_own(nbt, &stack), from_packager) {
            continue;
        }
        if target_inv::take(host, below, Dir::Up, slot, 1, &stack) != 1 {
            continue;
        }
        let mut boxed = stack;
        boxed.count = 1;
        port.start_animation(boxed, true, now);
        return true;
    }
    false
}

fn chain_conveyor_accepts(host: &mut dyn Host, conveyor: Pos, target: &Target, sample: &ItemStack) -> bool {
    super::chain_conveyor::export_from_port(host, conveyor, target, sample, true)
}

/// `tryPushingToAdjacentInventories`: the packages that match the port's address go into the
/// inventory below.
fn try_push(host: &mut dyn Host, pos: Pos, entity: &ModBlockEntity, port: &mut Port<ItemStack>) {
    port.failed_last_export = false;
    if port.filled() == 0 {
        return;
    }
    let below = util::add(pos, Dir::Down.normal());
    if host.block(below) == ID || !inv::is_inventory(host, below) {
        return;
    }
    for slot in 0..port.slots.len() {
        let Some(stack) = port.slots[slot].clone() else {
            continue;
        };
        if !rules::automation_can_extract(items::is_package(&stack), matches_own(&entity.nbt, &stack)) {
            continue;
        }
        match inv::insert(host, below, Dir::Up, &stack, false) {
            Some(rest) if rest.count == 0 => port.slots[slot] = None,
            _ => port.failed_last_export = true,
        }
    }
}

fn drop_box(host: &mut dyn Host, pos: Pos, boxed: &ItemStack) {
    util::spawn_item(host, util::centre(pos), boxed, [0.0; 3], 10);
}

// --- the hooks ---------------------------------------------------------------------------------------

impl Hooks for Frogports {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [BODY, HEAD, HEAD_GOGGLES, TONGUE] {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// The inventory: 18 slots, reachable from every side.
    fn block_entity_container(&self, _: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        (event.block == ID).then(|| {
            let all: Vec<usize> = (0..rules::SLOTS).collect();
            ModContainer { slots: rules::SLOTS, faces: [(); 6].map(|()| all.clone()), inputs: all.clone(), outputs: all, menu: None }
        })
    }

    /// `PackagePortAutomationInventoryWrapper.insertItem`: only packages that don't match the
    /// port's own address go in (they are the ones to send).
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        if event.entity.id != ID {
            return Flow::Continue;
        }
        let stack = event.stack;
        if rules::automation_can_insert(items::is_package(stack), matches_own(&event.entity.nbt, stack)) {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let now = util::game_time(host);
        let mut port = read(event.entity);
        let before = port.clone();
        let target = target_of(&event.entity.nbt);

        for effect in port.tick(now) {
            match effect {
                Effect::Send(boxed) => {
                    let sent = target
                        .as_ref()
                        .is_some_and(|t| super::chain_conveyor::export_from_port(host, conveyor_of(pos, t), t, &boxed, false));
                    if sent {
                        host.play_sound("create:frogport_deposit", util::centre(pos), 4, 1.0, 1.0);
                    } else {
                        drop_box(host, pos, &boxed);
                    }
                }
                Effect::Receive(boxed) => {
                    if let Err(boxed) = port.insert(boxed) {
                        drop_box(host, pos, &boxed);
                    }
                }
            }
        }

        // `lazyTick`: registers with the conveyor, then works.
        if (now + i64::from(pos[0] + pos[1] + pos[2]).rem_euclid(rules::LAZY_PERIOD)) % rules::LAZY_PERIOD == 0 {
            if let Some(target) = &target {
                if let Some(filter) = filter_of(&event.entity.nbt) {
                    let conveyor = conveyor_of(pos, target);
                    let offset = [-target.relative_pos[0], -target.relative_pos[1], -target.relative_pos[2]];
                    super::chain_conveyor::register_port(&*host, conveyor, offset, target, &filter);
                }
            }
            if !port.animating(now) {
                try_push(host, pos, event.entity, &mut port);
                let snapshot = event.entity.clone();
                try_pull(host, pos, &snapshot, &mut port, now);
            }
        }

        if port != before && store(event.entity, &port) {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        for key in [TARGET, ADDRESS, ANIMATED] {
            if let Some(value) = util::get(nbt, key) {
                tag.insert(key.into(), value.clone());
            }
        }
        tag.insert(START.into(), Tag::Long(if util::has(nbt, START) { util::long(nbt, START) } else { -1 }));
        for key in [DEPOSIT, GOGGLES] {
            tag.insert(key.into(), Tag::Byte(i8::from(util::boolean(nbt, key))));
        }
        tag.insert(YAW.into(), Tag::Float(crate::float(nbt, YAW).unwrap_or(0.0).to_bits()));
        Some(tag)
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block == ID {
            *event.bounds = [-6.0, -2.0, -6.0, 7.0, 7.0, 7.0];
        }
    }

    /// `FrogportRenderer`: the body, the head tipping up, the tongue and the package in it.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let nbt = &event.entity.nbt;
        let target = target_of(nbt);
        let port = {
            let mut port: Port<ItemStack> = Port::default();
            if util::has(nbt, START) && util::long(nbt, START) >= 0 {
                port.animation = Some(rules::Animation {
                    start: util::long(nbt, START),
                    depositing: util::boolean(nbt, DEPOSIT),
                    item: util::get(nbt, ANIMATED).and_then(util::stack_of),
                });
            }
            port
        };
        let now = event.draw.game_time as i64;
        let partial = event.draw.partial;
        let pos = event.pos;
        let progress = port.progress(now, partial);
        let depositing = port.animation.as_ref().is_some_and(|a| a.depositing);
        let goggles = util::boolean(nbt, GOGGLES);
        let passive_yaw = crate::float(nbt, YAW).unwrap_or(0.0);

        // The target's direction from the middle of the block, lowered unless a box is being sent.
        let mut diff = [0.0; 3];
        if let Some(target) = &target {
            if let Some(at) = super::chain_conveyor::exact_location(host, pos, target) {
                let lower = if progress.is_some() && depositing { 0.0 } else { 0.75 };
                diff = [at[0] - (f64::from(pos[0]) + 0.5), at[1] - lower - (f64::from(pos[1]) + 0.5), at[2] - (f64::from(pos[2]) + 0.5)];
            }
        }
        let yaw = if target.is_some() { rules::yaw_towards(diff[0], diff[2]) } else { passive_yaw };
        let pose = rules::pose(diff, progress, depositing, 0.0);

        let draw = &mut *event.draw;
        const CENTRE: [f32; 3] = [0.5; 3];
        const UP: [f32; 3] = [0.0, 1.0, 0.0];
        const PIVOT: [f32; 3] = [0.5, 10.0 / 16.0, 11.0 / 16.0];
        draw.push();
        draw.rotate_about(UP, yaw, CENTRE);
        host.draw_model(draw, BODY, 0xFF_FFFF);
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], pose.head_pitch, PIVOT);
        host.draw_model(draw, if goggles { HEAD_GOGGLES } else { HEAD }, 0xFF_FFFF);
        draw.pop();
        if pose.tongue_length > 0.0 {
            draw.push();
            draw.translate(PIVOT);
            draw.rotate_about([1.0, 0.0, 0.0], pose.tongue_pitch, [0.0; 3]);
            draw.scale([1.0, 1.0, pose.tongue_length / (7.0 / 16.0)]);
            draw.translate(PIVOT.map(|v| -v));
            host.draw_model(draw, TONGUE, 0xFF_FFFF);
            draw.pop();
        }
        draw.pop();

        // The package on the tongue.
        if let (Some(animation), Some(progress)) = (&port.animation, progress) {
            if let Some(boxed) = &animation.item {
                if pose.scale >= 0.45 {
                    let norm = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt().max(1e-6);
                    let along = f64::from(pose.item_distance);
                    let lowered = if depositing { 0.75 } else { 0.0 };
                    draw.push();
                    draw.translate([
                        0.5 + (diff[0] / norm * along) as f32,
                        0.5 + 3.0 / 16.0 + (diff[1] / norm * along - lowered) as f32 - 0.2,
                        0.5 + (diff[2] / norm * along) as f32,
                    ]);
                    draw.scale([1.49 * pose.scale; 3]);
                    host.draw_item(draw, boxed, DisplayContext::Fixed, None, 0);
                    draw.pop();
                }
            }
            let _ = progress;
        }
        draw.still();
    }

    /// `FrogportBlock.setPlacedBy`: faces the placer; and the nearest chain point is its target.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let pos = event.pos;
        let yaw = host
            .acting_player()
            .map(|p| rules::placed_yaw(f64::from(pos[0]) + 0.5 - p.position[0], f64::from(pos[2]) + 0.5 - p.position[2]));
        let target = chain::nearest_target(&super::chain_conveyor::candidates_near(&*host, pos));
        host.change_mod_block_entity(pos, &mut |entity| {
            if let Some(yaw) = yaw {
                crate::set_float(&mut entity.nbt, YAW, yaw);
            }
            if let Some(target) = &target {
                util::put(&mut entity.nbt, TARGET, target_tag(target));
            }
        });
        host.block_entity_sync(pos);
        if let Some(player) = host.acting_player() {
            let line = if target.is_some() { "Frogport attached to a chain" } else { "No chain in range: use the Wrench near one" };
            host.action_bar(player.uuid, Text::literal(line.to_owned()));
        }
    }

    /// A broken port drops its packages (the container's items drop on their own) and tells its
    /// conveyor it is gone.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut horizon_client::mod_api::DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        if let Some(entity) = host.mod_block_entity(pos) {
            if let (Some(target), Some(filter)) = (target_of(&entity.nbt), filter_of(&entity.nbt)) {
                let offset = [-target.relative_pos[0], -target.relative_pos[1], -target.relative_pos[2]];
                super::chain_conveyor::deregister_port(&*host, conveyor_of(pos, &target), offset, &filter);
            }
            // A box in the middle of the animation.
            if let Some(animation) = read(&entity).animation {
                if let Some(boxed) = animation.item {
                    drop_box(host, pos, &boxed);
                }
            }
        }
        Flow::Continue
    }

    /// `PackagePortBlockEntity.use` with an empty hand.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR
            || host.held_stack(player.uuid).is_some()
            || host.player_equipment(player.uuid, horizon_client::mod_api::EquipmentSlot::OffHand).is_some()
        {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        if player.sneaking {
            let mut port = read(&entity);
            let mut given = 0;
            for slot in port.slots.iter_mut() {
                if let Some(boxed) = slot.take() {
                    given += 1;
                    if let Some(rest) = host.give_stack(player.uuid, boxed) {
                        util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
                    }
                }
            }
            if given > 0 {
                host.change_mod_block_entity(pos, &mut |e| {
                    store(e, &port);
                });
                host.block_entity_sync(pos);
            }
            host.action_bar(player.uuid, Text::literal(format!("Took {given} packages")));
            return Flow::Cancel;
        }
        host.action_bar(player.uuid, Text::literal(describe(&entity)));
        Flow::Cancel
    }

    /// A package goes into it; the Wrench picks the chain point again; a Name Tag sets the
    /// address; goggles go on its head.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let stack = event.stack;
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if &*stack.item == util::WRENCH {
            if player.sneaking {
                return util::wrench(host, pos, |_| {});
            }
            if !util::first_click(host, player.uuid, pos) {
                return Flow::Cancel;
            }
            let target = chain::nearest_target(&super::chain_conveyor::candidates_near(&*host, pos));
            let line = match &target {
                Some(_) => "Frogport attached to the nearest chain point",
                None => "No chain in range",
            };
            if let Some(target) = target {
                rebind(host, pos, Some(&target), None);
            }
            host.action_bar(player.uuid, Text::literal(line.to_owned()));
            return Flow::Cancel;
        }
        let consumed = matches!(&*stack.item, "minecraft:name_tag" | "create:goggles") || items::is_package(stack);
        if !consumed {
            return Flow::Continue;
        }
        if player.sneaking {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if &*stack.item == "minecraft:name_tag" {
            let name = stack.data().and_then(|d| d.custom_name_plain()).unwrap_or_default();
            rebind(host, pos, None, Some(&name));
            let line = if name.is_empty() { "Address cleared".to_owned() } else { format!("Address: {name}") };
            host.action_bar(player.uuid, Text::literal(line));
        } else if &*stack.item == "create:goggles" {
            host.change_mod_block_entity(pos, &mut |e| {
                if !util::boolean(&e.nbt, GOGGLES) {
                    util::put(&mut e.nbt, GOGGLES, Tag::Byte(1));
                }
            });
            host.block_entity_sync(pos);
        } else if let Some(entity) = host.mod_block_entity(pos) {
            let mut port = read(&entity);
            let mut one = stack.clone();
            one.count = 1;
            if port.insert(one).is_ok() {
                host.change_mod_block_entity(pos, &mut |e| {
                    store(e, &port);
                });
                host.block_entity_sync(pos);
                items::shrink_held(player.uuid, &stack.item);
            }
        }
        Flow::Cancel
    }
}

/// `PackagePortBlockEntity.filterChanged` and a new target: the port leaves its old conveyor's
/// tables and the new data is saved (`target`, or `address` where given).
fn rebind(host: &mut dyn Host, pos: Pos, target: Option<&Target>, address: Option<&str>) {
    if let Some(entity) = host.mod_block_entity(pos) {
        if let (Some(old), Some(filter)) = (target_of(&entity.nbt), filter_of(&entity.nbt)) {
            let offset = [-old.relative_pos[0], -old.relative_pos[1], -old.relative_pos[2]];
            super::chain_conveyor::deregister_port(&*host, conveyor_of(pos, &old), offset, &filter);
        }
    }
    host.change_mod_block_entity(pos, &mut |e| {
        if let Some(target) = target {
            util::put(&mut e.nbt, TARGET, target_tag(target));
        }
        if let Some(address) = address {
            util::put(&mut e.nbt, ADDRESS, Tag::String(address.into()));
        }
    });
    host.block_entity_sync(pos);
}

/// What an empty-handed click says.
fn describe(entity: &ModBlockEntity) -> String {
    let port = read(entity);
    let address = filter_of(&entity.nbt).filter(|a| !package_rules::is_blank(a));
    let attached = target_of(&entity.nbt).map_or("no chain".to_owned(), |t| match t.connection {
        Some(_) => "a chain".to_owned(),
        None => "a wheel".to_owned(),
    });
    format!(
        "Frogport: {}, on {attached}, {} of {} packages",
        address.map_or("no address".to_owned(), |a| format!("\u{2192} {a}")),
        port.filled(),
        rules::SLOTS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_survives_being_saved_and_read() {
        let target = Target { relative_pos: [-3, 1, 2], chain_pos: 4.5, connection: Some([10, 0, 0]), flipped: true };
        let mut nbt = Compound::new();
        util::put(&mut nbt, TARGET, target_tag(&target));
        assert_eq!(target_of(&nbt), Some(target));
        let wheel = Target { relative_pos: [1, 0, 0], chain_pos: 90.0, connection: None, flipped: false };
        util::put(&mut nbt, TARGET, target_tag(&wheel));
        assert_eq!(target_of(&nbt), Some(wheel));
        assert_eq!(target_of(&Compound::new()), None);
    }

    #[test]
    fn the_filter_is_the_address_unless_the_port_takes_no_packages() {
        let mut nbt = Compound::new();
        assert_eq!(filter_of(&nbt), Some(String::new()));
        util::put(&mut nbt, ADDRESS, Tag::String("Farm".into()));
        assert_eq!(filter_of(&nbt), Some("Farm".to_owned()));
        util::put(&mut nbt, ACCEPTS, Tag::Byte(0));
        assert_eq!(filter_of(&nbt), None);
    }
}

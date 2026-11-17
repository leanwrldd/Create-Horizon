//! Packages and the Package Filter (`PackageItem`, `PackageStyles`, `PackageFilterItem`): the
//! fourteen cardboard boxes that hold up to nine stacks and an address, and the filter that passes
//! packages by address. The rules (styles, address matching, throwing maths) are
//! `create_kinetics::rules::package`; this reads a package's components into them and handles
//! the item's uses. The block that fills and empties packages is `packager.rs`.
//!
//! A package keeps its contents, address and order data in registered Horizon components
//! (`Hooks::component_types`), under the ids Create's `AllDataComponents` use
//! (`create:package_contents`, `create:package_address`, `create:package_order_data`,
//! `create:package_order_context`), saved as plain NBT: the contents as a list of `{slot, item}`
//! (as the List Filter's), the order data with Create's codec field names.
//!
//! Using a package (`PackageItem.use`, `useOn`, `releaseUsing`):
//! - Sneaking, in the air or on a block: opens it, and its items go to the player's inventory
//!   (what doesn't fit drops at their feet).
//! - Holding the use button charges a throw, as a bow does; letting go throws it (at least a tenth
//!   of full speed).
//! - Clicking a block (not sneaking): sets the package down on the block as an item.
//!
//! Known differences from Create:
//! - Create's package entity doesn't exist (Horizon has no entity type of a mod's own): a
//!   thrown or set-down package is an **item entity** holding the package, which lands, rolls
//!   and is picked up like any item. It isn't pushed along by belts as a bigger entity or
//!   carried by Create's package-specific handlers (a Frogport doesn't catch one; the Chain
//!   Conveyor and Packager take it as an item). The thrown package doesn't remember who tossed it.
//! - A spawn egg inside a package isn't spawned when the box is opened: the egg goes to the
//!   player like any other item.
//! - Opening shows no particles.
//! - The address (Create: a text field in the Package Filter's and the ports' screens) is set with
//!   the `/address <text>` command for a Package Filter or a package in the main hand (no text
//!   clears it), by a sign on a Packager, and by a renamed Name Tag on a Frogport or Postbox.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::package::{self as rules, Big, Context, Contents, Craft, OrderData};
use horizon_client::mod_api::{
    ComponentTypesEvent, Compound, Flow, Hooks, Host, InventoryTickEvent, ItemStack, NativeCommand,
    PerformCommandEvent, RegisterCommandsEvent, ReleaseUsingEvent, Tag, Text, TooltipEvent,
    UseAnimation, UseAnimationEvent, UseDurationEvent, UseItemEvent, UseItemOnEvent,
};

use crate::util;

pub const FILTER: &str = "create:package_filter";

struct Packages;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Packages)
}

// --- reading and writing a package ------------------------------------------------------------

pub fn is_package(stack: &ItemStack) -> bool {
    rules::is_package(&stack.item)
}

fn component<'a>(stack: &'a ItemStack, id: &str) -> Option<&'a Tag> {
    stack.data()?.other(id)
}

/// `PackageItem.getAddress` of a stack that has the component (a Package Filter with none set,
/// or a package never addressed, is `None`).
pub fn address_of(stack: &ItemStack) -> Option<String> {
    match component(stack, rules::ADDRESS) {
        Some(Tag::String(address)) => Some(address.to_string_lossy()),
        _ => None,
    }
}

/// `PackageItem.getAddress`: the address, or an empty one.
pub fn address(stack: &ItemStack) -> String {
    address_of(stack).unwrap_or_default()
}

/// `PackageItem.addAddress`.
pub fn set_address(host: &dyn Host, stack: &mut ItemStack, address: &str) {
    let _ = host.set_component(stack, rules::ADDRESS, Tag::String(address.into()));
}

/// `PackageItem.clearAddress`.
pub fn clear_address(host: &dyn Host, stack: &mut ItemStack) {
    let _ = host.remove_component(stack, rules::ADDRESS);
}

/// The key a stack is compared by in the rules: the item and its components, count 1.
pub fn key(stack: &ItemStack) -> ItemStack {
    let mut key = stack.clone();
    key.count = 1;
    key
}

/// `PackageItem.getContents`: the nine slots.
pub fn slots_of(stack: &ItemStack) -> Vec<Option<ItemStack>> {
    let mut slots = vec![None; rules::SLOTS];
    if let Some(Tag::List(entries)) = component(stack, rules::CONTENTS) {
        for entry in entries {
            let index = match entry.get("slot") {
                Some(Tag::Int(slot)) => *slot as usize,
                _ => continue,
            };
            if index < rules::SLOTS {
                slots[index] = entry.get("item").and_then(util::stack_of);
            }
        }
    }
    slots
}

/// The stacks in a package, in slot order (`ItemHelper.getNonEmptyStacks`).
pub fn contents_of(stack: &ItemStack) -> Vec<ItemStack> {
    slots_of(stack).into_iter().flatten().collect()
}

/// A package's contents as the rules' slots.
pub fn contents_big(stack: &ItemStack) -> Vec<Big<ItemStack>> {
    contents_of(stack).iter().map(|s| (key(s), s.count)).collect()
}

fn contents_tag(contents: &Contents<ItemStack>) -> Tag {
    Tag::List(
        contents
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| {
                let (item, count) = held.as_ref()?;
                let mut stack = item.clone();
                stack.count = *count;
                let mut entry = Compound::new();
                entry.insert("slot".into(), Tag::Int(slot as i32));
                entry.insert("item".into(), util::stack_tag(&stack));
                Some(Tag::Compound(entry))
            })
            .collect(),
    )
}

/// `PackageItem.containing(ItemStackHandler)`: a box of a random style holding `contents`.
pub fn containing(host: &dyn Host, contents: &Contents<ItemStack>) -> ItemStack {
    let item = random_box();
    let mut stack = ItemStack { item: item.into(), count: 1, damage: 0, extra: None };
    let _ = host.set_component(&mut stack, rules::CONTENTS, contents_tag(contents));
    stack
}

/// `PackageItem.getHookDistance` of an item, in blocks.
pub fn style_hook(item: &str) -> f32 {
    rules::hook_distance(item)
}

/// `PackageStyles.getRandomBox`.
pub fn random_box() -> &'static str {
    rules::random_box(|bound| (util::random() % u64::from(bound.max(1))) as u32)
}

fn big_tag(entries: &[Big<ItemStack>]) -> Tag {
    let mut list = Vec::new();
    for (stack, count) in entries {
        let mut entry = Compound::new();
        let mut one = stack.clone();
        one.count = 1;
        entry.insert("item_stack".into(), util::stack_tag(&one));
        entry.insert("count".into(), Tag::Int(*count as i32));
        list.push(Tag::Compound(entry));
    }
    let mut order = Compound::new();
    order.insert("entries".into(), Tag::List(list));
    Tag::Compound(order)
}

fn big_of(tag: Option<&Tag>) -> Vec<Big<ItemStack>> {
    let Some(Tag::List(entries)) = tag.and_then(|t| t.get("entries")) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let stack = util::stack_of(entry.get("item_stack")?)?;
            let count = match entry.get("count") {
                Some(Tag::Int(count)) => (*count).max(0) as u32,
                _ => 0,
            };
            Some((key(&stack), count))
        })
        .collect()
}

/// `PackageOrderWithCrafts.CODEC`.
pub fn context_tag(context: &Context<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert("ordered_stacks".into(), big_tag(&context.stacks));
    let crafts = context
        .crafts
        .iter()
        .map(|craft| {
            let mut entry = Compound::new();
            entry.insert("pattern".into(), big_tag(&craft.pattern));
            entry.insert("count".into(), Tag::Int(craft.count as i32));
            Tag::Compound(entry)
        })
        .collect();
    tag.insert("ordered_crafts".into(), Tag::List(crafts));
    Tag::Compound(tag)
}

/// `PackageOrderWithCrafts.CODEC` read back (including the legacy `entries` form).
pub fn context_of(tag: &Tag) -> Option<Context<ItemStack>> {
    if let Some(stacks) = tag.get("ordered_stacks") {
        let crafts = match tag.get("ordered_crafts") {
            Some(Tag::List(list)) => list
                .iter()
                .map(|entry| Craft {
                    pattern: big_of(entry.get("pattern")),
                    count: match entry.get("count") {
                        Some(Tag::Int(count)) => (*count).max(0) as u32,
                        _ => 1,
                    },
                })
                .collect(),
            _ => Vec::new(),
        };
        return Some(Context { stacks: big_of(Some(stacks)), crafts });
    }
    // Legacy format (6.0.0 - 6.0.2): `entries` straight under the context.
    tag.get("entries")?;
    Some(Context { stacks: big_of(Some(tag)), crafts: Vec::new() })
}

fn order_tag(order: &OrderData<ItemStack>) -> Tag {
    let mut tag = Compound::new();
    tag.insert("order_id".into(), Tag::Int(order.order_id));
    tag.insert("link_index".into(), Tag::Int(order.link_index));
    tag.insert("is_final_link".into(), Tag::Byte(i8::from(order.is_final_link)));
    tag.insert("fragment_index".into(), Tag::Int(order.fragment_index));
    tag.insert("is_final".into(), Tag::Byte(i8::from(order.is_final)));
    if let Some(context) = &order.context {
        tag.insert("order_context".into(), context_tag(context));
    }
    Tag::Compound(tag)
}

/// `PackageItem.setOrder`.
pub fn set_order(host: &dyn Host, stack: &mut ItemStack, order: &OrderData<ItemStack>) {
    let _ = host.set_component(stack, rules::ORDER_DATA, order_tag(order));
}

/// `AllDataComponents.PACKAGE_ORDER_DATA` of a box, when it has one (a box that is a piece of an
/// order).
pub fn order_of(stack: &ItemStack) -> Option<OrderData<ItemStack>> {
    let tag = component(stack, rules::ORDER_DATA)?;
    let int = |name: &str| match tag.get(name) {
        Some(Tag::Int(value)) => *value,
        _ => 0,
    };
    let flag = |name: &str| matches!(tag.get(name), Some(Tag::Byte(1)));
    Some(OrderData {
        order_id: int("order_id"),
        link_index: int("link_index"),
        is_final_link: flag("is_final_link"),
        fragment_index: int("fragment_index"),
        is_final: flag("is_final"),
        context: tag.get("order_context").and_then(context_of),
    })
}

/// `PackageItem.getOrderContext`: the order data's context, else the box's own context component.
#[allow(dead_code)]
pub fn order_context_of(stack: &ItemStack) -> Option<Context<ItemStack>> {
    match order_of(stack) {
        Some(order) => order.context,
        None => component(stack, rules::ORDER_CONTEXT).and_then(context_of),
    }
}

/// `PackageItem.getOrderId`.
pub fn order_id(stack: &ItemStack) -> i32 {
    order_of(stack).map_or(-1, |order| order.order_id)
}

// --- uses -------------------------------------------------------------------------------------

/// What the selected stack of a player becomes on the next inventory tick (a player's own hook
/// can't change the stack in a hand).
enum Edit {
    /// One package less (the item guards against the player having changed slot or item).
    Shrink(String),
    /// A new address for the Package Filter or package in the main hand.
    Address(Option<String>),
}

static PENDING: Mutex<BTreeMap<u128, Vec<Edit>>> = Mutex::new(BTreeMap::new());
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

fn pending() -> std::sync::MutexGuard<'static, BTreeMap<u128, Vec<Edit>>> {
    PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn queue(uuid: u128, edit: Edit) {
    let mut pending = pending();
    pending.entry(uuid).or_default().push(edit);
    PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
}

const PLAYERS: usize = 7;
const NEUTRAL: usize = 6;

/// Blocks that take a package from the player's hand themselves (`useItemOn` of the Packager,
/// the Repackager, the Frogport and the Postboxes): the package's own click leaves them alone.
pub fn takes_packages(block: &str) -> bool {
    matches!(block, "create:packager" | "create:repackager" | "create:package_frogport")
        || block.strip_prefix("create:").is_some_and(|rest| rest.ends_with("_postbox"))
}

/// One package less in the main hand of `uuid`, on the next inventory tick (for a click that
/// puts a package into a block).
pub fn shrink_held(uuid: u128, item: &str) {
    queue(uuid, Edit::Shrink(item.to_owned()));
}

/// `PackageItem.open`: the contents go to the player and one box is used up.
fn open(host: &mut dyn Host, uuid: u128, position: [f64; 3], stack: &ItemStack) {
    for content in contents_of(stack) {
        if let Some(rest) = host.give_stack(uuid, content) {
            util::spawn_item(host, position, &rest, [0.0; 3], 0);
        }
    }
    host.play_sound("create:package_pop", position, PLAYERS, 1.0, 1.0);
    queue(uuid, Edit::Shrink(stack.item.to_string()));
}

/// The face's outward normal as a block offset.
fn normal(face: [i32; 3]) -> [i32; 3] {
    face
}

impl Hooks for Packages {
    /// `AllDataComponents.PACKAGE_*`.
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        for id in rules::COMPONENTS {
            if !event.types.iter().any(|t| t == id) {
                event.types.push(id.to_owned());
            }
        }
    }

    /// `PackageItem.getUseDuration`: held like a bow.
    fn use_duration(&self, _host: &dyn Host, event: &mut UseDurationEvent<'_>) {
        if rules::is_package(&event.stack.item) {
            event.duration = rules::USE_DURATION;
        }
    }

    /// `PackageItem.getUseAnimation`.
    fn use_animation(&self, _host: &dyn Host, event: &mut UseAnimationEvent<'_>) {
        if rules::is_package(&event.stack.item) {
            *event.animation = UseAnimation::Bow;
        }
    }

    /// `PackageItem.use`: sneaking opens the box; otherwise the throw starts charging.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if host.window() {
            return Flow::Continue;
        }
        let item = &event.stack;
        if rules::is_package(&item.item) {
            let Some(player) = host.acting_player() else {
                return Flow::Continue;
            };
            if !player.sneaking {
                return Flow::Continue;
            }
            if util::click_repeated_anywhere(host, player.uuid) {
                return Flow::Cancel;
            }
            let at = [player.position[0] as i32, player.position[1] as i32, player.position[2] as i32];
            util::click_taken(host, player.uuid, at);
            open(host, player.uuid, player.position, event.stack);
            return Flow::Cancel;
        }
        if &*item.item == FILTER {
            return self.filter_line(host, event.stack, None);
        }
        Flow::Continue
    }

    /// `PackageItem.useOn`: sneaking opens the box; else the box is set down on the block.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item == FILTER {
            let pos = event.pos;
            let block = host.block(pos);
            if takes_packages(&block) {
                return Flow::Continue;
            }
            return self.filter_line(host, event.stack, Some(pos));
        }
        if !rules::is_package(&event.stack.item) {
            return Flow::Continue;
        }
        let pos = event.pos;
        if takes_packages(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if player.sneaking {
            open(host, player.uuid, player.position, event.stack);
            return Flow::Cancel;
        }
        // `PackageEntity` set on the clicked face: here an item at the middle of the face.
        let face = normal(event.face);
        let at = [
            f64::from(pos[0]) + 0.5 + f64::from(face[0]) * 0.6,
            f64::from(pos[1]) + 0.5 + f64::from(face[1]) * 0.55 - if face[1] == 0 { 0.4 } else { 0.0 },
            f64::from(pos[2]) + 0.5 + f64::from(face[2]) * 0.6,
        ];
        let mut one = event.stack.clone();
        one.count = 1;
        util::spawn_item(host, at, &one, [0.0; 3], 10);
        queue(player.uuid, Edit::Shrink(event.stack.item.to_string()));
        Flow::Cancel
    }

    /// `PackageItem.releaseUsing`: the throw, after holding for as long as the charge needs.
    fn release_using(&self, host: &mut dyn Host, event: &mut ReleaseUsingEvent<'_>) {
        if host.window() || !rules::is_package(&event.stack.item) {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let held = rules::USE_DURATION - event.time_left;
        if held < 0 {
            return;
        }
        let velocity = rules::throw_velocity(held);
        let yaw = f64::from(player.yaw).to_radians();
        let pitch = f64::from(player.pitch).to_radians();
        let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let Some(motion) = rules::throw_motion(look, velocity) else {
            return;
        };
        host.play_sound("minecraft:entity.snowball.throw", player.position, NEUTRAL, 0.5, 0.5);
        // The box starts at the middle of the player's body, a motion ahead.
        let at = [player.position[0] + motion[0], player.position[1] + 0.9 + motion[1], player.position[2] + motion[2]];
        let mut one = event.stack.clone();
        one.count = 1;
        util::spawn_item(host, at, &one, motion, 20);
        if player.game_mode != util::CREATIVE {
            queue(player.uuid, Edit::Shrink(event.stack.item.to_string()));
        }
        event.used = true;
    }

    /// `PackageItem.appendHoverText` and `PackageFilterItem.makeSummary`.
    fn tooltip(&self, host: &dyn Host, event: &mut TooltipEvent<'_>) {
        let stack = event.stack;
        if &*stack.item == FILTER {
            let address = address(stack);
            if !rules::is_blank(&address) {
                // `-> ` in grey, the address in gold.
                event.lines.push((Text::literal(String::new()), u32::MAX));
                event.lines.push((Text::literal(format!("-> {address}")), 0xFFFF_AA00));
            }
            return;
        }
        if !rules::is_package(&stack.item) {
            return;
        }
        if let Some(address) = address_of(stack) {
            event.lines.push((Text::literal(format!("\u{2192} {address}")), 0xFFFF_AA00));
        }
        let (mut shown, mut skipped) = (0, 0);
        for held in contents_of(stack) {
            if held.item.ends_with("_spawn_egg") {
                continue;
            }
            if shown > 2 {
                skipped += 1;
                continue;
            }
            shown += 1;
            let name = held
                .data()
                .and_then(|data| data.custom_name_plain())
                .unwrap_or_else(|| create_kinetics::rules::filter::display_name(&held.item));
            event.lines.push((Text::literal(format!("{name} x{}", held.count)), 0xFFAA_AAAA));
        }
        if skipped > 0 {
            event.lines.push((Text::literal(format!("and {skipped} more...")), 0xFFAA_AAAA));
        }
        let _ = host;
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(NativeCommand::new("address"));
        event.commands.push(NativeCommand::new("address <text:text>"));
    }

    /// `/address <text>`: the stand-in for the address field of Create's Package Filter screen.
    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = (if line == "address" { Some("") } else { line.strip_prefix("address ") }) else {
            return Flow::Continue;
        };
        let uuid = event.player.uuid;
        let text = rest.trim().to_owned();
        let held = host
            .held_stack(uuid)
            .filter(|stack| &*stack.item == FILTER || rules::is_package(&stack.item));
        let reply = match held {
            None => "Hold a Package Filter (or a package) to set its address".to_owned(),
            Some(_) if text.is_empty() => {
                queue(uuid, Edit::Address(None));
                "Address cleared".to_owned()
            }
            Some(_) => {
                queue(uuid, Edit::Address(Some(text.clone())));
                format!("Address: {text}")
            }
        };
        host.action_bar(uuid, Text::literal(reply));
        Flow::Cancel
    }

    /// Applies the edits made in this player's hook to the selected stack.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || PENDING_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let edits = {
            let mut pending = pending();
            let edits = pending.remove(&player.uuid);
            PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
            edits
        };
        let Some(edits) = edits else {
            return;
        };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else {
            return;
        };
        for edit in edits {
            match edit {
                Edit::Shrink(item) if *stack.item == *item => stack.count = stack.count.saturating_sub(1),
                Edit::Address(address) if &*stack.item == FILTER || is_package(&stack) => match address {
                    Some(text) => set_address(&*host, &mut stack, &text),
                    None => clear_address(&*host, &mut stack),
                },
                _ => {}
            }
        }
        if let Some(slot) = host.carried_stack(event.slot) {
            *slot = stack;
        }
    }
}

impl Packages {
    /// A click with the Package Filter: shows its address.
    fn filter_line(&self, host: &mut dyn Host, stack: &ItemStack, pos: Option<[i32; 3]>) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if let Some(pos) = pos {
            if !util::first_click(host, player.uuid, pos) {
                return Flow::Cancel;
            }
        } else {
            if util::click_repeated_anywhere(host, player.uuid) {
                return Flow::Cancel;
            }
            let at = [player.position[0] as i32, player.position[1] as i32, player.position[2] as i32];
            util::click_taken(host, player.uuid, at);
        }
        let address = address(stack);
        let line = if rules::is_blank(&address) {
            "Package Filter: no address (set it with /address <text>)".to_owned()
        } else {
            format!("Package Filter: {address}")
        };
        host.action_bar(player.uuid, Text::literal(line));
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postboxes_and_ports_take_packages_themselves() {
        assert!(takes_packages("create:packager"));
        assert!(takes_packages("create:red_postbox"));
        assert!(takes_packages("create:package_frogport"));
        assert!(!takes_packages("minecraft:chest"));
    }

    #[test]
    fn an_order_context_round_trips() {
        let egg = ItemStack { item: "minecraft:egg".into(), count: 1, damage: 0, extra: None };
        let context = Context {
            stacks: vec![(egg.clone(), 20)],
            crafts: vec![Craft { pattern: vec![(egg, 2)], count: 3 }],
        };
        assert_eq!(context_of(&context_tag(&context)), Some(context));
    }
}

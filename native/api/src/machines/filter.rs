//! The List Filter (`create:filter`) and the Attribute Filter (`create:attribute_filter`)
//! (`FilterItem`, `ListFilterItem`, `AttributeFilterItem`, `FilterItemStack`): items that stand in
//! a filter slot and filter by what they hold. The matching and the attributes are
//! `create_kinetics::rules::filter`; this reads a filter's components into those types, answers
//! the questions an attribute asks of an item, and edits a filter with clicks.
//!
//! Where a filter is used: every place that takes an item as a filter asks
//! [`matches`] (`chute_inv::Filter::test`: the Smart Chute, Brass Funnels and Brass Belt Funnels,
//! the Deployer, the Smart Observer and the Threshold Switch, the Mechanical Arm's funnel check;
//! the Brass Tunnel's sides; the Placard; the Smart Fluid Pipe through [`matches_fluid`]).
//!
//! Editing (stand-in for Create's screens, which Horizon's mod API has no menu for; the lines of
//! the filter show on the action bar). With the filter in the main hand, on a block that doesn't
//! take a filter or in the air, and the other hand holding a reference item:
//! - List Filter: the reference item is added to the list (up to 18), or taken out if it is in it.
//! - Attribute Filter: the first attribute of the reference item that isn't in the list yet is
//!   added; sneaking adds its opposite ("is not ...").
//!
//! With the other hand empty: a click cycles the mode (List Filter: allow, allow and respect
//! data, deny, deny and respect data; Attribute Filter: any, all, deny); sneaking clears the filter.
//! The edited filter replaces the one in the hand on the next tick (a player's own hook can't
//! change the stack in a hand).
//!
//! Known differences from Create: see `rules/filter.rs`. One more here: the components are
//! registered Horizon components (`Hooks::component_types`), saved as plain NBT (the container
//! contents as a list of `{slot, item}`, the attributes as `{attribute: {type, value}, inverted}`),
//! not NeoForge data components, so a filter is not interchangeable with one made in Create.

use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::arm_items;
use create_kinetics::rules::filter::{
    self as rules, Attribute, AttributeFilter, Env, Facts, Fill, Filter, ListFilter, Mode,
    Singleton, Toggle,
};
use create_kinetics::rules::filter_items as tables;
use create_kinetics::rules::fluids;
use horizon_client::mod_api::{
    ComponentTypesEvent, Compound, EquipmentSlot, Flow, Hooks, Host, ItemStack,
    LevelTickPostEvent, NativeInventoryKind, Tag, Text, TooltipEvent, UseItemEvent, UseItemOnEvent,
};

use crate::{recipes, util};

pub const LIST: &str = "create:filter";
pub const ATTRIBUTE: &str = "create:attribute_filter";

/// Blocks that take a filter themselves: a click on one with a filter sets that block's filter.
const TAKES_A_FILTER: &[&str] = &[
    "create:smart_chute",
    "create:brass_funnel",
    "create:brass_belt_funnel",
    "create:brass_tunnel",
    "create:smart_fluid_pipe",
    "create:deployer",
    "create:content_observer",
    "create:stockpile_switch",
    "create:creative_crate",
    "create:placard",
];

struct FilterItems;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(FilterItems)
}

pub fn is_filter_item(item: &str) -> bool {
    item == LIST || item == ATTRIBUTE
}

// --- reading a filter ---------------------------------------------------------------------------

fn component<'a>(stack: &'a ItemStack, id: &str) -> Option<&'a Tag> {
    stack.data()?.other(id)
}

fn flag(stack: &ItemStack, id: &str) -> bool {
    matches!(component(stack, id), Some(Tag::Byte(1)))
}

fn text<'a>(tag: &'a Tag, key: &str) -> Option<String> {
    match tag.get(key) {
        Some(Tag::String(value)) => Some(value.to_string_lossy()),
        _ => None,
    }
}

/// `FilterItemStack.of`: the filter a stack stands for. A filter item with none of its components
/// set is an ordinary item (Create: `isComponentsPatchEmpty`).
pub fn read(stack: &ItemStack) -> Filter<ItemStack> {
    match &*stack.item {
        LIST if component(stack, rules::FILTER_ITEMS).is_some()
            || component(stack, rules::FILTER_ITEMS_BLACKLIST).is_some()
            || component(stack, rules::FILTER_ITEMS_RESPECT_NBT).is_some() =>
        {
            Filter::List(read_list(stack))
        }
        ATTRIBUTE
            if component(stack, rules::ATTRIBUTE_FILTER_WHITELIST_MODE).is_some()
                || component(stack, rules::ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES).is_some() =>
        {
            Filter::Attribute(read_attributes(stack))
        }
        _ => Filter::Item(stack.clone()),
    }
}

fn read_list(stack: &ItemStack) -> ListFilter<ItemStack> {
    ListFilter {
        items: list_items(stack).iter().map(read).collect(),
        respect_nbt: flag(stack, rules::FILTER_ITEMS_RESPECT_NBT),
        blacklist: flag(stack, rules::FILTER_ITEMS_BLACKLIST),
    }
}

/// The items of a List Filter, in slot order.
fn list_items(stack: &ItemStack) -> Vec<ItemStack> {
    let Some(Tag::List(entries)) = component(stack, rules::FILTER_ITEMS) else {
        return Vec::new();
    };
    let mut slots: Vec<(i32, ItemStack)> = entries
        .iter()
        .filter_map(|entry| {
            let slot = match entry.get("slot") {
                Some(Tag::Int(slot)) => *slot,
                _ => 0,
            };
            Some((slot, util::stack_of(entry.get("item")?)?))
        })
        .collect();
    slots.sort_by_key(|(slot, _)| *slot);
    slots.into_iter().map(|(_, item)| item).collect()
}

fn read_attributes(stack: &ItemStack) -> AttributeFilter {
    let mode = component(stack, rules::ATTRIBUTE_FILTER_WHITELIST_MODE)
        .and_then(|tag| match tag {
            Tag::String(name) => Mode::from_name(&name.to_string_lossy()),
            _ => None,
        })
        .unwrap_or(Mode::WhitelistDisj);
    AttributeFilter { mode, tests: attribute_list(stack) }
}

fn attribute_list(stack: &ItemStack) -> Vec<(Attribute, bool)> {
    let Some(Tag::List(entries)) = component(stack, rules::ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let attribute = entry.get("attribute")?;
            let parsed = Attribute::parse(&text(attribute, "type")?, text(attribute, "value").as_deref())?;
            let inverted = matches!(entry.get("inverted"), Some(Tag::Byte(1)));
            Some((parsed, inverted))
        })
        .collect()
}

// --- writing a filter ---------------------------------------------------------------------------

fn items_tag(items: &[ItemStack]) -> Tag {
    Tag::List(
        items
            .iter()
            .enumerate()
            .map(|(slot, item)| {
                let mut entry = Compound::new();
                entry.insert("slot".into(), Tag::Int(slot as i32));
                entry.insert("item".into(), util::stack_tag(item));
                Tag::Compound(entry)
            })
            .collect(),
    )
}

fn attributes_tag(tests: &[(Attribute, bool)]) -> Tag {
    Tag::List(
        tests
            .iter()
            .map(|(attribute, inverted)| {
                let mut body = Compound::new();
                body.insert("type".into(), Tag::String(attribute.type_id().as_str().into()));
                if let Some(value) = attribute.value() {
                    body.insert("value".into(), Tag::String(value.as_str().into()));
                }
                let mut entry = Compound::new();
                entry.insert("attribute".into(), Tag::Compound(body));
                entry.insert("inverted".into(), Tag::Byte(i8::from(*inverted)));
                Tag::Compound(entry)
            })
            .collect(),
    )
}

fn write_list(host: &dyn Host, stack: &mut ItemStack, items: &[ItemStack], blacklist: bool, respect: bool) {
    let _ = host.set_component(stack, rules::FILTER_ITEMS, items_tag(items));
    let _ = host.set_component(stack, rules::FILTER_ITEMS_BLACKLIST, Tag::Byte(i8::from(blacklist)));
    let _ = host.set_component(stack, rules::FILTER_ITEMS_RESPECT_NBT, Tag::Byte(i8::from(respect)));
}

fn write_attributes(host: &dyn Host, stack: &mut ItemStack, mode: Mode, tests: &[(Attribute, bool)]) {
    let _ = host.set_component(
        stack,
        rules::ATTRIBUTE_FILTER_WHITELIST_MODE,
        Tag::String(mode.name().into()),
    );
    let _ = host.set_component(stack, rules::ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES, attributes_tag(tests));
}

// --- the game's answers ---------------------------------------------------------------------------

/// What matching asks of the game, for filters made of item stacks.
struct World<'a> {
    host: Option<&'a dyn Host>,
}

impl Env<ItemStack> for World<'_> {
    fn same_item(&self, filter: &ItemStack, stack: &ItemStack) -> bool {
        filter.item == stack.item
    }

    fn same_components(&self, filter: &ItemStack, stack: &ItemStack) -> bool {
        util::same_item(filter, stack)
    }

    fn has(&self, attribute: &Attribute, stack: &ItemStack) -> bool {
        rules::applies(attribute, &ItemFacts { host: self.host, stack })
    }
}

/// `FilterItemStack.of(filter).test(world, stack)`: whether `stack` passes the filter item
/// `filter` (an ordinary item passes its own item).
pub fn matches(host: &dyn Host, filter: &ItemStack, stack: &ItemStack) -> bool {
    if !is_filter_item(&filter.item) {
        return filter.item == stack.item;
    }
    read(filter).test(&World { host: Some(host) }, stack, false)
}

/// [`matches`] without the host: an Attribute Filter's attributes that ask the game (tags, recipes,
/// whether an item is placeable) don't match; everything else is answered as usual.
pub fn matches_static(filter: &ItemStack, stack: &ItemStack) -> bool {
    if !is_filter_item(&filter.item) {
        return filter.item == stack.item;
    }
    read(filter).test(&World { host: None }, stack, false)
}

/// `FilterItemStack.test(world, FluidStack)`: whether the fluid passes the filter. A List Filter
/// passes a fluid that one of its items holds (the buckets, here); an Attribute Filter never does.
pub fn matches_fluid(filter: &ItemStack, fluid: &str) -> bool {
    // `GenericItemEmptying.emptyItem` of an item: the fluid it holds (this port's emptying table).
    let holds = |item: &ItemStack| fluids::emptied(&item.item).is_some_and(|held| held.fluid == fluid);
    match read(filter) {
        Filter::Item(item) => holds(&item),
        Filter::Attribute(_) => false,
        Filter::List(list) => {
            let hit = list.items.iter().any(|entry| matches!(entry, Filter::Item(item) if holds(item)));
            hit != list.blacklist
        }
    }
}

/// The questions of [`rules::Facts`] answered for one stack.
struct ItemFacts<'a> {
    host: Option<&'a dyn Host>,
    stack: &'a ItemStack,
}

impl ItemFacts<'_> {
    fn recipe(&self, kind: &str) -> bool {
        self.host.is_some_and(|host| recipes::for_stack(host, kind, self.stack).is_some())
    }

    fn enchantments_for_crafting(&self) -> Vec<String> {
        // `EnchantmentHelper.getEnchantmentsForCrafting`: a book's stored ones, else the item's own.
        let stored = self.stack.stored_enchantments();
        let list = if stored.is_empty() { self.stack.enchantments() } else { stored };
        list.iter().map(|(id, _)| id.to_string()).collect()
    }
}

impl Facts for ItemFacts<'_> {
    fn singleton(&self, kind: Singleton) -> bool {
        let item: &str = &self.stack.item;
        match kind {
            Singleton::Placeable => self.host.is_some_and(|host| host.default_state(item).is_some()),
            Singleton::Consumable => tables::is_food(item),
            Singleton::FluidContainer => tables::bucket_fluid(item).is_some(),
            Singleton::Enchanted => !self.stack.enchantments().is_empty() || !self.stack.stored_enchantments().is_empty(),
            Singleton::MaxEnchanted => self.stack.enchantments().iter().any(|(id, level)| {
                tables::enchant_max(id).is_some_and(|max| max <= *level)
            }),
            Singleton::Renamed => self.stack.data().is_some_and(|data| data.custom_name.is_some()),
            Singleton::Damaged => self.stack.damage > 0 && tables::max_damage(item).is_some(),
            Singleton::BadlyDamaged => tables::max_damage(item).is_some_and(|max| {
                self.stack.damage > 0 && self.stack.damage as f32 / max as f32 > 3.0 / 4.0
            }),
            Singleton::NotStackable => {
                // `!ItemStack.isStackable`: stacks to more than 1 and isn't a damaged tool.
                let damaged = tables::max_damage(item).is_some() && self.stack.damage > 0;
                !(self.host.and_then(|host| host.max_stack_size(item)).unwrap_or(64) > 1 && !damaged)
            }
            Singleton::Equipable => tables::is_equipable(item),
            Singleton::FurnaceFuel => tables::is_fuel(item),
            Singleton::Washable => self.recipe("create:splashing"),
            Singleton::Hauntable => self.recipe("create:haunting"),
            Singleton::Crushable => self.recipe("create:crushing") || self.recipe("create:milling"),
            Singleton::Smeltable => self.recipe("create:baked_smelting"),
            Singleton::Smokable => self.recipe("create:baked_smoking"),
            Singleton::Blastable => self.recipe("create:baked_blasting"),
            Singleton::Compostable => arm_items::compostable(item),
        }
    }

    fn in_tag(&self, tag: &str) -> bool {
        self.host.is_some_and(|host| host.server_item_in_tag(self.stack, tag).unwrap_or(false))
    }

    fn tags(&self) -> Vec<String> {
        tables::ITEM_TAGS
            .iter()
            .filter(|tag| self.in_tag(tag))
            .map(|tag| (*tag).to_owned())
            .collect()
    }

    fn namespace(&self) -> String {
        self.stack.item.split(':').next().unwrap_or_default().to_owned()
    }

    fn enchantments(&self) -> Vec<String> {
        self.enchantments_for_crafting()
    }

    fn shulker_fill(&self) -> Option<Fill> {
        if !self.stack.item.ends_with("shulker_box") {
            return None;
        }
        let contents = self.stack.data().map(|data| data.container.as_slice()).unwrap_or_default();
        let full = contents.len() >= 27
            && contents.iter().all(|(_, held)| {
                self.host.and_then(|host| host.max_stack_size(&held.item)).is_some_and(|max| held.count == max)
            });
        Some(rules::shulker_fill(contents.len(), full))
    }

    fn colours(&self) -> Vec<String> {
        // `DyeColor.getColor` for a dye, else the colours the item's id starts with.
        const COLOURS: [&str; 16] = [
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
            "cyan", "purple", "blue", "brown", "green", "red", "black",
        ];
        let path = self.stack.item.split(':').nth(1).unwrap_or_default();
        COLOURS
            .iter()
            .filter(|colour| path.strip_prefix(**colour).is_some_and(|rest| rest.starts_with('_')))
            .map(|colour| (*colour).to_owned())
            .collect()
    }

    fn custom_name(&self) -> Option<String> {
        self.stack.data().and_then(|data| data.custom_name_plain())
    }

    fn book_author(&self) -> Option<String> {
        self.stack.data().and_then(|data| data.written.as_ref().map(|book| book.author.clone()))
    }

    fn book_generation(&self) -> Option<i32> {
        self.stack.data().and_then(|data| data.written.as_ref().map(|book| i32::from(book.generation)))
    }

    fn fluids(&self) -> Vec<String> {
        tables::bucket_fluid(&self.stack.item)
            .filter(|fluid| !fluid.is_empty())
            .map(|fluid| vec![fluid.to_owned()])
            .unwrap_or_default()
    }
}

// --- editing -------------------------------------------------------------------------------------

/// Edits waiting for the next tick: the player, the hotbar slot and the filter it becomes.
static EDITS: LazyLock<Mutex<Vec<(u128, usize, ItemStack)>>> = LazyLock::new(Default::default);

/// The reference item's name for a line of text.
fn name_of(stack: &ItemStack) -> String {
    stack
        .data()
        .and_then(|data| data.custom_name_plain())
        .unwrap_or_else(|| rules::display_name(&stack.item))
}

/// What a click with the filter `filter` does, given the item in the other hand: the edited
/// filter (if it changed) and the line to show.
fn edit(
    host: &dyn Host,
    filter: &ItemStack,
    reference: Option<&ItemStack>,
    sneaking: bool,
) -> (Option<ItemStack>, String) {
    let mut edited = filter.clone();
    if &*filter.item == LIST {
        let mut items = list_items(filter);
        let (blacklist, respect) = (flag(filter, rules::FILTER_ITEMS_BLACKLIST), flag(filter, rules::FILTER_ITEMS_RESPECT_NBT));
        let (mut blacklist_now, mut respect_now) = (blacklist, respect);
        let line = match reference {
            Some(reference) => {
                let mut one = reference.clone();
                one.count = 1;
                // `ghost` entries of a list are single items; the same item with other components is another entry.
                match rules::toggle_item(&mut items, one, |a, b| util::same_item(a, b)) {
                    Toggle::Added => format!("Added {}", name_of(reference)),
                    Toggle::Removed => format!("Removed {}", name_of(reference)),
                    Toggle::Full => format!("The filter is full ({} items)", rules::LIST_SLOTS),
                }
            }
            None if sneaking => {
                items.clear();
                "Filter cleared".to_owned()
            }
            None => {
                (blacklist_now, respect_now) = rules::next_list_mode(blacklist, respect);
                format!(
                    "{}, {}",
                    if blacklist_now { "Deny-List" } else { "Allow-List" },
                    if respect_now { "Respect Data" } else { "Ignore Data" }
                )
            }
        };
        write_list(host, &mut edited, &items, blacklist_now, respect_now);
        return (Some(edited), line);
    }
    // The Attribute Filter.
    let mut tests = attribute_list(filter);
    let mut mode = read_attributes(filter).mode;
    let line = match reference {
        Some(reference) => {
            let facts = ItemFacts { host: Some(host), stack: reference };
            let candidates = rules::all_attributes(&facts);
            match rules::next_attribute(&candidates, &tests, sneaking) {
                Some(attribute) => {
                    let line = format!("Added: {}", attribute.describe(sneaking));
                    tests.push((attribute, sneaking));
                    line
                }
                None if candidates.is_empty() => format!("{} has no attributes", name_of(reference)),
                None => "Every attribute of that item is in the filter already".to_owned(),
            }
        }
        None if sneaking => {
            tests.clear();
            "Filter cleared".to_owned()
        }
        None => {
            mode = mode.next();
            mode.label().to_owned()
        }
    };
    write_attributes(host, &mut edited, mode, &tests);
    (Some(edited), line)
}

/// A click with a filter in the main hand: queues the edit and shows the line.
fn click(host: &mut dyn Host, stack: &ItemStack) -> Flow {
    if !is_filter_item(&stack.item) {
        return Flow::Continue;
    }
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == util::SPECTATOR {
        return Flow::Continue;
    }
    // Only the filter in the main hand is edited.
    if host.held_stack(player.uuid).as_ref() != Some(stack) {
        return Flow::Continue;
    }
    let reference = host.player_equipment(player.uuid, EquipmentSlot::OffHand);
    let (edited, line) = edit(host, stack, reference.as_ref(), player.sneaking);
    if let Some(edited) = edited
        && edited != *stack
    {
        if let Ok(mut queue) = EDITS.lock() {
            queue.retain(|(uuid, _, _)| *uuid != player.uuid);
            queue.push((player.uuid, usize::from(player.selected_slot), edited));
        }
    }
    host.action_bar(player.uuid, Text::literal(line));
    Flow::Cancel
}

impl Hooks for FilterItems {
    /// `AllDataComponents.FILTER_ITEMS` and the rest.
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        for id in rules::COMPONENTS {
            if !event.types.iter().any(|t| t == id) {
                event.types.push(id.to_owned());
            }
        }
    }

    /// `FilterItem.appendHoverText`: the filter's lines under its name.
    fn tooltip(&self, _host: &dyn Host, event: &mut TooltipEvent<'_>) {
        let stack = event.stack;
        let lines = match &*stack.item {
            LIST => match read(stack) {
                Filter::List(list) => {
                    let names: Vec<String> = list_items_names(stack);
                    rules::list_summary(list.blacklist, &names)
                }
                _ => Vec::new(),
            },
            ATTRIBUTE => match read(stack) {
                Filter::Attribute(filter) => rules::attribute_summary(&filter),
                _ => Vec::new(),
            },
            _ => return,
        };
        if lines.is_empty() {
            return;
        }
        event.lines.push((Text::literal(String::new()), u32::MAX));
        for (index, line) in lines.into_iter().enumerate() {
            // `ChatFormatting.GOLD` for the heading, `GRAY` for the rest.
            let colour = if index == 0 { 0xFFFF_AA00 } else { 0xFFAA_AAAA };
            event.lines.push((Text::literal(line), colour));
        }
    }

    /// `FilterItem.use` on a block (`useOn`): a click on a block that takes no filter.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if !is_filter_item(&event.stack.item) {
            return Flow::Continue;
        }
        let pos = event.pos;
        if TAKES_A_FILTER.contains(&host.block(pos).as_str()) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        click(host, event.stack)
    }

    /// `FilterItem.use`: a click in the air.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if !is_filter_item(&event.stack.item) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if util::click_repeated_anywhere(host, player.uuid) {
            return Flow::Cancel;
        }
        let at = [player.position[0] as i32, player.position[1] as i32, player.position[2] as i32];
        util::click_taken(host, player.uuid, at);
        click(host, event.stack)
    }

    /// Applies the edits made this tick, where the inventory can be changed.
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let waiting: Vec<(u128, usize, ItemStack)> = match EDITS.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return,
        };
        for (uuid, slot, edited) in waiting {
            let Some(held) = host.held_stack(uuid).filter(|held| is_filter_item(&held.item)) else {
                continue;
            };
            let _ = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&held), Some(edited));
        }
    }
}

/// The names of a List Filter's items for its tooltip.
fn list_items_names(stack: &ItemStack) -> Vec<String> {
    list_items(stack).iter().map(name_of).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_filters_are_filter_items() {
        assert!(is_filter_item("create:filter"));
        assert!(is_filter_item("create:attribute_filter"));
        assert!(!is_filter_item("minecraft:stick"));
    }

    #[test]
    fn a_fluid_matches_an_item_that_holds_it() {
        let bucket = ItemStack {
            item: "minecraft:water_bucket".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        // An ordinary item as a filter: the fluid it holds.
        assert!(matches_fluid(&bucket, "minecraft:water"));
        assert!(!matches_fluid(&bucket, "minecraft:lava"));
    }
}

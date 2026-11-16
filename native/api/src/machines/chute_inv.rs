//! What the chutes and the funnels share: taking a stack out of an inventory by Create's rules
//! (`ItemHelper.extract`, `InvManipulationBehaviour`), putting one in
//! (`ItemHandlerHelper.insertItemStacked`), and the item filter that stands in for Create's
//! `FilteringBehaviour` slot. The rules are `create_kinetics::rules::chute`.
//!
//! Known differences from Create, for the README:
//! - Horizon's `Host` can't list another block's slots, so an inventory is read from its saved
//!   data (`block_entity_tag`, the `Items` list: chests, barrels, hoppers, furnaces, shulker boxes
//!   and Create's own containers once they hold something) and taken from slot by slot
//!   (`extract_item`). A block that keeps its items elsewhere (a jukebox, a decorated pot) is not
//!   an inventory here, and a mod's container is one only if it is named in `CONTAINERS` (an
//!   empty one saves no `Items`). A double chest's halves are joined as Horizon does (right half
//!   first).
//! - Nothing can ask a container whether it would take a stack without putting it in, so a
//!   transfer into a container is made for real at the moment it is tried: the "simulate" steps of
//!   Create's chutes and funnels happen for belts, depots and chutes only.
//! - The filter is an item set by clicking, not Create's filter slot and value box: see
//!   [`click`].

use create_kinetics::rules::chute::{self as rules, Amount, Mode, Plan, Slot};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Compound, Hooks, Host, ItemStack, Tag, Text};

use crate::{int, util};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `FilteringBehaviour.write`: the filter stack, `FilterAmount` and `UpTo`.
pub const FILTER: &str = "Filter";
pub const FILTER_AMOUNT: &str = "FilterAmount";
pub const UP_TO: &str = "UpTo";

/// The block entity's filter.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub item: Option<ItemStack>,
    pub amount: Amount,
}

impl Filter {
    /// `FilteringBehaviour.test`.
    pub fn test(&self, stack: &ItemStack) -> bool {
        rules::filter_passes(self.item.as_ref().map(|s| &*s.item), &stack.item)
    }

    pub fn max_stack(&self, host: &dyn Host) -> i32 {
        self.item
            .as_ref()
            .and_then(|s| host.max_stack_size(&s.item))
            .map_or(64, |m| m as i32)
    }
}

/// `FilteringBehaviour.read`.
pub fn filter_of(nbt: &Compound) -> Filter {
    let item = util::get(nbt, FILTER).and_then(util::stack_of);
    let amount = if util::has(nbt, FILTER_AMOUNT) {
        Amount {
            count: int(nbt, FILTER_AMOUNT),
            up_to: util::boolean(nbt, UP_TO),
        }
    } else {
        Amount::default()
    };
    Filter { item, amount }
}

/// `FilteringBehaviour.write`, only where it differs.
pub fn write_filter(nbt: &mut Compound, filter: &Filter) {
    let tag = match &filter.item {
        Some(stack) => util::stack_tag(stack),
        None => Tag::Compound(Compound::new()),
    };
    nbt.insert(FILTER.into(), tag);
    nbt.insert(FILTER_AMOUNT.into(), Tag::Int(filter.amount.count));
    nbt.insert(UP_TO.into(), Tag::Byte(i8::from(filter.amount.up_to)));
}

/// What a player's click does to the filter of the block entity at `pos`, standing in for
/// putting an item in Create's filter slot and setting its value box:
/// - with an item: it becomes the filter (one of it); clicking again with the item that is
///   already the filter steps the amount through [`rules::AMOUNT_STEPS`];
/// - with an empty hand, sneaking: the filter is cleared.
///
/// Answers the line to show the player, or `None` for a block entity that isn't there.
pub fn click(host: &mut dyn Host, pos: Pos, hand: Option<&ItemStack>) -> Option<String> {
    let entity = host.mod_block_entity(pos)?;
    let before = filter_of(&entity.nbt);
    let after = match hand {
        None => Filter { item: None, amount: Amount::default() },
        Some(stack) => {
            let mut one = stack.clone();
            one.count = 1;
            if before.item.as_ref().is_some_and(|have| util::same_item(have, &one)) {
                let max = host.max_stack_size(&one.item).unwrap_or(64) as i32;
                Filter { item: before.item.clone(), amount: rules::next_amount(before.amount, max) }
            } else {
                // `FilteringBehaviour.setFilter`: a new item keeps the amount if it fits.
                let max = host.max_stack_size(&one.item).unwrap_or(64) as i32;
                let mut amount = before.amount;
                if !amount.up_to {
                    amount.count = amount.count.min(max);
                }
                Filter { item: Some(one), amount }
            }
        }
    };
    if after != before {
        host.change_mod_block_entity(pos, &mut |entity| write_filter(&mut entity.nbt, &after));
        host.block_entity_sync(pos);
    }
    Some(match &after.item {
        None => "Filter cleared".to_owned(),
        Some(stack) => {
            let how = if after.amount.up_to && after.amount.count >= 64 {
                "any amount".to_owned()
            } else if after.amount.up_to {
                format!("up to {}", after.amount.count)
            } else {
                format!("exactly {}", after.amount.count)
            };
            format!("Filter: {} ({how})", stack.item)
        }
    })
}

/// Tells the acting player the line, on the action bar.
pub fn tell(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line.to_owned()));
    }
}

// --- reading an inventory ---------------------------------------------------------------------

fn counter_clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::West,
        Dir::West => Dir::South,
        Dir::South => Dir::East,
        Dir::East => Dir::North,
        other => other,
    }
}

fn clockwise(facing: Dir) -> Dir {
    counter_clockwise(counter_clockwise(counter_clockwise(facing)))
}

fn is_chest(name: &str) -> bool {
    name.contains("chest") && !name.contains("ender")
}

/// The blocks whose items make up the container at `pos`, with the slot each one's list starts
/// at: a double chest is its right half then its left (`HopperBlockEntity.getBlockContainer`).
pub(crate) fn parts(host: &dyn Host, pos: Pos) -> Vec<(Pos, usize)> {
    let name = host.block(pos);
    if name == VAULT
        && let Some(members) = vault_parts(host, pos)
    {
        return members;
    }
    if is_chest(&name) {
        let kind = host.block_property(pos, "type");
        let facing = host
            .block_property(pos, "facing")
            .as_deref()
            .and_then(Dir::from_name);
        if let (Some(kind), Some(facing)) = (kind.as_deref(), facing) {
            let towards = match kind {
                "left" => Some(clockwise(facing)),
                "right" => Some(counter_clockwise(facing)),
                _ => None,
            };
            if let Some(towards) = towards {
                let other = util::add(pos, towards.normal());
                let paired = host.block(other) == name
                    && host.block_property(other, "facing").as_deref() == Some(facing.name())
                    && host.block_property(other, "type").as_deref()
                        == Some(if kind == "left" { "right" } else { "left" });
                if paired {
                    return if kind == "right" {
                        vec![(pos, 0), (other, 27)]
                    } else {
                        vec![(other, 0), (pos, 27)]
                    };
                }
            }
        }
    }
    vec![(pos, 0)]
}

/// The items of the container at `pos`: their slots and stacks, from the saved data (`Items`:
/// `Slot`, `id`, `count`, `components`). `None` where there is no such list.
pub(crate) fn items_of(host: &mut dyn Host, pos: Pos) -> Option<Vec<(usize, ItemStack)>> {
    let dimension = host.dimension();
    let mut out = Vec::new();
    let mut found = false;
    for (part, offset) in parts(host, pos) {
        let Some(Tag::Compound(data)) = host.block_entity_tag(dimension, part) else {
            continue;
        };
        let Some(Tag::List(list)) = util::get(&data, "Items") else {
            continue;
        };
        found = true;
        for entry in list {
            let slot = match entry.get("Slot") {
                Some(Tag::Byte(slot)) => usize::from(*slot as u8),
                Some(Tag::Int(slot)) => *slot as usize,
                _ => continue,
            };
            if let Some(stack) = util::stack_of(entry) {
                out.push((offset + slot, stack));
            }
        }
    }
    found.then_some(out)
}

/// Create's own blocks that hold items for hoppers (their `block_entity_container`). Horizon
/// can't say whether a mod block has a container, and an empty one saves no `Items` list, so
/// they are named here.
const CONTAINERS: [&str; 6] = [
    "create:basin",
    "create:millstone",
    "create:mechanical_saw",
    "create:crushing_wheel_controller",
    VAULT,
    "create:creative_crate",
];

/// The Item Vault (`item_vault.rs`), one inventory across all its blocks (`initCapability`).
pub(crate) const VAULT: &str = "create:item_vault";
/// `CLogistics.vaultCapacity`: the slots one vault block holds.
pub(crate) const VAULT_SLOTS: usize = 20;

/// The blocks of the Item Vault that `pos` is part of, with the combined slot each one's list
/// starts at, in the order of `ItemVaultBlockEntity.initCapability` (the offsets are the ones
/// of `ConnectivityHandler.tryToFormNewMultiOfWidth`). Read from the saved `Controller`, `Size`
/// and `Length`, so this file names no vault code.
pub(crate) fn vault_parts(host: &dyn Host, pos: Pos) -> Option<Vec<(Pos, usize)>> {
    let own = host.mod_block_entity(pos)?;
    let controller = match util::get(&own.nbt, "Controller") {
        Some(Tag::IntArray(at)) if at.len() == 3 => [at[0], at[1], at[2]],
        _ => pos,
    };
    let lead = if controller == pos { own } else { host.mod_block_entity(controller)? };
    let size = util::int(&lead.nbt, "Size").max(1);
    let length = util::int(&lead.nbt, "Length").max(1);
    let along_z = host.block_property(controller, "axis").as_deref() == Some("z");
    let mut out = Vec::new();
    for y in 0..length {
        for x in 0..size {
            for z in 0..size {
                let at = if along_z {
                    [controller[0] + x, controller[1] + z, controller[2] + y]
                } else {
                    [controller[0] + y, controller[1] + x, controller[2] + z]
                };
                out.push((at, out.len() * VAULT_SLOTS));
            }
        }
    }
    Some(out)
}

/// Whether the block at `pos` is an inventory the funnels and chutes can put items in or take
/// them from: a container with an `Items` list, one of Create's own, a belt or a depot. (Chutes
/// are not: they are asked for separately.)
pub fn is_inventory(host: &mut dyn Host, pos: Pos) -> bool {
    match host.block(pos).as_str() {
        crate::belt::ID | crate::depot::ID => true,
        block if CONTAINERS.contains(&block) => true,
        _ => items_of(host, pos).is_some(),
    }
}

/// The slots of the container at `pos` as `ItemHelper.extract` sees them: each stack's item
/// (count 1) as its key.
fn slots(host: &mut dyn Host, pos: Pos) -> Vec<Slot<ItemStack>> {
    let Some(mut items) = items_of(host, pos) else {
        return Vec::new();
    };
    items.sort_by_key(|(slot, _)| *slot);
    items
        .into_iter()
        .map(|(slot, stack)| {
            let max = host.max_stack_size(&stack.item).unwrap_or(64);
            let count = stack.count;
            let mut key = stack;
            key.count = 1;
            Slot { slot, key, count, max }
        })
        .collect()
}

// --- taking ---------------------------------------------------------------------------------

/// `ItemHelper.extract` as a plan, without taking anything: what would come out of the
/// container at `pos`. `test` is asked of each candidate (the stack that would be taken from one
/// slot) and may use the world.
pub fn plan(
    host: &mut dyn Host,
    pos: Pos,
    mode: Mode,
    amount: u32,
    test: &mut dyn FnMut(&mut dyn Host, &ItemStack) -> bool,
) -> Option<Plan<ItemStack>> {
    let slots = slots(host, pos);
    if slots.is_empty() {
        return None;
    }
    rules::extract_plan(
        &slots,
        &mut |key, count| {
            let mut stack = key.clone();
            stack.count = count;
            test(host, &stack)
        },
        mode,
        amount,
    )
}

/// Takes what `plan` said from the container at `pos`, reaching it through its side `face` (the
/// side that touches whoever takes). A slot that gives something else than was planned (another
/// numbering than Horizon's) has its stack put back. The stack taken.
pub fn take(host: &mut dyn Host, pos: Pos, face: Dir, plan: &Plan<ItemStack>) -> Option<ItemStack> {
    let dimension = host.dimension();
    let mut taken: Option<ItemStack> = None;
    for (slot, count) in &plan.takes {
        // A vault's slots are numbered across all its blocks: ask the block that holds the slot.
        let (at, local) = if host.block(pos) == VAULT {
            parts(host, pos)
                .into_iter()
                .rfind(|(_, first)| first <= slot)
                .map_or((pos, *slot), |(member, first)| (member, slot - first))
        } else {
            (pos, *slot)
        };
        let Some(got) = host.extract_item(dimension, at, util::direction(face), local, *count)
        else {
            break;
        };
        if !util::same_item(&got, &plan.key) {
            put_back(host, pos, face, got);
            break;
        }
        match &mut taken {
            Some(have) => have.count += got.count,
            None => taken = Some(got),
        }
    }
    taken
}

/// `ItemHelper.extract` in one go.
pub fn extract(
    host: &mut dyn Host,
    pos: Pos,
    face: Dir,
    mode: Mode,
    amount: u32,
    test: &mut dyn FnMut(&mut dyn Host, &ItemStack) -> bool,
) -> Option<ItemStack> {
    let plan = plan(host, pos, mode, amount, test)?;
    take(host, pos, face, &plan)
}

/// Gives back a stack taken by mistake; what the container won't take drops at its block.
fn put_back(host: &mut dyn Host, pos: Pos, face: Dir, stack: ItemStack) {
    let rest = host.insert_item(pos, util::direction(face), stack);
    if let Some(rest) = rest {
        util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
    }
}

// --- putting in -----------------------------------------------------------------------------

/// `ItemHandlerHelper.insertItemStacked` into the inventory at `pos`, reached through its side
/// `face`: what is left of `stack` (count 0 when all went in). `None` where there is no
/// inventory. A belt or a depot takes it as a hopper's drop (`ItemHandlerBeltSegment`,
/// `DepotItemHandler`); a container takes it for real, `simulate` or not, as the host can't ask
/// first (a `simulate` into a container answers that nothing fits).
pub fn insert(
    host: &mut dyn Host,
    pos: Pos,
    face: Dir,
    stack: &ItemStack,
    simulate: bool,
) -> Option<ItemStack> {
    if let Some(rest) = crate::belt::handler_insert(host, pos, stack, simulate) {
        return Some(rest);
    }
    if !is_inventory(host, pos) {
        return None;
    }
    if simulate {
        return Some(stack.clone());
    }
    if host.block(pos) == VAULT {
        // Into the first block of the vault with room, as the combined handler fills its slots.
        let mut rest = stack.clone();
        for (member, _) in parts(host, pos) {
            match host.insert_item(member, util::direction(face), rest.clone()) {
                Some(left) if left.count > 0 => rest = left,
                _ => {
                    rest.count = 0;
                    break;
                }
            }
        }
        return Some(rest);
    }
    Some(host.insert_item(pos, util::direction(face), stack.clone()).unwrap_or_else(|| {
        let mut none = stack.clone();
        none.count = 0;
        none
    }))
}

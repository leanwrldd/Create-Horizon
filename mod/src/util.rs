//! What every machine module needs and none should write again: reading and writing a block
//! entity's NBT, item stacks as NBT, dropping items in the world, and the Wrench.
//!
//! NBT helpers are generic over the key type because `Compound`'s key (`JavaString`) isn't
//! re-exported by `mod_api`.

// Shared with the machine modules, which don't all use every helper.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};

use create_kinetics::{Dir, Pos};
use crate::compat::item_data::{decode_components, encode_components};
use crate::compat::{Compound, Direction, Flow, Host, ItemStack, Tag};

pub(crate) use crate::{float, int, set_float};

pub const WRENCH: &str = "create:wrench";
pub const SURVIVAL: i32 = 0;
pub const CREATIVE: i32 = 1;
pub const SPECTATOR: i32 = 3;
/// `Block.UPDATE_ALL`, the flags for `Host::set_block`.
pub const UPDATE_ALL: i32 = 3;

pub fn get<'m, K: Ord + for<'a> From<&'a str>>(
    nbt: &'m BTreeMap<K, Tag>,
    key: &str,
) -> Option<&'m Tag> {
    nbt.get(&K::from(key))
}

pub fn has<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> bool {
    nbt.contains_key(&K::from(key))
}

pub fn remove<K: Ord + for<'a> From<&'a str>>(
    nbt: &mut BTreeMap<K, Tag>,
    key: &str,
) -> Option<Tag> {
    nbt.remove(&K::from(key))
}

pub fn long<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> i64 {
    match nbt.get(&K::from(key)) {
        Some(Tag::Long(value)) => *value,
        _ => 0,
    }
}

pub fn boolean<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> bool {
    int(nbt, key) != 0
}

/// Writes `value` under `key` only when it differs. Reaching a block entity's fields mutably
/// counts as a change (its `DerefMut` bumps the revision), so pass `&mut entity.nbt` only after
/// [`differs`] said the write is needed, or use this on a compound of your own.
pub fn put(nbt: &mut Compound, key: &str, value: Tag) {
    nbt.insert(key.into(), value);
}

/// The bracket a shaft, cogwheel or pipe holds (`BracketedBlockEntityBehaviour`, saved as `Bracket`),
/// copied into an update tag: an empty compound when there is none, so a client that had one drops it.
pub fn copy_bracket(tag: &mut Compound, nbt: &Compound) {
    let bracket = match get(nbt, "Bracket") {
        Some(Tag::Compound(bracket)) => bracket.clone(),
        _ => Compound::new(),
    };
    tag.insert("Bracket".into(), Tag::Compound(bracket));
}

/// Whether `nbt` holds something other than `value` under `key`: check before any write to a
/// block entity.
pub fn differs<K: Ord + for<'a> From<&'a str>>(
    nbt: &BTreeMap<K, Tag>,
    key: &str,
    value: &Tag,
) -> bool {
    nbt.get(&K::from(key)) != Some(value)
}

/// `ItemStack.CODEC`: id, count, components.
pub fn stack_tag(stack: &ItemStack) -> Tag {
    let mut tag = Compound::new();
    tag.insert("id".into(), Tag::String((&*stack.item).into()));
    tag.insert("count".into(), Tag::Int(stack.count as i32));
    if let Some(components) = encode_components(stack) {
        tag.insert("components".into(), components);
    }
    Tag::Compound(tag)
}

pub fn stack_of(tag: &Tag) -> Option<ItemStack> {
    let Some(Tag::String(id)) = tag.get("id") else {
        return None;
    };
    let id = id.to_string_lossy();
    if id.is_empty() || id == "minecraft:air" {
        return None;
    }
    let count = match tag.get("count") {
        Some(Tag::Int(count)) if *count > 0 => *count as u32,
        _ => 1,
    };
    let (damage, extra) = decode_components(tag.get("components"));
    Some(ItemStack {
        item: id.as_str().into(),
        count,
        damage,
        extra,
    })
}

/// A list of stacks under one key, as Create's `ItemStackHandler` would save them.
pub fn stacks_tag(stacks: &[ItemStack]) -> Tag {
    Tag::List(stacks.iter().map(stack_tag).collect())
}

pub fn stacks_of<K: Ord + for<'a> From<&'a str>>(
    nbt: &BTreeMap<K, Tag>,
    key: &str,
) -> Vec<ItemStack> {
    match nbt.get(&K::from(key)) {
        Some(Tag::List(list)) => list.iter().filter_map(stack_of).collect(),
        _ => Vec::new(),
    }
}

/// Whether two stacks are the same item with the same components, whatever their counts.
pub fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    let mut probe = a.clone();
    probe.count = b.count;
    probe == *b
}

/// Adds `stack` to `stacks`, topping up equal stacks first.
pub fn merge(host: &dyn Host, stacks: &mut Vec<ItemStack>, mut stack: ItemStack) {
    let max = host.max_stack_size(&stack.item).unwrap_or(64);
    for have in stacks.iter_mut() {
        if stack.count == 0 {
            return;
        }
        if !same_item(&stack, have) {
            continue;
        }
        let moved = max.saturating_sub(have.count).min(stack.count);
        have.count += moved;
        stack.count -= moved;
    }
    if stack.count > 0 {
        stacks.push(stack);
    }
}

/// A fresh random number where no level random is at hand.
pub fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

pub fn centre(pos: Pos) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}

pub fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn dir(direction: Direction) -> Dir {
    crate::dir(direction)
}

pub fn direction(dir: Dir) -> Direction {
    match dir {
        Dir::Down => Direction::Down,
        Dir::Up => Direction::Up,
        Dir::North => Direction::North,
        Dir::South => Direction::South,
        Dir::West => Direction::West,
        Dir::East => Direction::East,
    }
}

/// The game time of the level a hook runs in; 0 outside one.
pub fn game_time(host: &dyn Host) -> i64 {
    host.level(host.dimension())
        .and_then(|level| level.game_time(host))
        .unwrap_or(0)
}

// --- one click, heard once for each hand -------------------------------------------------------------

/// The last click each player's block took, by player: the memory behind [`click_repeated`].
static TAKEN_CLICKS: std::sync::Mutex<BTreeMap<u128, create_kinetics::rules::click::Click>> =
    std::sync::Mutex::new(BTreeMap::new());

fn last_click(who: u128) -> Option<create_kinetics::rules::click::Click> {
    let taken = TAKEN_CLICKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    taken.get(&who).copied()
}

/// Whether this call of `right_click_block` is the rest of a click `who` already had taken at
/// `pos` (the off hand's turn of it): the handler then answers `Flow::Cancel` without acting. A
/// click is heard once for each hand the game tries and `RightClickBlockEvent` doesn't say which
/// (`right_click_block_v2` does, but isn't asked once a v1 handler cancels). Ask after the
/// handler has decided the click is its own, and call [`click_taken`] once it acts; or use
/// [`first_click`] when it acts at once. The decision is `rules::click`.
pub fn click_repeated(host: &dyn Host, who: u128, pos: Pos) -> bool {
    create_kinetics::rules::click::is_repeat(last_click(who), game_time(host), pos)
}

/// Whether anything now is the rest of a click `who` had taken, wherever: the item use the client
/// sends after a click a block took (a bucket's own use) has no position.
pub fn click_repeated_anywhere(host: &dyn Host, who: u128) -> bool {
    create_kinetics::rules::click::is_repeat_anywhere(last_click(who), game_time(host))
}

/// Records that the click of `who` on `pos` is taken, so [`click_repeated`] knows its repeat.
/// Only for a handler that goes on to return `Flow::Cancel`.
pub fn click_taken(host: &dyn Host, who: u128, pos: Pos) {
    let click = create_kinetics::rules::click::Click { tick: game_time(host), pos };
    TAKEN_CLICKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(who, click);
}

/// True for a new click, which it records as taken; false for the repeat of one (answer
/// `Flow::Cancel`). For a handler that acts whenever it gets this far.
pub fn first_click(host: &dyn Host, who: u128, pos: Pos) -> bool {
    if click_repeated(host, who, pos) {
        return false;
    }
    click_taken(host, who, pos);
    true
}

/// Drops `stack` as an item entity at `at` moving at `motion` (blocks a tick).
pub fn spawn_item(
    host: &mut dyn Host,
    at: [f64; 3],
    stack: &ItemStack,
    motion: [f64; 3],
    pickup_delay: i16,
) -> Option<u32> {
    let mut nbt = Compound::new();
    nbt.insert("Item".into(), stack_tag(stack));
    nbt.insert(
        "Motion".into(),
        Tag::List(motion.iter().map(|m| Tag::Double(m.to_bits())).collect()),
    );
    nbt.insert("PickupDelay".into(), Tag::Short(pickup_delay));
    let dimension = host.dimension();
    let id = host.summon(dimension, "minecraft:item", at, nbt)?;
    // Components the summon's item compound can't carry are set whole.
    host.set_item_entity(dimension, id, stack.clone());
    Some(id)
}

/// The Wrench on a block of yours, from `Hooks::use_item_on`: call it once you know the block at
/// `pos` is yours and the stack is [`WRENCH`]. Sneaking picks the block up
/// (`IWrenchable.onSneakWrenched`: the drops go to a survival player); otherwise `turn` runs
/// (`IWrenchable.onWrenched`), given the host to change the block's state. Always takes the click.
pub fn wrench(host: &mut dyn Host, pos: Pos, turn: impl FnOnce(&mut dyn Host)) -> Flow {
    if let Some(player) = host.acting_player()
        && player.sneaking
    {
        if matches!(player.game_mode, SURVIVAL | CREATIVE) {
            host.player_break_block(player.uuid, pos, true);
        }
        return Flow::Cancel;
    }
    turn(host);
    Flow::Cancel
}

/// A block's state property as a direction (`facing`).
pub fn facing(host: &dyn Host, pos: Pos, key: &str) -> Option<Dir> {
    host.block_property(pos, key)
        .as_deref()
        .and_then(Dir::from_name)
}

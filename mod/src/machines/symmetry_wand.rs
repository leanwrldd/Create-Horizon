//! The Wand of Symmetry (`SymmetryWandItem`, `SymmetryHandler`, `SymmetryMirror` and its types): with the wand in
//! any hotbar slot, blocks its player places or breaks near the mirror are copied across it. The positions, the
//! turned block states, the click rules and the costs are `create_kinetics::rules::symmetry_wand` (and
//! `symmetry_wand_state`); its header lists the known differences from Create (no screen, the mirror drawn as
//! outlines, no obstruction check).
//!
//! Using it (stand-in for Create's screen; the settings show on the action bar):
//! - right-click a block: put the mirror there (the type stays, a new wand makes a plane);
//! - right-click in the air: take the mirror away (the wand keeps it, switched off);
//! - sneak + right-click: next mirror type (plane, cross plane, triple plane); with a Wrench in the other hand,
//!   next orientation.
//!
//! The mirror is saved in the item as Create's components (`create:symmetry_wand` with `orientation_index`,
//! `position`, `type` and `enable`, and `create:symmetry_wand_enable`), changed on the next tick (a player's own
//! hook can't change the stack). A placement or a break is only heard in the player's own hook, where the hotbar's
//! other stacks can't be read: `inventory_tick` keeps each player's hotbar mirrors, the hooks queue the copies and
//! `level_tick_post` makes them (so the copies don't hear themselves) and takes the items of a survival player. The
//! player's client is sent the mirrors so it can draw them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::symmetry_wand::{self as rules, Kind, Mirror};
use create_kinetics::rules::worldshaper::UNBREAKABLE;
use crate::compat::{
    ComponentTypesEvent, Compound, DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, InventoryTickEvent,
    ItemStack, LevelTickPostEvent, NativeInventoryKind, NetFromServerEvent, ParticleOptions, PlaceBlockEvent,
    PlayerTickEvent, Tag, Text, UseItemEvent, UseItemOnEvent, WorldPrimitivesEvent,
};

use super::diodes_signal as signal;
use crate::util;

pub const ID: &str = "create:wand_of_symmetry";
/// The channel the server tells a player's client its hotbar mirrors on.
const CHANNEL: &str = "create:symmetry_mirrors";

struct SymmetryWand;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(SymmetryWand)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- The wand's data -------------------------------------------------------------------------------------------

fn double(tag: Option<&Tag>) -> f64 {
    match tag {
        Some(Tag::Double(bits)) => f64::from_bits(*bits),
        Some(Tag::Float(bits)) => f64::from(f32::from_bits(*bits)),
        Some(Tag::Int(n)) => f64::from(*n),
        _ => 0.0,
    }
}

/// `SymmetryMirror.CODEC` read back.
fn mirror_of(tag: &Compound) -> Mirror {
    let orientation = match util::get(tag, "orientation_index") {
        Some(Tag::Int(n)) => *n,
        _ => 0,
    };
    let position = match util::get(tag, "position") {
        Some(Tag::List(list)) if list.len() == 3 => [double(list.first()), double(list.get(1)), double(list.get(2))],
        _ => [0.0; 3],
    };
    let kind = match util::get(tag, "type") {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    };
    Mirror::from_fields(orientation, position, &kind, util::boolean(tag, "enable"))
}

/// `SymmetryMirror.CODEC` written.
fn mirror_tag(mirror: &Mirror) -> Compound {
    let mut tag = Compound::new();
    tag.insert("orientation_index".into(), Tag::Int(mirror.orientation));
    let position = mirror.position.map(|c| Tag::Double(f64::from(c).to_bits()));
    tag.insert("position".into(), Tag::List(position.to_vec()));
    tag.insert("type".into(), Tag::String(mirror.kind.name().into()));
    tag.insert("enable".into(), Tag::Byte(i8::from(mirror.enable)));
    tag
}

/// The wand's mirror and `SYMMETRY_WAND_ENABLE` (`checkComponents`: no mirror is the empty one, off).
fn read(stack: &ItemStack) -> (Mirror, bool) {
    let data = stack.data();
    let mirror = match data.and_then(|d| d.other(rules::COMPONENT)) {
        Some(Tag::Compound(tag)) => mirror_of(tag),
        _ => Mirror::EMPTY,
    };
    let enabled = matches!(data.and_then(|d| d.other(rules::ENABLE_COMPONENT)), Some(Tag::Byte(b)) if *b != 0);
    (mirror, enabled)
}

fn write(host: &dyn Host, stack: &mut ItemStack, mirror: &Mirror, enabled: bool) {
    let _ = host.set_component(stack, rules::COMPONENT, Tag::Compound(mirror_tag(mirror)));
    let _ = host.set_component(stack, rules::ENABLE_COMPONENT, Tag::Byte(i8::from(enabled)));
}

/// What the wand does while it is in the hotbar: `isEnabled` and a mirror to copy across.
fn active(stack: &ItemStack) -> Option<Mirror> {
    let (mirror, enabled) = read(stack);
    (enabled && mirror.kind != Kind::Empty).then_some(mirror)
}

// ---- Memory between hooks -----------------------------------------------------------------------------------

/// Each player's hotbar mirrors, with the game time they were read at (`inventory_tick`).
static HOTBAR: LazyLock<Mutex<HashMap<u128, (i64, Vec<Mirror>)>>> = LazyLock::new(Default::default);
/// What each player's client was last sent, and when.
static SENT: LazyLock<Mutex<HashMap<u128, (Vec<Mirror>, i64)>>> = LazyLock::new(Default::default);
/// The mirrors the client was sent, for drawing.
static CLIENT: LazyLock<Mutex<Vec<Mirror>>> = LazyLock::new(Default::default);
/// Edits of the held wand waiting for the next tick.
static EDITS: LazyLock<Mutex<Vec<(u128, usize, ItemStack)>>> = LazyLock::new(Default::default);
/// Copies waiting for the next tick.
static JOBS: LazyLock<Mutex<Vec<Job>>> = LazyLock::new(Default::default);
/// Set while the copies are made, so that they aren't copied in turn.
static BUSY: AtomicBool = AtomicBool::new(false);

/// How a mirror's copy is kept inside the hotbar's memory.
fn hotbar(now: i64, uuid: u128) -> Vec<Mirror> {
    match lock(&HOTBAR).get(&uuid) {
        Some((stamp, mirrors)) if *stamp >= now - 1 => mirrors.clone(),
        _ => Vec::new(),
    }
}

enum Job {
    /// `SymmetryWandItem.apply`.
    Place {
        uuid: u128,
        dimension: &'static str,
        centre: [i32; 3],
        at: [i32; 3],
        item: String,
        creative: bool,
        selected: usize,
        copies: Vec<([i32; 3], String)>,
    },
    /// `SymmetryWandItem.remove`.
    Remove {
        uuid: u128,
        dimension: &'static str,
        centre: [i32; 3],
        original: String,
        creative: bool,
        tool: Option<ItemStack>,
        positions: Vec<[i32; 3]>,
    },
}

fn queue_edit(uuid: u128, slot: usize, stack: ItemStack) {
    let mut edits = lock(&EDITS);
    edits.retain(|(who, _, _)| *who != uuid);
    edits.push((uuid, slot, stack));
}

fn tell(host: &mut dyn Host, line: String) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line));
    }
}

// ---- Clicks -------------------------------------------------------------------------------------------------------

/// `SymmetryWandItem.useOn` and `use`: `target` is the block clicked and its face, none in the air.
fn click(host: &mut dyn Host, stack: &ItemStack, target: Option<([i32; 3], [i32; 3])>) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).as_ref() != Some(stack) {
        return Flow::Continue;
    }
    let (mirror, enabled) = read(stack);
    let (mirror, enabled, line) = if player.sneaking {
        // The screen: a type selector and an orientation selector.
        let off = host.player_equipment(player.uuid, EquipmentSlot::OffHand);
        let next = if off.as_ref().is_some_and(|s| &*s.item == util::WRENCH) {
            rules::next_orientation(mirror)
        } else {
            rules::next_kind(mirror)
        };
        (next, enabled, next.describe())
    } else if let Some((pos, face)) = target {
        // `useOn`: make or move the mirror and switch the wand on.
        let facing = rules::direction_of_yaw(player.yaw);
        let made = rules::click(mirror, pos, face, facing, player.yaw);
        (made, true, made.describe())
    } else {
        // `use`: switch the mirror off.
        (mirror, false, "Mirror removed".to_owned())
    };
    let mut edited = stack.clone();
    write(&*host, &mut edited, &mirror, enabled);
    queue_edit(player.uuid, usize::from(player.selected_slot), edited);
    tell(host, line);
    Flow::Cancel
}

// ---- Making the copies --------------------------------------------------------------------------------------

/// The dust along the lines from the mirror to what was changed (`SymmetryEffectPacket`).
fn effects(host: &mut dyn Host, uuid: u128, centre: [i32; 3], targets: &[[i32; 3]]) {
    let dust = ParticleOptions::dust([1.0, 1.0, 1.0], 1.0);
    for target in targets {
        for at in rules::effect_points(centre, *target) {
            host.send_particles_to(uuid, &dust, None, at, [0.0; 3], 0.0, 1, false);
        }
    }
}

/// `BlockHelper.findAndRemoveInInventory`: takes `need` of `item` from the stacks, the held slot first; false when
/// they aren't enough (nothing is taken then).
fn pay(host: &mut dyn Host, uuid: u128, stacks: &mut [Option<ItemStack>], item: &str, selected: usize, need: u32) -> bool {
    let order: Vec<usize> = std::iter::once(selected).chain((0..stacks.len()).filter(|i| *i != selected)).collect();
    let found: Vec<(usize, u32)> = order
        .iter()
        .filter_map(|&i| stacks.get(i)?.as_ref().filter(|s| &*s.item == item && s.count > 0).map(|s| (i, s.count)))
        .collect();
    let Some(plan) = rules::take_plan(&found, need) else {
        return false;
    };
    for (slot, taken) in plan {
        let Some(old) = stacks[slot].clone() else {
            continue;
        };
        let mut left = old.clone();
        left.count -= taken;
        let new = (left.count > 0).then_some(left);
        if host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&old), new.clone()).is_ok() {
            stacks[slot] = new;
        }
    }
    true
}

fn place(
    host: &mut dyn Host,
    (uuid, dimension, centre, at): (u128, &'static str, [i32; 3], [i32; 3]),
    item: &str,
    creative: bool,
    selected: usize,
    copies: &[([i32; 3], String)],
) {
    let mut stacks = if creative { None } else { host.player_stacks(uuid) };
    let mut targets = vec![at];
    for (pos, text) in copies {
        if creative {
            // `world.setBlockAndUpdate(position, blockState)`.
        } else {
            // A survival copy needs a place to go and an item to pay with.
            let there = host.block(*pos);
            if !signal::replaceable(&there) || UNBREAKABLE.contains(&there.as_str()) {
                continue;
            }
            let Some(stacks) = stacks.as_mut() else {
                continue;
            };
            if !pay(host, uuid, stacks, item, selected, rules::item_cost(text)) {
                continue;
            }
        }
        let line = format!("execute in {dimension} run setblock {} {} {} {text} replace", pos[0], pos[1], pos[2]);
        if host.run_command(&line) {
            targets.push(*pos);
        }
    }
    effects(host, uuid, centre, &targets[1..]);
}

fn remove(
    host: &mut dyn Host,
    (uuid, centre): (u128, [i32; 3]),
    original: &str,
    creative: bool,
    tool: Option<&ItemStack>,
    positions: &[[i32; 3]],
) {
    let dimension = host.dimension();
    let mut targets = Vec::new();
    for pos in positions {
        let there = host.block(*pos);
        if there == "minecraft:air" || (!creative && there != original) {
            continue;
        }
        BUSY.store(true, Ordering::Relaxed);
        let broke = if creative {
            host.destroy_block(*pos, None, true).is_some()
        } else {
            host.destroy_block(*pos, tool, false).is_some()
        };
        BUSY.store(false, Ordering::Relaxed);
        if !broke {
            let Some(air) = host.default_state("minecraft:air") else {
                continue;
            };
            if !host.set_block(dimension, *pos, air, util::UPDATE_ALL) {
                continue;
            }
        }
        targets.push(*pos);
    }
    effects(host, uuid, centre, &targets);
}

impl Hooks for SymmetryWand {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        for id in [rules::COMPONENT, rules::ENABLE_COMPONENT] {
            if !event.types.iter().any(|t| t == id) {
                event.types.push(id.to_owned());
            }
        }
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        // Heard once for each hand: only the first is the click.
        if !util::first_click(host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        click(host, event.stack, Some((event.pos, event.face)))
    }

    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if util::click_repeated_anywhere(host, player.uuid) {
            return Flow::Cancel;
        }
        util::click_taken(host, player.uuid, [0, 0, 0]);
        click(host, event.stack, None)
    }

    /// `SymmetryHandler.onBlockPlaced`: a block the player placed is copied across each hotbar wand's mirror.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if BUSY.load(Ordering::Relaxed) {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let now = util::game_time(&*host);
        let mirrors = hotbar(now, player.uuid);
        if mirrors.is_empty() {
            return;
        }
        let dimension = host.dimension();
        let Some(text) = host.level(dimension).and_then(|level| level.block_state(&*host, event.pos)) else {
            return;
        };
        let mut jobs = lock(&JOBS);
        for mirror in mirrors {
            if !rules::in_range(&mirror, event.pos) {
                continue;
            }
            jobs.push(Job::Place {
                uuid: player.uuid,
                dimension,
                centre: mirror.position,
                at: event.pos,
                item: event.stack.item.to_string(),
                creative: player.game_mode == util::CREATIVE,
                selected: usize::from(player.selected_slot),
                copies: rules::copies_of_state(&mirror, event.pos, &text),
            });
        }
    }

    /// `SymmetryHandler.onBlockDestroyed`.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if BUSY.load(Ordering::Relaxed) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let now = util::game_time(&*host);
        let mirrors = hotbar(now, player.uuid);
        if mirrors.is_empty() {
            return Flow::Continue;
        }
        let original = host.block(event.pos);
        let tool = host.held_stack(player.uuid);
        let dimension = host.dimension();
        let mut jobs = lock(&JOBS);
        for mirror in mirrors {
            if !rules::in_range(&mirror, event.pos) {
                continue;
            }
            jobs.push(Job::Remove {
                uuid: player.uuid,
                dimension,
                centre: mirror.position,
                original: original.clone(),
                creative: player.game_mode == util::CREATIVE,
                tool: tool.clone(),
                positions: rules::copies(&mirror, event.pos).into_iter().map(|c| c.pos).collect(),
            });
        }
        Flow::Continue
    }

    /// Keeps the hotbar's mirrors: the other slots can't be read from a player's own hook.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !(0..9).contains(&event.slot) {
            return;
        }
        let Some(mirror) = host.carried_stack(event.slot).filter(|s| &*s.item == ID).and_then(|s| active(s)) else {
            return;
        };
        let Some(player) = host.acting_player() else {
            return;
        };
        let now = util::game_time(&*host);
        let mut hotbar = lock(&HOTBAR);
        let entry = hotbar.entry(player.uuid).or_insert((now, Vec::new()));
        if entry.0 != now {
            *entry = (now, Vec::new());
        }
        entry.1.push(mirror);
    }

    /// Tells the player's client its mirrors when they change (and now and then, in case a packet was lost).
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let now = util::game_time(&*host);
        let mirrors = hotbar(now, player.uuid);
        {
            let mut sent = lock(&SENT);
            let entry = sent.entry(player.uuid).or_insert((Vec::new(), now));
            let stale = !mirrors.is_empty() && now - entry.1 >= 100;
            if entry.0 == mirrors && !stale {
                return;
            }
            *entry = (mirrors.clone(), now);
        }
        let mut data = vec![mirrors.len().min(255) as u8];
        for mirror in mirrors.iter().take(255) {
            data.push(match mirror.kind {
                Kind::Empty => 0,
                Kind::Plane => 1,
                Kind::CrossPlane => 2,
                Kind::TriplePlane => 3,
            });
            data.push(mirror.orientation as u8);
            for c in mirror.position {
                data.extend_from_slice(&c.to_le_bytes());
            }
        }
        let _ = host.net_send_to_player(player.uuid, CHANNEL, &data);
    }

    /// The client half: the mirrors to draw.
    fn net_from_server(&self, _: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != CHANNEL || event.data.is_empty() {
            return;
        }
        let count = usize::from(event.data[0]);
        let mut mirrors = Vec::new();
        for chunk in event.data[1..].chunks_exact(14).take(count) {
            let kind = match chunk[0] {
                1 => Kind::Plane,
                2 => Kind::CrossPlane,
                3 => Kind::TriplePlane,
                _ => continue,
            };
            let coord = |i: usize| i32::from_le_bytes([chunk[2 + i * 4], chunk[3 + i * 4], chunk[4 + i * 4], chunk[5 + i * 4]]);
            mirrors.push(Mirror {
                kind,
                orientation: i32::from(chunk[1]),
                position: [coord(0), coord(1), coord(2)],
                enable: true,
            });
        }
        *lock(&CLIENT) = mirrors;
    }

    /// `SymmetryHandler.onRenderWorld`: the mirrors, as outlines.
    fn world_primitives(&self, _: &dyn Host, event: &mut WorldPrimitivesEvent<'_>) {
        let mirrors = lock(&CLIENT).clone();
        if mirrors.is_empty() {
            return;
        }
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let bob = rules::bob(seconds * 20.0);
        for mirror in mirrors {
            for (from, to) in rules::outline(&mirror, bob) {
                event.primitives.line(from, to, 0xFFB8_E6FF, 2.0);
            }
        }
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let edits = std::mem::take(&mut *lock(&EDITS));
        for (uuid, slot, edited) in edits {
            let Some(held) = host.held_stack(uuid).filter(|held| &*held.item == ID) else {
                continue;
            };
            let _ = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&held), Some(edited));
        }
        // A level's tick makes the copies in its own dimension; the others wait for theirs.
        let dimension = host.dimension();
        let (mine, other): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut *lock(&JOBS)).into_iter().partition(|job| match job {
                Job::Place { dimension: d, .. } | Job::Remove { dimension: d, .. } => *d == dimension,
            });
        lock(&JOBS).extend(other);
        for job in mine {
            match job {
                Job::Place { uuid, dimension, centre, at, item, creative, selected, copies } => {
                    place(host, (uuid, dimension, centre, at), &item, creative, selected, &copies);
                }
                Job::Remove { uuid, centre, original, creative, tool, positions, .. } => {
                    remove(host, (uuid, centre), &original, creative, tool.as_ref(), &positions);
                }
            }
        }
    }
}

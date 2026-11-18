//! Super Glue (`SuperGlueItem`, `SuperGlueEntity`, `SuperGlueSelectionHandler`,
//! `SuperGlueSelectionHelper`, `SuperGlueSelectionPacket`, `SuperGlueRemovalPacket`): right-click a
//! block with the glue to pick one corner and another to glue the box between them (when the
//! blocks in it hold together), sneak-click to forget the first, left-click a glued block to take
//! the glue you look at off. A structure that gathers a block in a glued box takes the blocks the
//! box holds to it. The rules (the boxes, the click sequence, what a structure carries) are
//! `create_kinetics::rules::super_glue`; this reads the world, asks them and writes what they say.
//!
//! # How the glue is stored
//!
//! Horizon has no entity type a mod can give a bounding box, so a glue is not an entity. Each
//! level keeps its glue as one **level attachment** (`create:glue`, `Holder::Level`, saved with the
//! level, a list of boxes as `IntArray` of six): it survives a save and a reload and is per
//! dimension. The server reads it when a hand picks or takes glue, and a structure reads it as it
//! gathers (`refresh`, then `glued`, from a copy held in memory for the one search); a structure
//! carries the glue it takes in its own data (`Moving.glue`, Create's `Superglue`) and puts it back
//! where it lands. Nothing is sent to clients: the glue is shown by the server, as the Mechanical
//! Arm shows its points, with dust sent to the player holding the glue.
//!
//! Known differences from Create, for the README:
//! - A glue is a box of whole blocks saved in a level attachment, not an entity. Nothing else
//!   (pistons, other mods) can push or kill it; a vanilla piston moving a glued block leaves the
//!   glue where it was (Create's entity is discarded when it is moved).
//! - The corners come from the block clicked, and the selection is shown with dust at the edges
//!   of each glue near the player (the one looked at brighter), not Create's outline with its
//!   texture. The box from the first corner to the block looked at isn't previewed: the first
//!   block is marked and the result is told when the second is clicked.
//! - A glue to take off must be looked at with a block under the crosshair (a left click on a
//!   block, as Horizon hands it to a mod).
//! - The glue in the off hand doesn't glue the blocks you place (`glueInOffHandAppliesOnBlockPlace`);
//!   no advancement; a glue's break and the glue effect packet are the dust.
//! - A stack of glue wears one use for each glue made, as `hurtAndBreak`, but only the glue in the
//!   main hand is charged (Create searches the whole inventory).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use create_kinetics::rules::chassis;
use create_kinetics::rules::contraption::{State, Transform};
use create_kinetics::rules::super_glue::{
    self as rules, Click, GlueBox, GlueSet, ITEM, closer_than, has_charge, search_group, use_charge,
};
use create_kinetics::{Dir, Pos};
use crate::compat::TextExt;
use crate::compat::{
    AttachmentCodec, AttachmentType, Compound, Flow, Hooks, Holder, Host, LeftClickBlockEvent,
    LevelTickPostEvent, NativeInventoryKind, ParticleOptions, PlayerTickEvent, RightClickBlockEvent,
    Tag, Text,
};

use super::contraption::HostWorld;
use crate::util;

struct Glue;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Glue)
}

/// The level's glue (`create:glue`), saved with the level.
static GLUE: AttachmentType<Compound> = AttachmentType {
    namespace: "create",
    name: "glue",
    default: Compound::new,
    codec: Some(AttachmentCodec {
        encode: |data| Ok(data.clone()),
        decode: |data| Ok(data.clone()),
    }),
    copy_on_death: false,
    sync: false,
};

/// The saved list of boxes in the attachment: `IntArray` of `min` and `max`.
const BOXES: &str = "Boxes";

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- the stored glue ---------------------------------------------------------------------------------

fn boxes_to_nbt(set: &GlueSet) -> Compound {
    let list = set
        .boxes
        .iter()
        .map(|b| Tag::IntArray(b.min.iter().chain(b.max.iter()).copied().collect()))
        .collect();
    let mut nbt = Compound::new();
    nbt.insert(BOXES.into(), Tag::List(list));
    nbt
}

fn boxes_from_nbt(nbt: &Compound) -> GlueSet {
    let Some(Tag::List(list)) = util::get(nbt, BOXES) else {
        return GlueSet::default();
    };
    GlueSet::new(
        list.iter()
            .filter_map(|tag| match tag {
                Tag::IntArray(v) if v.len() == 6 => Some(GlueBox {
                    min: [v[0], v[1], v[2]],
                    max: [v[3], v[4], v[5]],
                }),
                _ => None,
            })
            .collect(),
    )
}

/// The copy of each dimension's glue a structure's search reads (`refresh`).
static SNAPSHOT: LazyLock<Mutex<HashMap<String, Arc<GlueSet>>>> = LazyLock::new(Mutex::default);

/// The level's glue, read from its attachment.
pub fn load(host: &mut dyn Host) -> GlueSet {
    match GLUE.get_existing(host.new_server(), Holder::Level) {
        Ok(Some(nbt)) => boxes_from_nbt(&nbt),
        _ => GlueSet::default(),
    }
}

fn store(host: &mut dyn Host, set: &GlueSet) {
    let _ = GLUE.set(host.new_server(), Holder::Level, boxes_to_nbt(set));
    lock(&SNAPSHOT).insert(host.dimension().to_owned(), Arc::new(set.clone()));
}

/// Reads the level's glue for the search that follows (`Contraption.searchMovedStructure` asks
/// `SuperGlueEntity.isGlued`): call before a `HostWorld` is made for an assembly.
pub fn refresh(host: &mut dyn Host) {
    let set = load(host);
    lock(&SNAPSHOT).insert(host.dimension().to_owned(), Arc::new(set));
}

/// `SuperGlueEntity.isGlued` from the copy `refresh` made; nothing for a dimension never read.
pub(crate) fn glued(host: &dyn Host, pos: Pos, face: Dir) -> bool {
    lock(&SNAPSHOT)
        .get(host.dimension())
        .is_some_and(|set| set.glued(pos, face))
}

/// What a structure takes with it (`Contraption.removeBlocksFromWorld`): the glue that holds its
/// blocks `found` to a neighbour leaves the level, shrunk to the blocks of the structure
/// (`structure`, all of them) and given relative to `origin`. Returns those boxes.
pub fn capture(host: &mut dyn Host, found: &[Pos], structure: &[Pos], origin: Pos) -> Vec<GlueBox> {
    let mut set = load(host);
    let found: HashSet<Pos> = found.iter().copied().collect();
    let taken = rules::captured(&set, &found);
    if taken.is_empty() {
        return Vec::new();
    }
    let boxes: Vec<GlueBox> = taken.iter().map(|i| set.boxes[*i]).collect();
    for glue in &boxes {
        set.remove(glue);
    }
    store(host, &set);
    boxes
        .iter()
        .filter_map(|b| rules::minimised(b, structure))
        .map(|b| b.relative_to(origin))
        .collect()
}

/// Puts a structure's glue back where it lands (`Contraption.addBlocksToWorld`): `glue` is
/// relative to the anchor, `transform` takes the structure's positions to the world.
pub fn restore(host: &mut dyn Host, glue: &[GlueBox], transform: &Transform) {
    if glue.is_empty() {
        return;
    }
    let mut set = load(host);
    for b in glue {
        set.add(b.transformed(transform));
    }
    store(host, &set);
}

// --- who has picked what -----------------------------------------------------------------------------

/// The first corner each player has picked (`firstPos`), with the dimension.
static FIRST: LazyLock<Mutex<HashMap<u128, (String, Pos)>>> = LazyLock::new(Mutex::default);
/// Players owed a charge, with the slot of their glue: charged at the level's tick, where the
/// inventory can be edited (a player's own hook can't).
static CHARGES: LazyLock<Mutex<Vec<(u128, usize)>>> = LazyLock::new(Mutex::default);

/// `SuperGlueSelectionHandler`'s colours.
const PASSIVE: u32 = 0x4D_9162;
const HIGHLIGHT: u32 = 0x68_C586;
const FAIL: u32 = 0xC5_B548;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn say(host: &mut dyn Host, uuid: u128, line: &str, colour: u32) {
    host.action_bar(uuid, Text::colored(line.to_owned(), colour));
}

fn holds_glue(host: &dyn Host, uuid: u128) -> Option<u32> {
    host.held_stack(uuid)
        .filter(|stack| &*stack.item == ITEM)
        .map(|stack| stack.damage)
}

fn dust(host: &mut dyn Host, uuid: u128, colour: u32, at: [f64; 3]) {
    let [r, g, b] = [16, 8, 0].map(|shift| ((colour >> shift) & 0xFF) as f32 / 255.0);
    host.send_particles_to(uuid, &ParticleOptions::dust([r, g, b], 0.5), None, at, [0.0; 3], 0.0, 1, false);
}

/// The dust that outlines a box (`Outliner.showAABB`): along each of its twelve edges.
fn outline(host: &mut dyn Host, uuid: u128, glue: &GlueBox, colour: u32, step: f64) {
    for at in glue.outline_points(step) {
        dust(host, uuid, colour, at);
    }
}

fn eye_and_look(player: &crate::compat::NativeServerPlayer) -> ([f64; 3], [f64; 3]) {
    let height = if player.sneaking { 1.27 } else { 1.62 };
    let (yaw, pitch) = (f64::from(player.yaw).to_radians(), f64::from(player.pitch).to_radians());
    (
        [player.position[0], player.position[1] + height, player.position[2]],
        [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()],
    )
}

/// A chassis side that takes the slime: the click is the chassis' (`SuperGlueItem
/// .glueItemAlwaysPlacesWhenUsed`).
fn chassis_takes_it(host: &dyn Host, pos: Pos, face: Dir) -> bool {
    let Some(level) = host.level(host.dimension()) else { return false };
    let Some(text) = crate::compat::state_text(host, level, pos) else { return false };
    let state = State::parse(&text);
    chassis::is_chassis(&state.name)
        && chassis::glueable_side(state.is(chassis::RADIAL), chassis::axis_of(&state), face).is_some()
}

fn spawn_effect(host: &mut dyn Host, glue: &GlueBox, uuid: u128) {
    outline(host, uuid, glue, 0xB5_F2C6, 0.5);
    let at = [0, 1, 2].map(|i| f64::from(glue.min[i] + glue.max[i]) / 2.0 + 0.5);
    host.play_sound("minecraft:block.slime_block.place", at, BLOCKS, 0.5, 0.95);
    host.play_sound("minecraft:entity.item_frame.add_item", at, BLOCKS, 0.75, 1.0);
}

impl Hooks for Glue {
    /// Two clicks make a glue (`SuperGlueSelectionHandler.onMouseInput`).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(damage) = holds_glue(&*host, player.uuid) else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        let pos = event.pos;
        if let Some(face) = Dir::from_normal(event.face)
            && chassis_takes_it(&*host, pos, face)
        {
            return Flow::Continue;
        }
        // Heard once for each hand: the glue always places when used.
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let dimension = host.dimension().to_owned();
        let first = lock(&FIRST)
            .get(&player.uuid)
            .filter(|(dim, _)| *dim == dimension)
            .map(|(_, pos)| *pos);
        refresh(host);
        let set = load(host);
        let creative = player.game_mode == util::CREATIVE;
        let outcome = {
            let Some(world) = HostWorld::new(&*host) else {
                return Flow::Cancel;
            };
            let group = |a: Pos, b: Pos| search_group(&world, &set, a, b, false);
            rules::click(first, pos, player.sneaking, &group, creative || has_charge(damage))
        };
        match outcome {
            Click::Ignore => {}
            Click::FirstCorner(corner) => {
                lock(&FIRST).insert(player.uuid, (dimension, corner));
                say(host, player.uuid, "First position selected", PASSIVE);
                let at = util::centre(corner);
                host.play_sound("minecraft:block.slime_block.place", at, BLOCKS, 0.5, 0.85);
                host.play_sound("minecraft:entity.item_frame.add_item", at, BLOCKS, 0.75, 1.0);
            }
            Click::Discard => {
                lock(&FIRST).remove(&player.uuid);
                say(host, player.uuid, "Selection discarded", PASSIVE);
            }
            Click::TooFar => say(host, player.uuid, "Selected area is too big", FAIL),
            Click::CannotReach => say(host, player.uuid, "Selected blocks must be connected", FAIL),
            Click::NotEnough => say(host, player.uuid, "Not enough glue in inventory", FAIL),
            Click::Glue(glue) => {
                let mut set = set;
                set.add(glue);
                store(host, &set);
                lock(&FIRST).remove(&player.uuid);
                if !creative {
                    lock(&CHARGES).push((player.uuid, usize::from(player.selected_slot)));
                }
                spawn_effect(host, &glue, player.uuid);
                say(host, player.uuid, "Applying Glue...", HIGHLIGHT);
            }
        }
        Flow::Cancel
    }

    /// A left click with the glue takes off the glue looked at (`SuperGlueRemovalPacket`), and
    /// never breaks the block (`SuperGlueItem.canAttackBlock`).
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if holds_glue(&*host, player.uuid).is_none() || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        let (eye, look) = eye_and_look(&player);
        let range = rules::removal_range(player.game_mode == util::CREATIVE);
        let mut set = load(host);
        if let Some(glue) = set.picked(eye, look, range) {
            set.remove(&glue);
            store(host, &set);
            let at = [0, 1, 2].map(|i| f64::from(glue.min[i] + glue.max[i]) / 2.0 + 0.5);
            host.play_sound("minecraft:block.slime_block.place", at, BLOCKS, 0.5, 0.5);
            outline(host, player.uuid, &glue, PASSIVE, 0.5);
        }
        Flow::Cancel
    }

    /// Dust at the edges of the glue near a player holding it (`SuperGlueSelectionHandler.tick`:
    /// passive green, the one looked at brighter), and on their first corner.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let holding = holds_glue(&*host, player.uuid).is_some();
        if !holding {
            lock(&FIRST).remove(&player.uuid);
            return;
        }
        // Half a second apart; players are spread over the ticks by their uuid.
        let now = util::game_time(&*host);
        if (now + (player.uuid % 10) as i64) % 10 != 0 {
            return;
        }
        let set = load(host);
        let (eye, look) = eye_and_look(&player);
        let picked = set.picked(eye, look, rules::removal_range(player.game_mode == util::CREATIVE));
        let mut near = set.near(player.position, [32.0, 16.0, 32.0]);
        near.sort_by(|a, b| {
            let d = |g: &GlueBox| (0..3).map(|i| (f64::from(g.min[i] + g.max[i]) / 2.0 - player.position[i]).powi(2)).sum::<f64>();
            d(a).total_cmp(&d(b))
        });
        for glue in near.iter().take(24) {
            if Some(*glue) == picked {
                outline(host, player.uuid, glue, HIGHLIGHT, 0.5);
            } else {
                outline(host, player.uuid, glue, PASSIVE, 1.0);
            }
        }
        let dimension = host.dimension().to_owned();
        let first = lock(&FIRST)
            .get(&player.uuid)
            .filter(|(dim, _)| *dim == dimension)
            .map(|(_, pos)| *pos);
        if let Some(first) = first {
            let marker = GlueBox::span(first, first);
            // Out of reach of the next click: the colour of a failure.
            let colour = if closer_than(first, [player.position[0] as i32, player.position[1] as i32, player.position[2] as i32], rules::MAX_DISTANCE) {
                HIGHLIGHT
            } else {
                FAIL
            };
            outline(host, player.uuid, &marker, colour, 0.5);
        }
    }

    /// The charges owed (`hurtAndBreak`), where the inventory can be edited.
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let owed: Vec<(u128, usize)> = std::mem::take(&mut *lock(&CHARGES));
        for (uuid, slot) in owed {
            let Some(stack) = host.held_stack(uuid).filter(|s| &*s.item == ITEM) else {
                continue;
            };
            let new = use_charge(stack.damage).map(|damage| {
                let mut worn = stack.clone();
                worn.damage = damage;
                worn
            });
            let done = host
                .set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&stack), new)
                .is_ok();
            if !done {
                // The slot changed under us: try the next tick.
                lock(&CHARGES).push((uuid, slot));
            }
        }
    }
}

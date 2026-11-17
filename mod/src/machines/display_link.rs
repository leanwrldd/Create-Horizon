//! The Display Link (`DisplayLinkBlock`, `DisplayLinkBlockEntity`, `LinkWithBulbBlockEntity`,
//! `LinkBulbRenderer`, `DisplayLinkConfigurationPacket`): a small plate that reads the block it
//! sits on and writes text to a block up to 64 blocks away: a sign, a lectern, a row of nixie
//! tubes or a display board. It refreshes every 100 ticks (20 for the stopwatch), and at once
//! when its redstone power falls. A link with power on it stops writing.
//!
//! How it is aimed is `display_link_select.rs` (click the target with the link in hand, then
//! place it). The source, its options and the target line are set by clicks in place of Create's
//! screen. What the sources and targets are is `create_kinetics::rules::display_link`; reading
//! the source is `display_link_source.rs`, writing the target `display_link_target.rs`.
//!
//! Clicks, in place of the screen (`DisplayLinkScreen`):
//! - Right-click: the next "type of information" among those the block it reads offers; the first
//!   click on a new link picks the first.
//! - The Wrench on the face the link looks out of: the next line of the target to start at
//!   (`targetLine`); on any other face: the next value of the source's option (display mode, time
//!   format, number format...). Sneaking with the Wrench picks the link up.
//! - A name tag sets the label that goes before a single-line source's text; sneak-right-click with
//!   an empty hand clears it.
//!
//! Known differences from Create, for the README:
//! - Besides the screen, the Wrench doesn't turn the link (Create's `WrenchableDirectionalBlock`).
//! - The sources that are in this port only (see the rules' header); a link reading a redstone
//!   power or a nixie tube notices a change on the tick after, not at once.
//! - The remembered first lines of sign and lectern runs are kept in memory, not saved.
//! - Text is plain, the bulb is lit by a plain model (no additive glow), and Create's
//!   `StructureTransform` (moving the link on a contraption) is not here.
//! - The link looks at the neighbouring block's redstone as `neighbor_signal` gives it, which
//!   counts a redstone wire beside it whichever way the wire points.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::rules::arm_points::block_title;
use create_kinetics::rules::display_link::{Config, Source, bulb_glow, sources_for};
use create_kinetics::rules::steam_engine::{horizontal_angle, vertical_angle};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost, RightClickBlockEvent,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::{display_link_source as sources, display_link_target as targets};
use crate::util;

pub const ID: &str = "create:display_link";
const TUBE: &str = "create:block/display_link/tube";
const GLOW: &str = "create:block/display_link/glow";
const NAME_TAG: &str = "minecraft:name_tag";
const CENTRE: [f32; 3] = [0.5; 3];

const OFFSET: &str = "TargetOffset";
const LINE: &str = "TargetLine";
const SOURCE: &str = "Source";
const PULSE: &str = "Pulse";

struct DisplayLink;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(DisplayLink)
}

// --- the saved link ------------------------------------------------------------------------------

/// What the link keeps (`targetOffset`, `targetLine`, `activeSource` and `sourceConfig`), and the
/// game time of its last write for the bulb.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub offset: Pos,
    pub line: usize,
    pub source: Option<(Source, Config)>,
    pub pulse: i64,
}

pub fn load(nbt: &Compound) -> Link {
    let offset = match util::get(nbt, OFFSET) {
        Some(Tag::IntArray(at)) if at.len() == 3 => [at[0], at[1], at[2]],
        _ => [0; 3],
    };
    let source = match util::get(nbt, SOURCE) {
        Some(Tag::Compound(tag)) => read_source(tag),
        _ => None,
    };
    Link {
        offset,
        line: util::int(nbt, LINE).max(0) as usize,
        source,
        pulse: if util::has(nbt, PULSE) { util::long(nbt, PULSE) } else { i64::MIN },
    }
}

fn read_source(tag: &Compound) -> Option<(Source, Config)> {
    let Some(Tag::String(id)) = util::get(tag, "Id") else {
        return None;
    };
    let source = Source::from_id(&id.to_string_lossy())?;
    let mut config = Config::default();
    if let Some(Tag::String(label)) = util::get(tag, "Label") {
        config.label = label.to_string_lossy();
    }
    if let Some((key, _)) = source.option() {
        config.option = util::int(tag, key);
    }
    if util::has(tag, "StartTime") {
        config.start_time = Some(util::long(tag, "StartTime"));
    }
    Some((source, config))
}

fn source_tag(source: Source, config: &Config) -> Tag {
    let mut tag = Compound::new();
    tag.insert("Id".into(), Tag::String(source.id().into()));
    tag.insert("Label".into(), Tag::String(config.label.as_str().into()));
    if let Some((key, _)) = source.option() {
        tag.insert(key.into(), Tag::Int(config.option));
    }
    if let Some(start) = config.start_time {
        tag.insert("StartTime".into(), Tag::Long(start));
    }
    Tag::Compound(tag)
}

fn store(nbt: &mut Compound, link: &Link) {
    nbt.insert(OFFSET.into(), Tag::IntArray(link.offset.to_vec()));
    nbt.insert(LINE.into(), Tag::Int(link.line as i32));
    match &link.source {
        Some((source, config)) => {
            nbt.insert(SOURCE.into(), source_tag(*source, config));
        }
        None => {
            util::remove(nbt, SOURCE);
        }
    }
    if link.pulse != i64::MIN {
        nbt.insert(PULSE.into(), Tag::Long(link.pulse));
    }
}

fn step(pos: Pos, dir: Dir) -> Pos {
    let n = dir.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `DisplayLinkBlockEntity.getSourcePosition`: the block the link is attached to.
pub fn source_pos(pos: Pos, facing: Dir) -> Pos {
    step(pos, facing.opposite())
}

/// Whether the chunk of `pos` is loaded (`level.isLoaded`).
fn loaded(host: &dyn Host, pos: Pos) -> bool {
    host.chunk_is_loaded(host.dimension(), [pos[0] >> 4, pos[2] >> 4])
}

// --- gathering ----------------------------------------------------------------------------------

/// What a link's gathering did.
#[derive(Clone, Copy, Debug, Default)]
struct Gathered {
    /// Text was written to a target.
    wrote: bool,
}

/// `DisplayLinkBlockEntity.updateGatheredData`: reads the source, writes the target. `link` is
/// changed as the block entity would be (the source forgotten if its block changed, the config's
/// stopwatch start, the pulse); the caller saves it.
fn gather(host: &mut dyn Host, pos: Pos, facing: Dir, link: &mut Link) -> Gathered {
    let src = source_pos(pos, facing);
    let tgt = [pos[0] + link.offset[0], pos[1] + link.offset[1], pos[2] + link.offset[2]];
    if !loaded(&*host, src) || !loaded(&*host, tgt) {
        return Gathered::default();
    }
    let offered = sources_for(&host.block(src));
    if let Some((active, _)) = &link.source
        && !offered.contains(active)
        // A source block that is gone (or not yet there) leaves the setting alone.
        && host.block(src) != "minecraft:air"
    {
        link.source = None;
    }
    let (Some((source, mut config)), Some(target)) = (link.source.clone(), targets::target_at(&*host, tgt)) else {
        return Gathered::default();
    };
    let reading = sources::read(host, source, src, facing);
    targets::deliver(host, pos, tgt, target, link.line, source, &mut config, &reading);
    link.source = Some((source, config));
    link.pulse = util::game_time(host);
    Gathered { wrote: true }
}

/// Runs a gathering for the link at `pos` from outside its own tick (a click): the block entity
/// is read, gathered with and written back.
fn gather_now(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    let mut link = load(&entity.nbt);
    let before = link.clone();
    gather(host, pos, facing, &mut link);
    if link != before {
        host.change_mod_block_entity(pos, &mut |e| store(&mut e.nbt, &link));
        host.block_entity_sync(pos);
    }
}

/// `DisplayLinkBlock.shouldBePowered`: power on any side but the one it is attached to.
fn should_be_powered(host: &mut dyn Host, pos: Pos, facing: Dir) -> bool {
    let open: Vec<Dir> = Dir::ALL.into_iter().filter(|d| *d != facing.opposite()).collect();
    signal::read_sides(host, pos, &open) > 0
}

// --- per-link memory that Create keeps in fields it doesn't save -------------------------------------

/// `refreshTicks`, and the last reading of a source that is polled (`neighborChanged`).
#[derive(Default)]
struct Memory {
    refresh: BTreeMap<(String, Pos), i32>,
    seen: BTreeMap<(String, Pos), String>,
}

static MEMORY: Mutex<Memory> = Mutex::new(Memory { refresh: BTreeMap::new(), seen: BTreeMap::new() });

fn memory<R>(f: impl FnOnce(&mut Memory) -> R) -> R {
    let mut guard = MEMORY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut guard)
}

/// A source whose block has no tick of its own to tell the link something changed: the link
/// looks at it each tick (`RedstonePowerDisplaySource`, `NixieTubeBlockEntity.notifyGatherers`).
fn is_polled(source: Source) -> bool {
    matches!(source, Source::RedstonePower | Source::NixieTube)
}

// --- messages ------------------------------------------------------------------------------------------

fn say(host: &mut dyn Host, line: String) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line));
    }
}

fn say_red(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::colored(line.to_owned(), 0xFF5555));
    }
}

/// A line telling what the link now does.
fn describe(host: &dyn Host, pos: Pos, facing: Dir, link: &Link) -> String {
    let src = host.block(source_pos(pos, facing));
    let mut line = match &link.source {
        Some((source, config)) => {
            let mut text = format!("{}: {}", block_title(&src), source.name());
            if let Some((_, values)) = source.option() {
                let at = (config.option.max(0) as usize).min(values.len() - 1);
                text.push_str(&format!(" ({})", values[at]));
            }
            if !config.label.is_empty() {
                text.push_str(&format!(", label \"{}\"", config.label));
            }
            text
        }
        None => format!("{}: not a display source", block_title(&src)),
    };
    let tgt = [pos[0] + link.offset[0], pos[1] + link.offset[1], pos[2] + link.offset[2]];
    match targets::target_at(host, tgt) {
        Some(target) => {
            line.push_str(&format!(" -> {} ({})", block_title(&host.block(tgt)), target.line_text(link.line)));
        }
        None => line.push_str(" -> not a display target"),
    }
    line
}

// --- the hooks -------------------------------------------------------------------------------------------

impl Hooks for DisplayLink {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend([TUBE, GLOW].map(str::to_owned));
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(OFFSET.into(), Tag::IntArray(vec![0, 0, 0]));
            nbt.insert(LINE.into(), Tag::Int(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `DisplayLinkBlock.neighborChanged` and `DisplayLinkBlockEntity.tick`.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(&*host, pos, "facing") else {
            return;
        };
        // Power: the block's `powered` follows it; when it falls the source resets and writes.
        let powered = should_be_powered(host, pos, facing);
        let was = host.block_property(pos, "powered").as_deref() == Some("true");
        if powered != was {
            host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
        }
        let mut link = load(&event.entity.nbt);
        let Some((source, _)) = link.source.clone() else {
            return;
        };
        let key = (host.dimension().to_owned(), pos);
        let now = util::game_time(&*host);
        let mut run = false;
        if was && !powered {
            // `onNoLongerPowered`.
            memory(|m| m.refresh.insert(key.clone(), 0));
            if let Some((_, config)) = &mut link.source {
                config.reset(source, now);
            }
            run = true;
        } else {
            let mut due = memory(|m| {
                let ticks = m.refresh.entry(key.clone()).or_insert(0);
                *ticks += 1;
                *ticks >= source.refresh_ticks()
            });
            if is_polled(source) && !due {
                let reading = sources::read(host, source, source_pos(pos, facing), facing);
                let signature = format!("{reading:?}");
                due = memory(|m| m.seen.insert(key.clone(), signature.clone()).is_some_and(|old| old != signature));
            }
            if due {
                // `tickSource`.
                memory(|m| m.refresh.insert(key.clone(), 0));
                run = !powered;
            }
        }
        if !run {
            return;
        }
        let before = link.clone();
        let wrote = gather(host, pos, facing, &mut link).wrote;
        if link != before {
            store(&mut event.entity.nbt, &link);
        }
        if wrote {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = Compound::new();
        tag.insert(PULSE.into(), Tag::Long(load(&event.entity.nbt).pulse));
        Some(tag)
    }

    /// `LinkBulbRenderer`: the bulb flashes when the link has written.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        if !util::has(&event.entity.nbt, PULSE) {
            return;
        }
        let pulse = util::long(&event.entity.nbt, PULSE);
        let draw = &mut *event.draw;
        let ticks = draw.game_time.saturating_sub(pulse) as f32 + draw.partial;
        let Some(brightness) = bulb_glow(ticks) else {
            return;
        };
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], horizontal_angle(facing) + 180.0, CENTRE);
        draw.rotate_about([1.0, 0.0, 0.0], -vertical_angle(facing) - 90.0, CENTRE);
        let offset = if facing.axis() == create_kinetics::Axis::Y {
            [5.0 / 16.0, 7.0 / 16.0, 11.0 / 16.0]
        } else {
            [11.0 / 16.0, 7.0 / 16.0, 5.0 / 16.0]
        };
        draw.translate(offset);
        host.draw_model(draw, TUBE, 0xFF_FFFF);
        host.draw_model(draw, GLOW, u32::from(brightness) * 0x01_0101);
        draw.pop();
    }

    /// `DisplayLinkBlock.getStateForPlacement`: it sits on the face that was clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        level.state_with_property(event.state, "facing", util::dir(context.face).name()).map(Some)
    }

    /// `DisplayLinkBlock.useWithoutItem`: the screen, here the next type of information.
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
        let held = host.held_stack(player.uuid);
        let tool = held.as_ref().is_some_and(|s| &*s.item == util::WRENCH || &*s.item == NAME_TAG);
        if tool {
            return Flow::Continue;
        }
        if player.sneaking {
            // Sneaking passes to the item: with an empty hand, clear the label.
            if held.is_some() || !signal::hands_empty(host) {
                return Flow::Continue;
            }
            if !util::first_click(host, player.uuid, pos) {
                return Flow::Cancel;
            }
            edit(host, pos, |link| {
                if let Some((_, config)) = &mut link.source {
                    config.label.clear();
                }
            });
            say(host, "Label cleared".to_owned());
            return Flow::Cancel;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        cycle_source(host, pos);
        Flow::Cancel
    }

    /// The Wrench and the name tag on a link.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let item = &*event.stack.item;
        if item == util::WRENCH {
            let Some(player) = host.acting_player() else {
                return Flow::Continue;
            };
            if player.sneaking {
                return util::wrench(host, pos, |_| {});
            }
            if !util::first_click(host, player.uuid, pos) {
                return Flow::Cancel;
            }
            let Some(facing) = util::facing(&*host, pos, "facing") else {
                return Flow::Cancel;
            };
            if event.face == facing.normal() {
                step_line(host, pos);
            } else {
                step_option(host, pos);
            }
            return Flow::Cancel;
        }
        if item == NAME_TAG {
            let name = event.stack.extra.as_ref().and_then(|d| d.custom_name_plain());
            let Some(name) = name else {
                return Flow::Continue;
            };
            let Some(player) = host.acting_player() else {
                return Flow::Continue;
            };
            if player.sneaking {
                return Flow::Continue;
            }
            if !util::first_click(host, player.uuid, pos) {
                return Flow::Cancel;
            }
            edit(host, pos, |link| {
                if let Some((_, config)) = &mut link.source {
                    config.label = name.clone();
                }
            });
            say(host, format!("Label: {name}"));
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

/// Changes the saved link and writes the target again, as `DisplayLinkConfigurationPacket` does.
fn edit(host: &mut dyn Host, pos: Pos, change: impl FnOnce(&mut Link)) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let mut link = load(&entity.nbt);
    change(&mut link);
    host.change_mod_block_entity(pos, &mut |e| store(&mut e.nbt, &link));
    gather_now(host, pos);
    host.block_entity_sync(pos);
}

/// The screen's "type of information" scroll: the next source the block it reads offers.
fn cycle_source(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    let link = load(&entity.nbt);
    if link.offset == [0; 3] {
        say_red(host, "Link has no valid target, try placing it again");
        return;
    }
    let offered = sources_for(&host.block(source_pos(pos, facing)));
    if offered.is_empty() {
        say_red(host, "Not a Display Source");
        return;
    }
    let next = match &link.source {
        Some((active, _)) => offered.iter().position(|s| s == active).map_or(0, |i| (i + 1) % offered.len()),
        None => 0,
    };
    edit(host, pos, |link| link.source = Some((offered[next], Config::default())));
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let text = describe(&*host, pos, facing, &load(&entity.nbt));
    say(host, text);
}

/// The screen's line scroll (`targetLineSelector`): the next row of the target to start at.
fn step_line(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let link = load(&entity.nbt);
    let tgt = [pos[0] + link.offset[0], pos[1] + link.offset[1], pos[2] + link.offset[2]];
    let rows = targets::target_at(&*host, tgt).map_or(1, |t| targets::stats(&*host, tgt, t).rows).max(1);
    edit(host, pos, |link| link.line = (link.line + 1) % rows);
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    if let Some(entity) = host.mod_block_entity(pos) {
        let text = describe(&*host, pos, facing, &load(&entity.nbt));
        say(host, text);
    }
}

/// The source's option widget (`initConfigurationWidgets`): its next value.
fn step_option(host: &mut dyn Host, pos: Pos) {
    let Some(facing) = util::facing(&*host, pos, "facing") else {
        return;
    };
    edit(host, pos, |link| {
        if let Some((source, config)) = &mut link.source {
            config.step_option(*source);
        }
    });
    if let Some(entity) = host.mod_block_entity(pos) {
        let text = describe(&*host, pos, facing, &load(&entity.nbt));
        say(host, text);
    }
}

/// Used by the item: the target is written once the link is placed.
pub(super) fn aim(host: &mut dyn Host, pos: Pos, offset: Pos) {
    host.change_mod_block_entity(pos, &mut |e| {
        e.nbt.insert(OFFSET.into(), Tag::IntArray(offset.to_vec()));
    });
    host.block_entity_sync(pos);
}

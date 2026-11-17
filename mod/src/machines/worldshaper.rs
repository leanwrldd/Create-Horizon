//! The Handheld Worldshaper (`WorldshaperItem`, `ZapperItem`, the brushes and tools): a creative
//! tool that fills, places, replaces, clears or overlays a shape of blocks where you look. The
//! shapes and tools are `create_kinetics::rules::worldshaper`; its header lists the known
//! differences from Create (no screen, one size for every dimension, no Flatten, no patterns).
//!
//! Using it (stand-in for Create's screen; the settings show on the action bar):
//! - right-click: shape the blocks where you look (up to 128 blocks away);
//! - left-click a block: use that block (its default state) for Fill, Place, Replace and Overlay;
//! - sneak + right-click with the other hand holding nothing: next brush (cuboid, sphere, cylinder,
//!   surface, cluster); a Wrench: next tool; a stick: size up (back to the smallest after the
//!   biggest); paper: next placement (merged, attached, inserted).
//!
//! The settings are saved in the item as registered components (`create:shaper_brush`,
//! `create:shaper_tool`, `create:shaper_placement_options`, `create:shaper_block_used`,
//! `create:shaper_size`), changed on the next tick (a player's own hook can't change the stack).

use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::worldshaper::{self as rules, Brush, Edit, Placement, Pos, Terrain, Tool};
use create_kinetics::{Dir, rules::worldshaper::UNBREAKABLE};
use crate::compat::{
    ComponentTypesEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack, LeftClickBlockEvent,
    LevelTickPostEvent, NativeInventoryKind, Tag, Text, UseItemEvent, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::util;

pub const ID: &str = "create:handheld_worldshaper";

const BRUSH: &str = "create:shaper_brush";
const TOOL: &str = "create:shaper_tool";
const PLACEMENT: &str = "create:shaper_placement_options";
const BLOCK: &str = "create:shaper_block_used";
const SIZE: &str = "create:shaper_size";
/// `WorldshaperItem.getZappingRange`.
const RANGE: f64 = 128.0;

struct Worldshaper;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Worldshaper)
}

/// The item's settings with Create's defaults (`Cuboid`, `Fill`, `Merged`).
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    brush: Brush,
    tool: Tool,
    placement: Placement,
    block: Option<String>,
    size: i32,
}

fn string(stack: &ItemStack, id: &str) -> Option<String> {
    match stack.data()?.other(id)? {
        Tag::String(value) => Some(value.to_string_lossy()),
        _ => None,
    }
}

fn read(stack: &ItemStack) -> Settings {
    Settings {
        brush: string(stack, BRUSH).and_then(|n| Brush::from_name(&n)).unwrap_or(Brush::Cuboid),
        tool: string(stack, TOOL).and_then(|n| Tool::from_name(&n)).unwrap_or(Tool::Fill),
        placement: string(stack, PLACEMENT)
            .and_then(|n| Placement::from_name(&n))
            .unwrap_or(Placement::Merged),
        block: string(stack, BLOCK),
        size: match stack.data().and_then(|d| d.other(SIZE)) {
            Some(Tag::Int(size)) => *size,
            _ => 3,
        },
    }
}

fn write(host: &dyn Host, stack: &mut ItemStack, settings: &Settings) {
    let set = |stack: &mut ItemStack, id: &str, tag: Tag| {
        let _ = host.set_component(stack, id, tag);
    };
    set(stack, BRUSH, Tag::String(settings.brush.name().into()));
    set(stack, TOOL, Tag::String(settings.tool.name().into()));
    set(stack, PLACEMENT, Tag::String(settings.placement.name().into()));
    set(stack, SIZE, Tag::Int(settings.size));
    if let Some(block) = &settings.block {
        set(stack, BLOCK, Tag::String(block.as_str().into()));
    }
}

fn summary(settings: &Settings) -> String {
    let (_, max) = settings.brush.size_range();
    format!(
        "{} {} (size {}/{}), {}, {}",
        settings.brush.name(),
        settings.tool.name(),
        settings.brush.params(settings.size)[0],
        max,
        settings.placement.name(),
        settings.block.as_deref().unwrap_or("no block chosen")
    )
}

/// Edits waiting for the next tick.
static EDITS: LazyLock<Mutex<Vec<(u128, usize, ItemStack)>>> = LazyLock::new(Default::default);

fn queue(uuid: u128, slot: usize, stack: ItemStack) {
    if let Ok(mut edits) = EDITS.lock() {
        edits.retain(|(who, _, _)| *who != uuid);
        edits.push((uuid, slot, stack));
    }
}

fn tell(host: &mut dyn Host, line: String) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line));
    }
}

/// The world as a brush asks of it.
struct World<'a> {
    host: &'a dyn Host,
}

impl Terrain for World<'_> {
    fn replaceable(&self, pos: Pos) -> bool {
        signal::replaceable(&self.host.block(pos))
    }

    fn unbreakable(&self, pos: Pos) -> bool {
        UNBREAKABLE.contains(&self.host.block(pos).as_str())
    }

    fn same_block(&self, a: Pos, b: Pos) -> bool {
        self.host.block(a) == self.host.block(b)
    }

    fn solid_side(&self, pos: Pos, _: Dir) -> bool {
        !self.replaceable(pos) && !self.host.block_collision(pos).is_empty()
    }
}

/// `ZapperItem.use` and `WorldshaperItem.activate`.
fn activate(host: &mut dyn Host, settings: &Settings) {
    let Some(player) = host.acting_player() else {
        return;
    };
    if settings.tool.requires_block() && settings.block.is_none() {
        host.play_sound("create:deny", player.position, 4, 1.0, 1.0);
        tell(host, "Left-click a block to choose the block to use".to_owned());
        return;
    }
    let eye = [player.position[0], player.position[1] + 1.62, player.position[2]];
    let (yaw, pitch) = (f64::from(player.yaw).to_radians(), f64::from(player.pitch).to_radians());
    let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let hit = {
        let world = &*host;
        rules::raycast(eye, look, RANGE, &|p| !signal::replaceable(&world.block(p)))
    };
    let Some((target, face)) = hit else {
        return;
    };
    let params = settings.brush.params(settings.size);
    let tool = settings.brush.redirect(settings.tool);
    let world = World { host: &*host };
    let positions: Vec<Pos> = match settings.brush {
        Brush::Surface | Brush::Cluster => rules::dynamic_positions(
            &world,
            settings.brush == Brush::Surface,
            params,
            target,
            face,
            tool,
        ),
        brush => {
            let centre = rules::offset(brush, params, face, settings.placement);
            let at = [target[0] + centre[0], target[1] + centre[1], target[2] + centre[2]];
            rules::shape(brush, params)
                .into_iter()
                .map(|p| [at[0] + p[0], at[1] + p[1], at[2] + p[2]])
                .collect()
        }
    };
    let painted = settings.block.clone().unwrap_or_default();
    let edits = rules::run(&world, tool, &positions, &|p| world.host.block(p) == painted);
    let dimension = host.dimension();
    let (Some(air), paint) = (host.default_state("minecraft:air"), host.default_state(&painted)) else {
        return;
    };
    for edit in edits {
        match edit {
            Edit::Air(pos) => {
                host.set_block(dimension, pos, air, util::UPDATE_ALL);
            }
            Edit::Set(pos) => {
                if let Some(state) = paint {
                    host.set_block(dimension, pos, state, util::UPDATE_ALL);
                }
            }
        }
    }
}

/// A click with the Worldshaper in the main hand.
fn click(host: &mut dyn Host, stack: &ItemStack) -> Flow {
    let Some(player) = host.acting_player() else {
        return Flow::Continue;
    };
    if player.game_mode == util::SPECTATOR || host.held_stack(player.uuid).as_ref() != Some(stack) {
        return Flow::Continue;
    }
    let mut settings = read(stack);
    if !player.sneaking {
        activate(host, &settings);
        return Flow::Cancel;
    }
    let off = host.player_equipment(player.uuid, EquipmentSlot::OffHand);
    match off.as_ref().map(|s| &*s.item) {
        None => {
            settings.brush = settings.brush.next();
            settings.tool = if settings.brush.tools().contains(&settings.tool) {
                settings.tool
            } else {
                settings.brush.tools()[0]
            };
            let (min, max) = settings.brush.size_range();
            settings.size = settings.size.clamp(min, max);
        }
        Some(util::WRENCH) => settings.tool = rules::next_tool(settings.brush, settings.tool),
        Some("minecraft:stick") => {
            let (min, max) = settings.brush.size_range();
            settings.size = if settings.size >= max { min } else { settings.size.max(min) + 1 };
        }
        Some("minecraft:paper") => settings.placement = settings.placement.next(),
        Some(_) => return Flow::Continue,
    }
    let mut edited = stack.clone();
    write(&*host, &mut edited, &settings);
    queue(player.uuid, usize::from(player.selected_slot), edited);
    tell(host, summary(&settings));
    Flow::Cancel
}

impl Hooks for Worldshaper {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        for id in [BRUSH, TOOL, PLACEMENT, BLOCK, SIZE] {
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
        // A held button repeats the click every few ticks: each is a shaping, the off hand's turn is not.
        if !util::first_click(host, player.uuid, event.pos) {
            return Flow::Cancel;
        }
        click(host, event.stack)
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
        click(host, event.stack)
    }

    /// `ZapperInteractionHandler`: left-clicking a block with the Worldshaper chooses it.
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(stack) = host.held_stack(player.uuid).filter(|s| &*s.item == ID) else {
            return Flow::Continue;
        };
        let block = host.block(event.pos);
        let mut settings = read(&stack);
        if settings.block.as_deref() != Some(block.as_str()) {
            settings.block = Some(block.clone());
            let mut edited = stack.clone();
            write(&*host, &mut edited, &settings);
            queue(player.uuid, usize::from(player.selected_slot), edited);
        }
        tell(host, format!("Block to use: {block}"));
        // The tool never breaks blocks (`canAttackBlock`).
        Flow::Cancel
    }

    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let waiting: Vec<(u128, usize, ItemStack)> = match EDITS.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => return,
        };
        for (uuid, slot, edited) in waiting {
            let Some(held) = host.held_stack(uuid).filter(|held| &*held.item == ID) else {
                continue;
            };
            let _ = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, slot, Some(&held), Some(edited));
        }
    }
}

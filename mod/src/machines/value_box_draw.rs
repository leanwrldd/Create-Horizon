//! What a player sees of value boxes (`ValueBox`, `ValueBoxRenderer`, `ScrollValueRenderer`, `FilteringRenderer`,
//! `ValueSettingsClient.render`): the filter item on its slot (always), and, while a block with boxes is looked at,
//! each box's content (its number, its option's name, its filter's amount), an outline on the box the crosshair is
//! on, and the label and hint at the bottom of the screen. The positions and what a box holds are the rules in
//! `create_kinetics::rules::value_box*`; this only reads the client's world and draws.
//!
//! Known differences from Create, for the README:
//! - The outline is four lines round the box and its text is name-tag text that turns to face the camera: Horizon
//!   draws neither Create's outline textures nor text lying on the box. An option is shown by its name, not its icon
//!   (those are in Create's own art sheet, which this port doesn't import).
//! - The filter item is drawn with the box's own pose as Create's renderer does (`ValueBoxRenderer`); the glint of an
//!   enchanted item is frozen and the hover fade of the hint is a plain on and off.
//! - The checkered face highlight of Create's filter box (`AllSpecialTextures.THIN_CHECKERED`) isn't drawn.

use create_kinetics::rules::value_box::{self as geo, Pose, Props, Vec3};
use create_kinetics::rules::value_box_blocks::{self as table, BoxDef, Kind};
use create_kinetics::rules::value_box_board::{self as board, Behaviour, Values};
use create_kinetics::{Axis, Dir, Pos};
use crate::compat::{
    BlockEntitySubmitEvent, DisplayContext, Hooks, Host, HudEvent, ItemStack, NativeScreenKind, WorldPrimitivesEvent,
};

use super::value_box::{self as vb, Hit};

struct Draw;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Draw)
}

/// The block the crosshair is on, with where.
#[derive(Clone, Debug)]
pub struct Looked {
    pub pos: Pos,
    pub block: String,
    pub hit: Hit,
}

/// `mc.hitResult` of a block with value boxes, with the face and point worked out as the server does.
pub fn looked_at(host: &dyn Host) -> Option<Looked> {
    let client = host.client_state()?;
    if client.screen != NativeScreenKind::Playing {
        return None;
    }
    let hit = host.raycast(vb::REACH as f32)?;
    if !table::has_boxes(&hit.block) {
        return None;
    }
    let pos = [hit.block_x, hit.block_y, hit.block_z];
    let eye = client.eye_position?;
    let look = geo::view_vector(f64::from(client.yaw), f64::from(client.pitch));
    let at = [hit.x - f64::from(pos[0]), hit.y - f64::from(pos[1]), hit.z - f64::from(pos[2])];
    let guess = geo::nearest_face(at).normal();
    let hit_side = vb::hit_of(host, eye, look, pos, guess)?;
    Some(Looked { pos, block: hit.block, hit: hit_side })
}

/// One box of the looked-at block as it is drawn.
pub struct Shown {
    pub def: &'static BoxDef,
    pub pose: Pose,
    /// The crosshair is on it.
    pub hovered: bool,
    pub values: Values,
    pub filter: Option<ItemStack>,
    /// `FilteringBehaviour.isCountVisible`.
    pub counts: bool,
}

/// The boxes of the looked-at block that are there on the face looked at.
pub fn shown(host: &dyn Host, looked: &Looked) -> Vec<Shown> {
    let side = looked.hit.side;
    vb::with_props(host, looked.pos, &looked.block, |p, nbt| {
        let Some(nbt) = nbt else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for def in table::boxes(&looked.block) {
            if !(def.active)(p) || !def.slot.exists(p, side) {
                continue;
            }
            let Some(pose) = def.slot.pose(p, side) else {
                continue;
            };
            let values = vb::values_of(host, nbt, def, side);
            out.push(Shown {
                def,
                pose,
                hovered: def.slot.test_hit(p, side, looked.hit.local),
                counts: vb::takes_board(def, p, &values),
                filter: def.is_filter().then(|| vb::filter_item(nbt, def, side)).flatten(),
                values,
            });
        }
        out
    })
}

/// A language key in the player's language (the key itself where the package lacks it, as Minecraft shows it).
fn say(host: &dyn Host, key: &str) -> String {
    super::goggles::lang(host).translate(key, &[])
}

// --- the world --------------------------------------------------------------------------------------------

const WHITE: u32 = 0xFF_FFFFFF;
const TEXT: u32 = 0xFF_EDEDED;
/// How far in front of the box (in its own units) the outline and the text float.
const LIFT: f64 = 0.06;

fn corner_lines(primitives: &mut crate::compat::WorldPrimitives, origin: Vec3, corners: [Vec3; 4], width: f32) {
    let at = |p: Vec3| [origin[0] + p[0], origin[1] + p[1], origin[2] + p[2]];
    for i in 0..4 {
        primitives.line(at(corners[i]), at(corners[(i + 1) % 4]), WHITE, width);
    }
}

impl Hooks for Draw {
    /// `ScrollValueRenderer.tick` and `FilteringRenderer.tick`: the boxes of the block looked at.
    fn world_primitives(&self, host: &dyn Host, event: &mut WorldPrimitivesEvent<'_>) {
        if !host.window() || host.sneaking() {
            return;
        }
        let Some(looked) = looked_at(host) else {
            return;
        };
        let origin = [f64::from(looked.pos[0]), f64::from(looked.pos[1]), f64::from(looked.pos[2])];
        for item in shown(host, &looked) {
            let tint = item.def.slot.override_color().map_or(TEXT, |rgb| 0xFF00_0000 | rgb);
            match item.def.kind {
                Kind::Value { behaviour, .. } => {
                    let (text, scale) = match behaviour {
                        Behaviour::Options { options } => {
                            let key = options.get(item.values.scroll.max(0) as usize).copied().unwrap_or("");
                            (say(host, key), 0.28)
                        }
                        _ => {
                            let text = board::box_text(behaviour, &item.values);
                            let scale = if text.len() <= 2 { 0.85 } else { 0.6 };
                            (text, scale)
                        }
                    };
                    let at = item.pose.front(LIFT * 2.0);
                    event.primitives.text([origin[0] + at[0], origin[1] + at[1], origin[2] + at[2]], text, tint, scale);
                }
                Kind::Filter { .. } => {
                    // The count's label beside the filter item (`ItemValueBox.renderContents`).
                    let label = board::count_label(item.counts, &item.values);
                    if !label.is_empty() {
                        let mut at = item.pose.front(LIFT * 2.0);
                        at[1] -= 0.17;
                        event.primitives.text([origin[0] + at[0], origin[1] + at[1], origin[2] + at[2]], label, TEXT, 0.8);
                    }
                }
            }
            if item.hovered {
                let wide = item.filter.is_some() || matches!(item.def.kind, Kind::Value { behaviour: Behaviour::Options { .. }, .. });
                corner_lines(event.primitives, origin, item.pose.corners(0.5, LIFT), if wide { 3.0 } else { 2.0 });
            }
        }
    }

    /// `ValueSettingsClient.render`: the label of the box looked at and what to do with it.
    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !host.window() || host.sneaking() {
            return;
        }
        let Some(client) = host.client_state() else {
            return;
        };
        if client.screen != NativeScreenKind::Playing || client.gui_hidden {
            return;
        }
        let Some(looked) = looked_at(&*host) else {
            return;
        };
        let Some(item) = shown(&*host, &looked).into_iter().find(|item| item.hovered) else {
            return;
        };
        let mut lines = vec![say(&*host, item.def.label())];
        match item.def.kind {
            Kind::Value { .. } => lines.push(say(&*host, "create.gui.value_settings.hold_to_edit")),
            Kind::Filter { .. } => {
                let tip = if item.filter.is_none() {
                    "create.logistics.filter.click_to_set"
                } else {
                    "create.logistics.filter.click_to_replace"
                };
                lines.push(say(&*host, tip));
                if item.counts {
                    lines.push(say(&*host, "create.logistics.filter.hold_to_set_amount"));
                }
            }
        }
        let (w, h) = (client.gui_size[0], client.gui_size[1]);
        let mut y = h - 75.0 - lines.len() as f32 * 12.0;
        for (i, line) in lines.iter().enumerate() {
            let width = host.text_width(line);
            let rgb = if i == 0 { 0xFB_DC7D } else { 0xFF_FFFF };
            host.text(event.batch, line, [(w - width) / 2.0, y], rgb, true);
            y += 12.0;
        }
    }

    /// `FilteringRenderer.renderOnBlockEntity`: the filter item on its slot, drawn with the slot's own pose.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let defs = table::boxes(event.block);
        if !defs.iter().any(BoxDef::is_filter) {
            return;
        }
        let nbt = &event.entity.nbt;
        let mut drawn = false;
        for def in defs.iter().filter(|def| def.is_filter()) {
            let sides: &[Dir] = if def.slot.sided() { &Dir::ALL } else { &[Dir::Up] };
            for &side in sides {
                let Some(stack) = vb::filter_item(nbt, def, side) else {
                    continue;
                };
                let pose = vb::with_props_nbt(host, event.pos, event.block, Some(nbt), |p: &Props| {
                    if (def.active)(p) && def.slot.exists(p, side) { def.slot.pose(p, side) } else { None }
                });
                let Some(pose) = pose else {
                    continue;
                };
                if !drawn {
                    // Over the block's own drawing, which stays as it was.
                    event.draw.still();
                    drawn = true;
                }
                draw_filter(host, event, &pose, &stack);
            }
        }
    }
}

/// `ValueBoxRenderer.renderItemIntoValueBox`: the box's pose, then the item at a flat item's half size (a block item
/// at the box's size) nudged to the front for the few thin blocks Create names.
fn draw_filter(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>, pose: &Pose, stack: &ItemStack) {
    let block_item = host.default_state(&stack.item).is_some();
    let scale = (if block_item { 1.0 } else { 0.5 }) + 1.0 / 64.0;
    let nudge = if nudged(&stack.item) { -0.1 } else { 0.0 };
    let z = (if block_item { 0.0 } else { -0.15 }) + nudge;
    let draw = &mut *event.draw;
    draw.push();
    draw.translate([pose.offset[0] as f32, pose.offset[1] as f32, pose.offset[2] as f32]);
    for (axis, degrees) in &pose.rotations {
        let vector = match axis {
            Axis::X => [1.0, 0.0, 0.0],
            Axis::Y => [0.0, 1.0, 0.0],
            Axis::Z => [0.0, 0.0, 1.0],
        };
        draw.rotate_about(vector, *degrees as f32, [0.0; 3]);
    }
    draw.scale([pose.scale as f32; 3]);
    draw.scale([scale as f32; 3]);
    draw.translate([0.0, 0.0, z as f32]);
    host.draw_item(draw, stack, DisplayContext::Fixed, None, 0);
    draw.pop();
}

/// `ValueBoxRenderer.customZOffset`: the block items Create pulls a tenth forward (shafts, fences, buttons, end rods).
fn nudged(item: &str) -> bool {
    let name = item.strip_prefix("minecraft:").unwrap_or(item);
    item == "create:shaft"
        || item == "create:powered_shaft"
        || name.ends_with("_fence")
        || name.ends_with("_button")
        || name == "end_rod"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thin_blocks_are_nudged() {
        assert!(nudged("create:shaft"));
        assert!(nudged("minecraft:oak_fence"));
        assert!(nudged("minecraft:stone_button"));
        assert!(nudged("minecraft:end_rod"));
        assert!(!nudged("minecraft:stone"));
    }
}

//! The Nixie Tube, in sixteen colours (`NixieTubeBlock`, `NixieTubeBlockEntity`,
//! `NixieTubeRenderer`): two glowing digits that show the redstone strength reaching the block,
//! or the text a name tag or a Display Link gave. Tubes of one colour side by side read as one
//! row: a name tag, a Display Link or a dye works on the whole row. The rules are
//! `create_kinetics::rules::nixie_tube`; the glyphs are drawn with `Host::draw_text`.
//!
//! Known differences from Create, for the README:
//! - Redstone is read every other tick rather than when a neighbour changes, and from all six
//!   sides (Create adds the signal "from above" of every side but the one it hangs on).
//! - A tube has the cube's hit box and no waterlogging (see the rules' header); the train signal
//!   and ComputerCraft displays are not here.
//! - A Display Link reading a tube notices a change on its next tick rather than at once.
//! - The text is plain (Create keeps a JSON text component and resolves selectors in it).
//! - The tube glows by `Host::draw_text`'s full-bright text, in the front and back colours with
//!   Create's flicker; there is no additive glow.

use create_kinetics::rules::nixie_tube::{self as rules, Face};
use create_kinetics::rules::steam_engine::horizontal_angle;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack,
    PlaceBlockEvent, RightClickBlockEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::diodes_signal as signal;
use super::encased;
use crate::util;

const CENTRE: [f32; 3] = [0.5; 3];
const NAME_TAG: &str = "minecraft:name_tag";
/// `RedstoneStrength`, `CustomText` and `CustomTextIndex`: Create's keys.
const STRENGTH: &str = "RedstoneStrength";
const TEXT: &str = "CustomText";
const INDEX: &str = "CustomTextIndex";
/// Ticks between looks at the redstone around a tube.
const POLL: i64 = 2;

struct Nixie;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Nixie)
}

pub fn is_tube(block: &str) -> bool {
    rules::dye_of_block(block).is_some()
}

/// A tube's own text, if one was written (`customText`), its index in its row and the strength.
fn custom_text(nbt: &Compound) -> Option<String> {
    match util::get(nbt, TEXT) {
        Some(Tag::String(text)) => Some(text.to_string_lossy()),
        _ => None,
    }
}

/// The face and facing of the tube block at `pos`.
fn face_of(host: &dyn Host, pos: Pos) -> Option<(Face, Dir)> {
    let face = host.block_property(pos, "double_face").and_then(|f| Face::from_name(&f))?;
    let facing = util::facing(host, pos, "facing")?;
    Some((face, facing))
}

/// `NixieTubeBlock.walkNixies`: the tubes of the row `start` is in, left to right, equal to it but
/// for the colour (`areNixieBlocksEqual`).
pub fn row_of(host: &dyn Host, start: Pos) -> Vec<Pos> {
    let Some((face, facing)) = face_of(host, start) else {
        return Vec::new();
    };
    let same = |at: Pos| is_tube(&host.block(at)) && face_of(host, at) == Some((face, facing));
    rules::row(start, rules::left_direction(face, facing), same)
}

/// What a Display Link reads from the tube at `pos` (`getFullText`), `None` where there is none.
pub fn text_at(host: &dyn Host, pos: Pos) -> Option<String> {
    let entity = host.mod_block_entity(pos)?;
    Some(rules::full_text(custom_text(&entity.nbt).as_deref(), util::int(&entity.nbt, STRENGTH)))
}

/// `displayCustomText`: a tube takes `text` for the characters from `index` in its row, unless it
/// already shows that text. True when it changed.
pub fn display_text(host: &mut dyn Host, pos: Pos, text: &str, index: usize) -> bool {
    let Some(entity) = host.mod_block_entity(pos) else {
        return false;
    };
    if custom_text(&entity.nbt).as_deref() == Some(text) {
        return false;
    }
    host.change_mod_block_entity(pos, &mut |e| {
        e.nbt.insert(TEXT.into(), Tag::String(text.into()));
        e.nbt.insert(INDEX.into(), Tag::Int(index as i32));
    });
    host.block_entity_sync(pos);
    true
}

/// `clearCustomText`: back to showing redstone.
fn clear_text(host: &mut dyn Host, pos: Pos) {
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    if custom_text(&entity.nbt).is_none() {
        return;
    }
    host.change_mod_block_entity(pos, &mut |e| {
        util::remove(&mut e.nbt, TEXT);
        e.nbt.insert(INDEX.into(), Tag::Int(0));
    });
    host.block_entity_sync(pos);
}

/// `NixieTubeBlock.getPower`, as far as the host reads one.
fn power_at(host: &mut dyn Host, pos: Pos) -> i32 {
    signal::read_sides(host, pos, &Dir::ALL)
}

/// The dye an item is, from its id.
fn dye_of(stack: &ItemStack) -> Option<&'static str> {
    rules::dye_of_item(&stack.item)
}

/// The name on a name tag (`DataComponents.CUSTOM_NAME`).
fn name_of(stack: &ItemStack) -> Option<String> {
    stack.extra.as_ref().and_then(|data| data.custom_name_plain())
}

/// The same tube in another colour (`NixieTubeBlock.withColor`), keeping its text.
fn recolour(host: &mut dyn Host, pos: Pos, dye: &str) {
    let Some((face, facing)) = face_of(host, pos) else {
        return;
    };
    if rules::dye_of_block(&host.block(pos)) == Some(dye) {
        return;
    }
    let saved = host.mod_block_entity(pos).map(|e| e.nbt.clone());
    let block = rules::block_id(dye);
    if !encased::switch_block(host, pos, &block, &[("double_face", face.name()), ("facing", facing.name())]) {
        return;
    }
    if let Some(saved) = saved {
        host.change_mod_block_entity(pos, &mut |e| e.nbt = saved.clone());
    }
}

impl Hooks for Nixie {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        is_tube(event.block).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(STRENGTH.into(), Tag::Int(0));
            (event.block.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_tube(event.block) {
            event.ticks = true;
        }
    }

    /// `NixieTubeBlock.neighborChanged` and `tick`: a tube that shows redstone follows the signal.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_tube(&event.entity.id) || util::game_time(host).rem_euclid(POLL) != 0 {
            return;
        }
        if custom_text(&event.entity.nbt).is_some() {
            return;
        }
        let power = power_at(host, event.pos);
        if util::int(&event.entity.nbt, STRENGTH) != power {
            event.entity.nbt.insert(STRENGTH.into(), Tag::Int(power));
            host.block_entity_sync(event.pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if !is_tube(&event.entity.id) {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(STRENGTH.into(), Tag::Int(util::int(nbt, STRENGTH)));
        // A key that can empty is always sent: a client merges what it is given into what it has.
        let text = custom_text(nbt);
        tag.insert("HasText".into(), Tag::Byte(i8::from(text.is_some())));
        tag.insert(TEXT.into(), Tag::String(text.unwrap_or_default().as_str().into()));
        tag.insert(INDEX.into(), Tag::Int(util::int(nbt, INDEX)));
        Some(tag)
    }

    /// `NixieTubeRenderer.renderSafe`: the two tubes' glyphs, lit.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let Some(dye) = rules::dye_of_block(event.block) else {
            return;
        };
        let face = host.block_state_property(event.state, "double_face").and_then(|f| Face::from_name(&f));
        let facing = host.block_state_property(event.state, "facing").and_then(|f| Dir::from_name(&f));
        let (Some(face), Some(facing)) = (face, facing) else {
            return;
        };
        let nbt = &event.entity.nbt;
        let text = (util::int(nbt, "HasText") != 0).then(|| custom_text(nbt)).flatten();
        let (first, second) = rules::displayed(
            text.as_deref(),
            util::int(nbt, STRENGTH),
            util::int(nbt, INDEX).max(0) as usize,
        );
        let draw = &mut *event.draw;
        let y_rot = horizontal_angle(facing) - 90.0 + if face == Face::WallReversed { 180.0 } else { 0.0 };
        let z_rot = match face {
            Face::Wall => -90.0,
            Face::WallReversed => 90.0,
            _ => 0.0,
        };
        draw.rotate_about([0.0, 1.0, 0.0], y_rot, CENTRE);
        draw.rotate_about([0.0, 0.0, 1.0], z_rot, CENTRE);
        draw.translate(CENTRE);
        let height = if face == Face::Ceiling { 5.0 } else { 3.0 };
        let [x, y, z] = event.pos;
        // `random.nextFloat()` each frame: a flicker that changes every tick.
        let seed = (draw.game_time as u64)
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add((x as u64).wrapping_mul(73_856_093) ^ (y as u64).wrapping_mul(19_349_663) ^ (z as u64).wrapping_mul(83_492_791));
        let flicker = ((seed >> 7) & 0xFF) as f32 / 255.0;
        for (shift, glyph) in [(-4.0 / 16.0, first), (4.0 / 16.0, second)] {
            draw.push();
            draw.translate([shift, 0.0, 0.0]);
            draw.scale([1.0 / 20.0, -1.0 / 20.0, 1.0 / 20.0]);
            draw_tube(host, draw, &glyph, height, dye, flicker);
            draw.pop();
        }
    }

    /// `DoubleFaceAttachedBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn crate::compat::LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != "create:nixie_tube" {
            return None;
        }
        let nearest = rules::nearest_looking(context.yaw, context.pitch, util::dir(context.face), context.replace_clicked);
        let horizontal = create_kinetics::looking(context.yaw, 0.0);
        let (face, facing) = rules::placement(nearest, horizontal, rules::look_vector(context.yaw, context.pitch));
        let state = level.state_with_property(event.state, "double_face", face.name())?;
        level.state_with_property(state, "facing", facing.name()).map(Some)
    }

    /// A tube put down takes the redstone at once.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !is_tube(&host.block(event.pos)) {
            return;
        }
        let power = power_at(host, event.pos);
        if power != 0 {
            host.change_mod_block_entity(event.pos, &mut |e| {
                e.nbt.insert(STRENGTH.into(), Tag::Int(power));
            });
            host.block_entity_sync(event.pos);
        }
    }

    /// `NixieTubeBlock.useItemOn` with an empty hand: forget the text and show redstone again.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if !is_tube(&host.block(pos)) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || !signal::hands_empty(host) {
            return Flow::Continue;
        }
        let shows_text = host.mod_block_entity(pos).is_some_and(|e| custom_text(&e.nbt).is_some());
        if !shows_text {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        clear_text(host, pos);
        let power = power_at(host, pos);
        host.change_mod_block_entity(pos, &mut |e| {
            e.nbt.insert(STRENGTH.into(), Tag::Int(power));
        });
        host.block_entity_sync(pos);
        Flow::Cancel
    }

    /// `NixieTubeBlock.useItemOn`: a name tag writes its name along the row, a dye recolours it;
    /// the Wrench picks a tube up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if !is_tube(&host.block(pos)) {
            return Flow::Continue;
        }
        if &*event.stack.item == util::WRENCH {
            return util::wrench(host, pos, |_| {});
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let name = (&*event.stack.item == NAME_TAG).then(|| name_of(event.stack)).flatten();
        let dye = dye_of(event.stack);
        if name.is_none() && dye.is_none() {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        for (index, at) in row_of(host, pos).into_iter().enumerate() {
            if let Some(name) = &name {
                display_text(host, at, name, index);
            }
            if let Some(dye) = dye {
                recolour(host, at, dye);
            }
        }
        Flow::Cancel
    }
}

/// `NixieTubeRenderer.drawTube`: the glyph in the tube's bright colour, a darker copy behind it,
/// and the mirrored reflection on the back of the glass. Under a pose scaled to 1/20 block a unit.
fn draw_tube(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    glyph: &str,
    height: f32,
    dye: &str,
    flicker: f32,
) {
    if glyph.trim().is_empty() {
        return;
    }
    let (bright, dark, deep) = rules::tube_colors(dye, flicker);
    let shadow = 0.5;
    let width = host.text_width(glyph);
    let text = Text::literal(glyph.to_owned());
    let put = |draw: &mut crate::compat::BlockEntityDraw, colour: u32| {
        host.draw_text(draw, &text, [0.0, 0.0], colour, true, 0);
    };

    draw.push();
    draw.translate([(width - shadow) / -2.0, -height, 0.0]);
    put(draw, bright);
    draw.push();
    draw.translate([shadow, shadow, -1.0 / 16.0]);
    put(draw, dark);
    draw.pop();
    draw.pop();

    draw.push();
    draw.scale([-1.0, 1.0, 1.0]);
    draw.translate([(width - shadow) / -2.0, -height, 0.0]);
    put(draw, dark);
    draw.push();
    draw.translate([-shadow, shadow, -1.0 / 16.0]);
    put(draw, deep);
    draw.pop();
    draw.pop();
}

//! The Display Board (`FlapDisplayBlock`, `FlapDisplayBlockEntity`, `FlapDisplayRenderer`,
//! `DisplayBoardTarget`): a flap display that turns on a rotation of 30 RPM or more, joins
//! boards beside and above each other into one, and shows lines of text on flaps that spin before
//! they settle. A Display Link writes it; a name tag writes one line, a dye colours it, a glow ink
//! sac makes it glow and an empty hand clears it. The rules are
//! `create_kinetics::rules::flap_display`.
//!
//! The board's top-left block is its controller: it keeps the text (`Lines`, in this port's own
//! layout) and draws the lines across the whole board with `Host::draw_text`. The flaps' spin is
//! worked out from the game time the text changed, so every client draws the same.
//!
//! Known differences from Create, for the README:
//! - Joining and the controller are found by looking at the neighbours every ten ticks and when a
//!   block is placed (Create reacts to `updateShape`, `onPlace` and `onRemove`), so a board takes
//!   up to half a second to join or split. The state keeps Create's `up` and `down`.
//! - A board placed against another does not take its facing (`getStateForPlacement`'s helper):
//!   it faces the player. There is no placement helper (`PlacementHelpers`) and no waterlogging.
//! - The text is plain and drawn with the game's font in one colour per line, 85% opaque in Create
//!   and fully opaque here; glowing lines are full bright.
//! - A line of one flap (the fluid unit) spins by dimming. The clatter plays on the server, from the
//!   same spin, at the board's middle.
//! - The sounds `scroll_value` and `calcite hit` play for the whole board, not for each client.

use create_kinetics::rules::flap_display::{
    self as rules, Board, LAZY_TICKS, Layout, Section, accept_flap_text, controller_sizes,
    line_index_at, line_start, max_char_count, spin_after, spin_limit, spin_seed, spinning_glyph,
    update_column,
};
use create_kinetics::rules::display_link::{self as link_rules, Config, Reading, Source, Stats};
use create_kinetics::rules::nixie_tube::{clockwise, dye_colors, dye_of_item, look_vector};
use create_kinetics::rules::steam_engine::horizontal_angle;
use create_kinetics::{Axis, Dir, Kind, Node, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Flow, Hooks, Host, ItemStack,
    RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::{kinetic, util};

pub const ID: &str = "create:display_board";
const COG: &str = "create:block/cogwheel_shaftless";
const CENTRE: [f32; 3] = [0.5; 3];
const GLOW_INK_SAC: &str = "minecraft:glow_ink_sac";
const NAME_TAG: &str = "minecraft:name_tag";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

const CONTROLLER: &str = "Controller";
const X_SIZE: &str = "XSize";
const Y_SIZE: &str = "YSize";
const LINES: &str = "Lines";

struct Flaps;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Flaps)
}

// --- saving a board ------------------------------------------------------------------------------

fn text_of(tag: &Compound, key: &str) -> Option<String> {
    match util::get(tag, key) {
        Some(Tag::String(text)) => Some(text.to_string_lossy()),
        _ => None,
    }
}

fn section_tag(section: &Section) -> Tag {
    let mut tag = Compound::new();
    util::set_float(&mut tag, "Width", section.size);
    tag.insert("Cycle".into(), Tag::String(section.cycle.as_str().into()));
    for (key, value) in [
        ("Single", section.single_flap),
        ("Gap", section.gap),
        ("Right", section.right_aligned),
        ("Wide", section.wide),
    ] {
        tag.insert(key.into(), Tag::Byte(i8::from(value)));
    }
    tag.insert("Text".into(), Tag::String(section.text.as_str().into()));
    tag.insert("Prev".into(), Tag::String(section.prev.as_str().into()));
    tag.insert("Since".into(), Tag::Long(section.since.unwrap_or(i64::MIN)));
    Tag::Compound(tag)
}

fn section_of(tag: &Tag) -> Option<Section> {
    let Tag::Compound(tag) = tag else {
        return None;
    };
    let mut section = Section::new(
        util::float(tag, "Width")?,
        &text_of(tag, "Cycle")?,
        util::boolean(tag, "Single"),
        util::boolean(tag, "Gap"),
    );
    section.right_aligned = util::boolean(tag, "Right");
    section.wide = util::boolean(tag, "Wide");
    section.text = text_of(tag, "Text")?;
    section.prev = text_of(tag, "Prev")?;
    let since = util::long(tag, "Since");
    section.since = (since != i64::MIN).then_some(since);
    Some(section)
}

/// The controller's board from its saved data, `None` for a block that is not a controller.
pub fn read_board(nbt: &Compound) -> Option<Board> {
    if !util::boolean(nbt, CONTROLLER) {
        return None;
    }
    let (x, y) = (util::int(nbt, X_SIZE), util::int(nbt, Y_SIZE));
    let mut board = Board::new(x, y);
    let Some(Tag::List(lines)) = util::get(nbt, LINES) else {
        return Some(board);
    };
    for (i, line) in lines.iter().enumerate().take(board.lines.len()) {
        let Tag::Compound(line) = line else {
            continue;
        };
        if let Some(Tag::List(sections)) = util::get(line, "Sections") {
            let sections: Vec<Section> = sections.iter().filter_map(section_of).collect();
            if !sections.is_empty() {
                board.lines[i] = Layout { key: text_of(line, "Key").unwrap_or_default(), sections };
            }
        }
        board.colour[i] = text_of(line, "Dye").filter(|d| !d.is_empty());
        board.glowing[i] = util::boolean(line, "Glow");
        board.manual[i] = util::boolean(line, "Manual");
        if let Some(Tag::IntArray(at)) = util::get(line, "Reserved")
            && at.len() == 3
        {
            board.reserved[i] = Some([at[0], at[1], at[2]]);
        }
    }
    Some(board)
}

pub fn write_board(nbt: &mut Compound, board: &Board) {
    nbt.insert(CONTROLLER.into(), Tag::Byte(1));
    nbt.insert(X_SIZE.into(), Tag::Int(board.x_size));
    nbt.insert(Y_SIZE.into(), Tag::Int(board.y_size));
    let lines = board
        .lines
        .iter()
        .enumerate()
        .map(|(i, layout)| {
            let mut line = Compound::new();
            line.insert("Key".into(), Tag::String(layout.key.as_str().into()));
            line.insert("Sections".into(), Tag::List(layout.sections.iter().map(section_tag).collect()));
            line.insert("Dye".into(), Tag::String(board.colour[i].clone().unwrap_or_default().as_str().into()));
            line.insert("Glow".into(), Tag::Byte(i8::from(board.glowing[i])));
            line.insert("Manual".into(), Tag::Byte(i8::from(board.manual[i])));
            let reserved = board.reserved[i].map(|p| p.to_vec()).unwrap_or_default();
            line.insert("Reserved".into(), Tag::IntArray(reserved));
            Tag::Compound(line)
        })
        .collect();
    nbt.insert(LINES.into(), Tag::List(lines));
}

fn write_not_controller(nbt: &mut Compound) {
    nbt.insert(CONTROLLER.into(), Tag::Byte(0));
    nbt.insert(X_SIZE.into(), Tag::Int(1));
    nbt.insert(Y_SIZE.into(), Tag::Int(1));
    nbt.insert(LINES.into(), Tag::List(Vec::new()));
}

// --- the blocks ----------------------------------------------------------------------------------

fn step(pos: Pos, dir: Dir, n: i32) -> Pos {
    let v = dir.normal();
    [pos[0] + v[0] * n, pos[1] + v[1] * n, pos[2] + v[2] * n]
}

/// A board block's state: its facing and joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct State {
    facing: Dir,
    up: bool,
    down: bool,
}

fn state_at(host: &dyn Host, pos: Pos) -> Option<State> {
    if host.block(pos) != ID {
        return None;
    }
    let flag = |key| host.block_property(pos, key).as_deref() == Some("true");
    Some(State { facing: util::facing(host, pos, "facing")?, up: flag("up"), down: flag("down") })
}

fn facing_of(host: &dyn Host, pos: Pos) -> Option<Dir> {
    (host.block(pos) == ID).then(|| util::facing(host, pos, "facing")).flatten()
}

/// `FlapDisplayBlock.updateColumn` for the board at `pos`.
fn column(host: &dyn Host, pos: Pos, facing: Dir) -> (bool, bool) {
    let along = clockwise(facing);
    update_column(|offset, dy| {
        let at = step(step(pos, along, offset), Dir::Up, dy);
        (offset == 0 && dy == 0) || facing_of(host, at) == Some(facing)
    })
}

/// `FlapDisplayBlockEntity.getController`: the controller block of the board the block at `pos`
/// belongs to.
pub fn controller_of(host: &dyn Host, pos: Pos) -> Option<Pos> {
    let facing = facing_of(host, pos)?;
    let side = clockwise(facing);
    let mut at = pos;
    for _ in 0..64 {
        if state_at(host, at).is_some_and(|s| s.up) {
            at = step(at, Dir::Up, 1);
            continue;
        }
        // The neighbour on the left is a board that doesn't hang from one above.
        if state_at(host, step(at, side, 1)).is_some_and(|s| !s.up) {
            at = step(at, side, 1);
            continue;
        }
        let entity = host.mod_block_entity(at)?;
        return util::boolean(&entity.nbt, CONTROLLER).then_some(at);
    }
    None
}

/// `DisplayBoardTarget.provideStats`.
pub fn stats(host: &dyn Host, target: Pos) -> Stats {
    let board = controller_of(host, target)
        .and_then(|c| host.mod_block_entity(c))
        .and_then(|entity| read_board(&entity.nbt));
    match board {
        Some(b) => Stats { rows: (b.y_size * 2) as usize, columns: b.max_char_count(0).max(0) as usize },
        None => Stats { rows: 1, columns: 1 },
    }
}

/// `DisplayBoardTarget.acceptFlapText`: writes what a Display Link read to the board the block at
/// `target` is part of. Nothing happens while the board turns too slowly.
#[allow(clippy::too_many_arguments)]
pub fn deliver(
    host: &mut dyn Host,
    link: Pos,
    target: Pos,
    line: usize,
    source: Source,
    config: &mut Config,
    reading: &Reading,
) -> bool {
    let Some(controller) = controller_of(&*host, target) else {
        return false;
    };
    let Some(entity) = host.mod_block_entity(controller) else {
        return false;
    };
    if !rules::speed_fulfilled(kinetic::speed(&entity)) {
        return false;
    }
    let Some(mut board) = read_board(&entity.nbt) else {
        return false;
    };
    let stats = Stats { rows: (board.y_size * 2) as usize, columns: board.max_char_count(0).max(0) as usize };
    let rows = link_rules::provide_flap_text(source, config, reading, &stats);
    let x_size = board.x_size;
    let now = util::game_time(&*host);
    let exists = |at: Pos| host.block(at) == "create:display_link";
    let cfg = config.clone();
    accept_flap_text(
        &mut board,
        line,
        &rows,
        source.is_single_line(),
        link,
        &exists,
        now,
        &|layout, _| link_rules::load_flap_layout(source, &cfg, reading, layout, x_size, &|gaps| max_char_count(x_size, gaps)),
    );
    host.change_mod_block_entity(controller, &mut |e| write_board(&mut e.nbt, &board));
    host.block_entity_sync(controller);
    true
}

/// A click on a board: which of its lines the player's look meets.
fn line_hit(host: &dyn Host, controller: Pos, pos: Pos, face: [i32; 3], board: &Board) -> Option<usize> {
    let player = host.acting_player()?;
    let eye_height = if player.sneaking { 1.27 } else { 1.62 };
    let eye = [player.position[0], player.position[1] + eye_height, player.position[2]];
    let hit = link_rules::hit_on_face(eye, look_vector(player.yaw, player.pitch), pos, face);
    let y = hit[1] - f64::from(face[1]) * 0.125;
    let index = line_index_at(controller[1], y, board.y_size);
    (index < board.lines.len()).then_some(index)
}

/// What a click with `stack` (or `None`, an empty hand) does to the line `index`.
fn edit_line(host: &mut dyn Host, controller: Pos, index: usize, stack: Option<&ItemStack>) -> bool {
    let Some(entity) = host.mod_block_entity(controller) else {
        return false;
    };
    let Some(mut board) = read_board(&entity.nbt) else {
        return false;
    };
    let running = rules::speed_fulfilled(kinetic::speed(&entity));
    let now = util::game_time(&*host);
    match stack {
        None => {
            if !running {
                return false;
            }
            board.apply_text_manually(index, None, now);
        }
        Some(stack) if &*stack.item == GLOW_INK_SAC => {
            board.glowing[index] = true;
            let at = util::centre(controller);
            host.play_sound("minecraft:item.ink_sac.use", at, BLOCKS, 1.0, 1.0);
        }
        Some(stack) => {
            let name = (&*stack.item == NAME_TAG)
                .then(|| stack.extra.as_ref().and_then(|d| d.custom_name_plain()))
                .flatten();
            let dye = dye_of_item(&stack.item);
            if name.is_none() && dye.is_none() {
                return false;
            }
            if dye.is_none() && !running {
                return false;
            }
            if let Some(name) = &name {
                board.apply_text_manually(index, Some(name), now);
            }
            if let Some(dye) = dye {
                host.play_sound("minecraft:item.dye.use", util::centre(controller), BLOCKS, 1.0, 1.0);
                board.colour[index] = (dye != "white").then(|| dye.to_owned());
            }
        }
    }
    host.change_mod_block_entity(controller, &mut |e| write_board(&mut e.nbt, &board));
    host.block_entity_sync(controller);
    true
}

/// `FlapDisplayBlockEntity.updateControllerStatus`, and the joins `FlapDisplayBlock.tick` keeps.
fn update_structure(host: &mut dyn Host, pos: Pos, entity: &mut crate::compat::ModBlockEntity) -> bool {
    let Some(state) = state_at(&*host, pos) else {
        return false;
    };
    let (up, down) = column(&*host, pos, state.facing);
    if up != state.up {
        host.set_block_property(pos, "up", if up { "true" } else { "false" });
    }
    if down != state.down {
        host.set_block_property(pos, "down", if down { "true" } else { "false" });
    }
    let state = State { up, down, ..state };
    let left = clockwise(state.facing);
    let right = left.opposite();
    let same = |at: Pos| state_at(&*host, at) == Some(state);
    let sizes = controller_sizes(
        state.up,
        same(step(pos, left, 1)),
        |n| same(step(pos, right, n)),
        |n| state_at(&*host, step(pos, Dir::Down, n)).is_some_and(|s| s.down),
    );
    let known = util::get(&entity.nbt, CONTROLLER).is_some();
    let old = read_board(&entity.nbt);
    let unchanged = known
        && match &old {
            Some(b) => sizes.controller && b.x_size == sizes.x_size && b.y_size == sizes.y_size,
            None => !sizes.controller,
        };
    if unchanged {
        return false;
    }
    if sizes.controller {
        // A board that grows or shrinks starts its lines again, keeping the dyes and the glow.
        let mut board = Board::new(sizes.x_size, sizes.y_size);
        if let Some(old) = old {
            for i in 0..board.lines.len().min(old.lines.len()) {
                board.colour[i].clone_from(&old.colour[i]);
                board.glowing[i] = old.glowing[i];
            }
        }
        write_board(&mut entity.nbt, &board);
    } else {
        write_not_controller(&mut entity.nbt);
    }
    true
}

/// Which way a board of `x_size` blocks reaches from its controller (`getDirection().getClockWise()`).
fn reach(facing: Dir) -> Dir {
    clockwise(facing).opposite()
}

// --- the hooks -----------------------------------------------------------------------------------

impl Hooks for Flaps {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(COG.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `FlapDisplayBlockEntity.createRenderBoundingBox`: the block widened along the row and down.
    fn block_entity_render_bounding_box(&self, host: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(board) = read_board(&event.entity.nbt) else {
            return;
        };
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let n = reach(facing).normal();
        let (x, z) = (board.x_size as f32 * n[0] as f32, board.x_size as f32 * n[2] as f32);
        *event.bounds = [x.min(0.0), -(board.y_size as f32), z.min(0.0), 1.0 + x.max(0.0), 1.0, 1.0 + z.max(0.0)];
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some(facing) = util::facing(host, pos, "facing") else {
            return;
        };
        // `ICogWheel` with no impact: a small cogwheel along the facing.
        let Some((rpm, mut changed)) = kinetic::tick(
            host,
            pos,
            event.entity,
            Node::passive(Kind::Cogwheel(facing.axis())),
        ) else {
            return;
        };
        let now = util::game_time(host);
        let unknown = util::get(&event.entity.nbt, CONTROLLER).is_none();
        let phase = (pos[0].rem_euclid(5) + pos[1].rem_euclid(3) * 2 + pos[2].rem_euclid(7)) as i64;
        if unknown || (now + phase).rem_euclid(LAZY_TICKS) == 0 {
            changed |= update_structure(host, pos, event.entity);
        }
        if changed {
            host.block_entity_sync(pos);
        }
        // The clatter of the flaps that still spin, once for the whole board, from its controller.
        if let Some(board) = read_board(&event.entity.nbt)
            && rules::speed_fulfilled(rpm)
        {
            clatter(host, pos, facing, &board, rpm, now);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let mut tag = kinetic::update_tag(event.entity);
        for key in [CONTROLLER, X_SIZE, Y_SIZE, LINES] {
            if let Some(value) = util::get(&event.entity.nbt, key) {
                tag.insert(key.into(), value.clone());
            }
        }
        if util::get(&event.entity.nbt, LINES).is_none() {
            tag.insert(LINES.into(), Tag::List(Vec::new()));
        }
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let axis = facing.axis();
        let rpm = kinetic::speed(event.entity);
        let pos = event.pos;
        let draw = &mut *event.draw;
        // `KineticBlockEntityRenderer`: the shaftless cogwheel, turned half a tooth where its
        // neighbours' teeth would meet.
        let in_plane: i32 = (0..3).filter(|&i| i != axis.index()).map(|i| pos[i]).sum();
        let phase = if in_plane.rem_euclid(2) == 0 { 22.5 } else { 0.0 };
        draw.spin(axis.unit(), CENTRE, rpm * 0.3, phase);
        draw.push();
        match axis {
            Axis::X => draw.rotate_about(Axis::Z.unit(), -90.0, CENTRE),
            Axis::Z => draw.rotate_about(Axis::X.unit(), 90.0, CENTRE),
            Axis::Y => {}
        }
        host.draw_model(draw, COG, 0xFF_FFFF);
        draw.pop();
        draw.still();

        let Some(board) = read_board(&event.entity.nbt) else {
            draw.keep();
            return;
        };
        let running = rules::speed_fulfilled(rpm);
        let animating = draw_lines(host, draw, pos, facing, &board, rpm, running);
        // Text that has settled is the same every frame.
        if !animating {
            draw.keep();
        }
    }

    /// An empty hand clears the line it clicks (`FlapDisplayBlock.useItemOn`).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking || player.game_mode == util::SPECTATOR || !signal::hands_empty(host) {
            return Flow::Continue;
        }
        let Some(controller) = controller_of(&*host, pos) else {
            return Flow::Continue;
        };
        let Some(board) = host.mod_block_entity(controller).and_then(|e| read_board(&e.nbt)) else {
            return Flow::Continue;
        };
        let Some(index) = line_hit(&*host, controller, pos, event.face, &board) else {
            return Flow::Continue;
        };
        let running = host.mod_block_entity(controller).is_some_and(|e| rules::speed_fulfilled(kinetic::speed(&e)));
        if !running {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        edit_line(host, controller, index, None);
        Flow::Cancel
    }

    /// A name tag, a dye or a glow ink sac on a line; the Wrench picks the block up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
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
        let item = &*event.stack.item;
        if item != GLOW_INK_SAC && item != NAME_TAG && dye_of_item(item).is_none() {
            return Flow::Continue;
        }
        let Some(controller) = controller_of(&*host, pos) else {
            return Flow::Continue;
        };
        let Some(board) = host.mod_block_entity(controller).and_then(|e| read_board(&e.nbt)) else {
            return Flow::Continue;
        };
        let Some(index) = line_hit(&*host, controller, pos, event.face, &board) else {
            return Flow::Continue;
        };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if edit_line(host, controller, index, Some(event.stack)) { Flow::Cancel } else { Flow::Continue }
    }

    fn state_for_placement(
        &self,
        level: &dyn crate::compat::LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<crate::compat::BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        // `HorizontalKineticBlock.getStateForPlacement`: facing the player.
        let facing = create_kinetics::looking(context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }
}

// --- drawing -------------------------------------------------------------------------------------

/// The clatter (`FlapDisplayBlockEntity.tick`): scroll and calcite sounds, louder for more
/// flaps still turning, while any section of the board is in its spin.
fn clatter(host: &mut dyn Host, pos: Pos, facing: Dir, board: &Board, rpm: f32, now: i64) {
    let instant = rpm.abs() > 128.0;
    let mut flaps = 0usize;
    for (line_index, line) in board.lines.iter().enumerate() {
        for (section_index, section) in line.sections.iter().enumerate() {
            let Some(since) = section.since else {
                continue;
            };
            let ticks = (now - since) as i32;
            let options = section.options().len();
            if ticks < 0 || ticks > spin_limit(options) + 1 {
                continue;
            }
            let seed = spin_seed(pos, line_index, section_index, since);
            flaps += spin_after(&section.changed(), options, ticks, instant, seed).2;
        }
    }
    if flaps == 0 {
        return;
    }
    let volume = (flaps as f32 / 20.0).clamp(0.25, 1.5);
    let background = (flaps as f32 / 40.0).clamp(0.25, 1.0);
    let middle = step(step(pos, reach(facing), board.x_size / 2), Dir::Down, board.y_size / 2);
    let at = util::centre(middle);
    host.play_sound("minecraft:block.note_block.hat", at, BLOCKS, volume, 0.56);
    host.play_sound("minecraft:block.calcite.hit", at, BLOCKS, 0.35 * background, 1.95);
}

/// `FlapDisplayRenderer.renderSafe` for the controller: every line's sections across the board.
/// True while any flap still spins (the frame then reads the game time).
fn draw_lines(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    pos: Pos,
    facing: Dir,
    board: &Board,
    rpm: f32,
    running: bool,
) -> bool {
    let mut animating = false;
    let instant = rpm.abs() > 128.0;
    let now = draw.game_time;
    let time = if running { now as f32 + draw.partial } else { 0.0 };
    draw.push();
    draw.rotate_about([0.0, 1.0, 0.0], horizontal_angle(facing), CENTRE);
    draw.translate([0.0, 1.0, 1.0 - 3.0 / 16.0]);
    draw.scale([1.0 / 32.0, -1.0 / 32.0, 1.0 / 32.0]);
    draw.translate([0.0, 0.0, 0.5]);
    for (line_index, line) in board.lines.iter().enumerate() {
        let colour = board.colour[line_index].as_deref().map_or(rules::DEFAULT_COLOUR, |d| dye_colors(d).0);
        let glowing = board.glowing[line_index];
        draw.push();
        draw.translate([line_start(board.x_size, line.width()), 4.5 + 16.0 * line_index as f32, 0.0]);
        for (section_index, section) in line.sections.iter().enumerate() {
            let options = section.options();
            let (spinning, spinning_ticks) = match section.since {
                Some(since) if running => {
                    let ticks = (now - since).max(0) as i32;
                    if ticks <= spin_limit(options.len()) + 1 {
                        animating = true;
                    }
                    let seed = spin_seed(pos, line_index, section_index, since);
                    let (s, t, _) = spin_after(&section.changed(), options.len(), ticks, instant, seed);
                    (s, t)
                }
                // A board at rest holds its flaps where the spin began.
                Some(_) => (section.changed(), 1),
                None => (vec![false; section.flaps()], 0),
            };
            let standard = if section.wide { rules::WIDE_MONOSPACE } else { rules::MONOSPACE };
            let mut x = 0.0f32;
            if section.single_flap {
                let dim = spinning.first().copied().unwrap_or(false);
                let shade = if dim { 0.75 } else { 1.0 };
                host.draw_text(draw, &Text::literal(section.text.clone()), [x, 0.0], dimmed(colour, shade), glowing, 0);
            } else {
                for (i, ch) in section.text.chars().enumerate() {
                    let spins = spinning.get(i).copied().unwrap_or(false);
                    let shown = if spins {
                        spinning_glyph(&options, spinning_ticks, time, i, line_index, !running)
                    } else {
                        ch
                    };
                    let glyph = shown.to_string();
                    let width = host.text_width(&glyph);
                    if shown != ' ' {
                        let shade = if spins { 0.75 } else { 1.0 };
                        let at = x + (standard - width) / 2.0;
                        host.draw_text(draw, &Text::literal(glyph), [at, 0.0], dimmed(colour, shade), glowing, 0);
                    }
                    x += standard;
                }
            }
            draw.translate([section.advance(), 0.0, 0.0]);
        }
        draw.pop();
    }
    draw.pop();
    animating
}

/// `colour` with each channel times `shade` (a spinning flap is dimmed to 75%).
fn dimmed(colour: u32, shade: f32) -> u32 {
    let channel = |shift: u32| (((colour >> shift) & 0xFF) as f32 * shade) as u32;
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

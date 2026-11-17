//! The client's half of using a value box: holding the right button on a box, and the value board that opens
//! (`ValueSettingsClient`, `ValueSettingsScreen`, `ValueSettingsPacket`). The box looked at, what a held click
//! decides and the layout of the board are rules (`create_kinetics::rules::value_box*`); this reads the buttons and
//! the pointer, draws the board on a mod screen of its own (`Host::mod_screen_open`) and sends the answer to the
//! server (`create:value_box`).
//!
//! Known differences from Create, for the README (the board's look is `rules::value_box_board`'s note):
//! - Horizon can't move the pointer, so the board doesn't open with the pointer on the box's current value: the
//!   current value is marked on the bar and stays selected until the pointer moves (or the wheel is turned); then
//!   the value under the pointer is the one set.
//! - A synthetic click from the agent tools doesn't reach the `key` hook that starts the hold, so a box with a board
//!   can only be tried with a real mouse.

use std::sync::{Mutex, PoisonError};

use create_kinetics::rules::value_box_blocks::{BoxDef, Kind};
use create_kinetics::rules::value_box_board::{self as board, Behaviour, Layout, Settings};
use create_kinetics::rules::value_box_input::{self as input, Held, Hold, Message};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    ClientTickPostEvent, Flow, Hooks, Host, KeyEvent, NativeScreenKind, ScreenClosingEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseScrolledPreEvent, ScreenRenderPostEvent,
};

use super::value_box_draw::{looked_at, shown, Shown};

/// The mod's own screens belong to mod `create` (Ponder's too: only one is open at a time).
const MOD: &str = "create";
/// `GLFW_MOUSE_BUTTON_RIGHT`.
const RIGHT: i32 = 1;

struct Screen;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Screen)
}

/// The box a hold or an open board is about.
#[derive(Clone)]
struct Target {
    pos: Pos,
    side: Dir,
    def: &'static BoxDef,
}

impl Target {
    fn side_id(&self) -> u8 {
        Dir::ALL.iter().position(|d| *d == self.side).unwrap_or(1) as u8
    }

    fn same_as(&self, other: &Shown, pos: Pos, side: Dir) -> bool {
        self.pos == pos && self.side == side && std::ptr::eq(self.def, other.def)
    }
}

/// The open value board.
struct Open {
    target: Target,
    behaviour: Behaviour,
    max_stack: i32,
    title: String,
    layout: Option<Layout>,
    /// What the box holds now.
    current: Settings,
    /// What would be set if the button were let go.
    selected: Settings,
    /// Whether the pointer chooses (it moved since the board opened or the wheel last turned).
    pointer_drives: bool,
    last_mouse: Option<[f32; 2]>,
    /// The right button was let go: the board closes on the next tick.
    closing: bool,
}

#[derive(Default)]
struct Client {
    right_down: bool,
    hold: Option<(Hold, Target)>,
    open: Option<Open>,
    outbox: Vec<Message>,
}

static CLIENT: Mutex<Client> = Mutex::new(Client {
    right_down: false,
    hold: None,
    open: None,
    outbox: Vec::new(),
});

fn client() -> std::sync::MutexGuard<'static, Client> {
    CLIENT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn say(host: &dyn Host, key: &str) -> String {
    super::goggles::lang(host).translate(key, &[])
}

/// The box with a board that the crosshair is on, if any (`ValueSettingsInputHandler`'s first hit that takes a value).
fn board_target(host: &dyn Host) -> Option<(Target, Shown)> {
    let looked = looked_at(host)?;
    let item = shown(host, &looked).into_iter().find(|s| s.hovered)?;
    let takes = match item.def.kind {
        Kind::Value { .. } => true,
        Kind::Filter { .. } => item.counts,
    };
    takes.then(|| (Target { pos: looked.pos, side: looked.hit.side, def: item.def }, item))
}

fn playing(host: &dyn Host) -> bool {
    host.client_state().is_some_and(|client| client.screen == NativeScreenKind::Playing)
}

impl Hooks for Screen {
    /// `ValueSettingsInputHandler` + `startInteractionWith`: a right click on a box with a board starts the hold.
    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        if !host.window() || event.key != "key.mouse.right" {
            return;
        }
        let mut c = client();
        c.right_down = event.pressed;
        if !event.pressed || c.hold.is_some() || !playing(&*host) || host.sneaking() {
            return;
        }
        drop(c);
        if let Some((target, _)) = board_target(&*host) {
            client().hold = Some((Hold::start(), target));
        }
    }

    /// `ValueSettingsClient.tick`: a short click, or the board after six ticks.
    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        if !host.window() {
            return;
        }
        // What the board's hooks, which have no `&mut Host`, asked for.
        let (messages, close) = {
            let mut c = client();
            let close = c.open.as_ref().is_some_and(|open| open.closing);
            (std::mem::take(&mut c.outbox), close)
        };
        for message in messages {
            let _ = host.net_send_to_server(input::CHANNEL, &message.encode());
        }
        if close && host.mod_screen() == Some(MOD) {
            host.mod_screen_close(MOD);
        }
        let Some((mut hold, target)) = client().hold.take() else {
            return;
        };
        let down = client().right_down;
        let now = board_target(&*host);
        let valid = !host.sneaking()
            && now.as_ref().is_some_and(|(t, item)| target.same_as(item, t.pos, t.side));
        match hold.tick(valid, down) {
            Held::Waiting => client().hold = Some((hold, target)),
            Held::Cancelled => {}
            Held::Short => {
                let message = Message::Short { pos: target.pos, net_id: target.def.net_id, side: target.side_id() };
                let _ = host.net_send_to_server(input::CHANNEL, &message.encode());
            }
            Held::Open => {
                let Some((_, item)) = now else {
                    return;
                };
                let Some(behaviour) = target.def.behaviour() else {
                    return;
                };
                let current = board::settings_of(behaviour, &item.values);
                let title = match target.def.kind {
                    Kind::Filter { .. } => say(&*host, "create.logistics.filter.extracted_amount"),
                    Kind::Value { .. } => say(&*host, target.def.label()),
                };
                if host.mod_screen_open(MOD) {
                    client().open = Some(Open {
                        target,
                        behaviour,
                        max_stack: item.values.max_stack,
                        title,
                        layout: None,
                        current,
                        selected: current,
                        pointer_drives: false,
                        last_mouse: None,
                        closing: false,
                    });
                }
            }
        }
    }

    /// `ValueSettingsScreen.renderWindow`.
    fn screen_render_post(&self, host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>) {
        if event.screen.id != "mod" || host.mod_screen() != Some(MOD) {
            return;
        }
        let mut c = client();
        let Some(open) = c.open.as_mut() else {
            return;
        };
        let (w, h) = (event.batch.width, event.batch.height);
        let words = |key: &str| say(host, key);
        let board_spec = board::board(open.behaviour, open.max_stack, &words);
        let label_width = board_spec.rows.iter().map(|row| host.text_width(row) as i32).max().unwrap_or(0);
        if open.layout.as_ref().is_none_or(|l| l.board != board_spec) {
            open.layout = Some(Layout::new(board_spec, label_width, [w as i32, h as i32]));
        }
        let layout = open.layout.clone().expect("just set");
        let mouse = event.screen.mouse;
        // The pointer picks the value once it moves; until then the box's own value stays.
        match open.last_mouse {
            None => open.last_mouse = Some(mouse),
            Some(last) if (mouse[0] - last[0]).abs() + (mouse[1] - last[1]).abs() > 1.0 => {
                open.pointer_drives = true;
                open.last_mouse = Some(mouse);
            }
            Some(_) => {}
        }
        if open.pointer_drives {
            open.selected = layout.closest(mouse, host.has_shift_down());
        }
        draw(host, event, open, &layout);
    }

    /// `ValueSettingsScreen.mouseReleased`: letting go of the right button sets the value under the pointer.
    fn screen_mouse_button_released_pre(&self, host: &dyn Host, event: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || host.mod_screen() != Some(MOD) || event.button != RIGHT {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.open.as_mut() else {
            return Flow::Continue;
        };
        if open.closing {
            return Flow::Cancel;
        }
        if let Some(layout) = &open.layout
            && open.pointer_drives
        {
            open.selected = layout.closest(event.screen.mouse, host.has_shift_down());
        }
        let message = Message::Set {
            pos: open.target.pos,
            net_id: open.target.def.net_id,
            side: open.target.side_id(),
            row: open.selected.row,
            value: open.selected.value,
        };
        open.closing = true;
        c.outbox.push(message);
        Flow::Cancel
    }

    /// `ValueSettingsScreen.mouseScrolled`: the wheel steps the value, Shift by the marked ones.
    fn screen_mouse_scrolled_pre(&self, host: &dyn Host, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || host.mod_screen() != Some(MOD) {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.open.as_mut() else {
            return Flow::Continue;
        };
        if let Some(layout) = &open.layout {
            open.selected = layout.scrolled(open.selected, event.scroll[1], host.has_shift_down());
            open.pointer_drives = false;
            open.last_mouse = Some(event.screen.mouse);
        }
        Flow::Cancel
    }

    fn screen_closing(&self, _: &dyn Host, event: &mut ScreenClosingEvent<'_>) {
        if event.screen.id == "mod" {
            client().open = None;
        }
    }
}

// --- drawing the board ----------------------------------------------------------------------------------------

const DIM: u32 = 0x50_101010;
const PANEL: u32 = 0xF0_2B2118;
const FRAME: u32 = 0xFF_B8924A;
const BAR_BG: u32 = 0xFF_3A2C1E;
const BAR: u32 = 0xFF_8A6B3C;
const MILESTONE: u32 = 0xFF_E8D08A;
const LABEL_BG: u32 = 0xFF_D9B96A;
const CURSOR: u32 = 0xFF_F0D890;
const INK: u32 = 0x44_2000;

fn rect(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    host.fill(event.batch, [x, y, w, h], argb);
}

/// `ValueSettingsScreen.renderWindow` in flat rectangles.
fn draw(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, open: &Open, layout: &Layout) {
    let words = |key: &str| say(host, key);
    let (w, h) = (event.batch.width, event.batch.height);
    rect(host, event, 0.0, 0.0, w, h, DIM);

    let b = &layout.board;
    let (x, y) = (layout.gui_left as f32, layout.gui_top as f32);
    let icon = b.icon_mode;
    let tip = say(host, "create.gui.value_settings.release_to_confirm").replace("%1$s", "Right Button");
    let additional = if icon { 46.0 } else { 33.0 };
    let mut fattest = host.text_width(&tip).max(host.text_width(&open.title));
    if icon {
        for value in 0..=b.max_value {
            let text = board::format(open.behaviour, open.max_stack, Settings { row: 0, value }, &words);
            fattest = fattest.max(host.text_width(&text));
        }
    }
    let bg_width = (layout.window_width as f32 + 13.0).max(fattest + 10.0);
    let fat_offset = ((fattest + 10.0 - (layout.window_width as f32 + 13.0)).max(0.0)) / 2.0;
    let left = x - 11.0 - fat_offset;
    rect(host, event, left, y - 17.0, bg_width, layout.window_height as f32 + additional, PANEL);
    let centre = left + bg_width / 2.0;
    host.text(event.batch, &open.title, [centre - host.text_width(&open.title) / 2.0, y - 14.0], 0xDD_DDDD, false);
    host.text(event.batch, &tip, [centre - host.text_width(&tip) / 2.0, y + layout.window_height as f32 + additional - 27.0], 0xDD_DDDD, false);

    let rows = b.rows.len() as f32;
    let bar_x = x + layout.max_label_width as f32 + 14.0;
    rect(host, event, bar_x, y - 3.0, layout.value_bar_width as f32 + 8.0, rows * 11.0 + 5.0, FRAME);
    rect(host, event, bar_x + 3.0, y, layout.value_bar_width as f32 + 2.0, rows * 11.0 - 1.0, BAR_BG);

    let milestones = b.max_value / b.milestone_interval + 1;
    let first = bar_x + 4.0;
    for (i, label) in b.rows.iter().enumerate() {
        let row_y = y + i as f32 * 11.0;
        if !icon {
            rect(host, event, x - 4.0, row_y, layout.max_label_width as f32 + 8.0, 11.0, LABEL_BG);
            rect(host, event, first, row_y + 1.0, layout.value_bar_width as f32, 8.0, BAR);
            host.text(event.batch, label, [x, row_y + 1.0], INK, false);
        }
        let mut at = first;
        for _ in 0..milestones {
            rect(host, event, at, row_y + 1.0, layout.milestone_size as f32, 8.0, MILESTONE);
            at += (layout.milestone_size + b.milestone_interval * layout.scale) as f32;
        }
        // The value the box holds now.
        if i as i32 == open.current.row {
            let [cx, _] = layout.coordinate_of(open.current.row, open.current.value);
            rect(host, event, cx - 0.5, row_y + 1.0, 1.0, 8.0, 0xFF_FFFFFF);
        }
    }

    let [cx, cy] = layout.coordinate_of(open.selected.row, open.selected.value);
    let text = board::format(open.behaviour, open.max_stack, open.selected, &words);
    if icon {
        rect(host, event, cx - 4.0, cy - 4.0, 8.0, 8.0, CURSOR);
        host.text(
            event.batch,
            &text,
            [left + (bg_width - host.text_width(&text)) / 2.0, y + layout.window_height as f32 + additional - 40.0],
            0xFB_DC7D,
            false,
        );
        return;
    }
    let width = (host.text_width(&text) / 2.0).floor() * 2.0 + 3.0;
    let (cursor_x, cursor_y) = (cx.floor() - (width / 2.0).floor(), cy.floor() - 7.0);
    rect(host, event, cursor_x - 3.0, cursor_y, width + 6.0, 14.0, CURSOR);
    host.text(event.batch, &text, [cursor_x + 2.0, cursor_y + 3.0], INK, false);
}

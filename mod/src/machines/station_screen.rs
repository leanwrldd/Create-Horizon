//! The client's half of the Train Station's screens (`StationScreen`, `AssemblyScreen`, `AbstractStationScreen`): the window the
//! station opens when it is right-clicked, drawn on a mod screen of its own (`Host::mod_screen_open`), and what it says back to the
//! server (`create:station_edit`: `StationEditPacket`, `TrainEditPacket.Serverbound`). Everything it decides (the buttons, the
//! train sliding in and out, the tooltips, the text fields, the messages) is `create_kinetics::rules::trains::station_screen`; the
//! server's half (the station's state sent while the screen is open, applying the edits) is `track_station.rs`.
//!
//! Known differences from Create, for the README (the rest are in the rules' header):
//! - The window is flat rectangles and text, not Create's textures; the train's icon is plain blocks of the icon's width, and the
//!   station's 3D model and flag at the window's corner (`renderAdditional`) aren't drawn.
//! - Buttons carry a short word where Create has an icon (the tooltip has the full name).
//! - `StationBlockEntity.computerBehaviour` (the ComputerCraft screen) isn't ported.

use std::sync::{Mutex, PoisonError};

use create_kinetics::rules::trains::station_screen::{
    self as rules, assembly_layout, station_layout, AssemblyScreen, AssemblyTick, Message, NameBox, StationScreen, StationView, TextField,
};
use crate::compat::{
    ClientTickPostEvent, Flow, Hooks, Host, NetFromServerEvent, ScreenCharTypedPreEvent, ScreenClosingEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, Text,
};

use super::track_station::{EDIT_CHANNEL, OPEN_CHANNEL, VIEW_CHANNEL};

/// The mod's own screens belong to mod `create` (Ponder's and the value board's too: only one is open at a time).
const MOD: &str = "create";
const SEP: char = '\u{1f}';

struct Screens;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Screens)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Station,
    Assembly,
}

/// The open screen.
struct Open {
    pos: [i32; 3],
    view: StationView,
    kind: Kind,
    station: StationScreen,
    assembly: AssemblyScreen,
    name: TextField,
    train_name: TextField,
    /// The size of the window the screen is drawn in (GUI pixels), as of the last frame.
    size: [i32; 2],
    /// `onClose` was asked for: the next tick closes the screen.
    close: bool,
}

impl Open {
    fn new(pos: [i32; 3], view: StationView) -> Open {
        let mut open = Open {
            pos,
            kind: if view.assembling { Kind::Assembly } else { Kind::Station },
            station: StationScreen::new(view.door_control),
            assembly: AssemblyScreen::default(),
            name: TextField::new(rules::STATION_NAME_LENGTH),
            train_name: TextField::new(rules::TRAIN_NAME_LENGTH),
            size: [0, 0],
            close: false,
            view,
        };
        open.name.set(&open.view.name);
        open
    }

    fn height(&self) -> i32 {
        match self.kind {
            Kind::Station => rules::STATION_HEIGHT,
            Kind::Assembly => rules::ASSEMBLY_HEIGHT,
        }
    }

    /// The window's corner.
    fn corner(&self) -> (i32, i32) {
        ((self.size[0] - rules::STATION_WIDTH) / 2, (self.size[1] - self.height()) / 2)
    }

    /// `StationScreen.init` again: a new screen for the same station.
    fn to_station(&mut self) {
        self.kind = Kind::Station;
        self.station = StationScreen::new(self.view.door_control);
        self.name.set(&self.view.name);
        self.train_name.set("");
    }

    /// The assembly button: `StationScreen.removed()` says what to do (with `switchingToAssemblyMode`), then the other screen.
    fn to_assembly(&mut self) {
        self.station.switching = true;
        for message in self.station.closing(&self.name.value(), &self.train_name.value(), self.pos) {
            send(message);
        }
        self.kind = Kind::Assembly;
        self.assembly = AssemblyScreen::default();
    }

    fn tick(&mut self) {
        match self.kind {
            Kind::Station => {
                match self.station.tick_train_display(&self.view) {
                    Some(NameBox::Show(name)) => {
                        self.train_name.set(&name);
                        self.train_name.focus(false);
                    }
                    Some(NameBox::Hide) => {
                        self.train_name.set("");
                        self.train_name.focus(false);
                    }
                    None => {}
                }
                // A name changed elsewhere (a command, a Name Tag) shows unless it is being edited.
                if !self.name.focused && self.name.value() != self.view.name {
                    self.name.set(&self.view.name);
                }
                if let Some(train) = &self.station.displayed
                    && !self.train_name.focused
                    && self.train_name.value() != train.name
                {
                    self.train_name.set(&train.name);
                }
            }
            Kind::Assembly => {
                if self.assembly.tick(&self.view) == AssemblyTick::BackToStation {
                    send(Message::Station { pos: self.pos, edit: rules::configure(false, &self.view.name, None) });
                    self.to_station();
                }
            }
        }
    }
}

static CLIENT: Mutex<Option<Open>> = Mutex::new(None);
/// Messages for the server, sent at the next client tick (the screen's hooks have no `&mut Host`).
static OUTBOX: Mutex<Vec<Message>> = Mutex::new(Vec::new());

fn client() -> std::sync::MutexGuard<'static, Option<Open>> {
    CLIENT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn send(message: Message) {
    OUTBOX.lock().unwrap_or_else(PoisonError::into_inner).push(message);
}

fn shown(host: &dyn Host) -> bool {
    host.mod_screen() == Some(MOD)
}

impl Hooks for Screens {
    /// The server opens the screen (`create:station_open`: `x y z` and the view) and keeps it up to date (`create:station_view`).
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        let Ok(text) = std::str::from_utf8(event.data) else { return };
        match event.channel {
            OPEN_CHANNEL => {
                let mut parts = text.splitn(4, SEP);
                let pos: Option<[i32; 3]> = (|| Some([parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?]))();
                let (Some(pos), Some(view)) = (pos, parts.next().and_then(StationView::decode)) else { return };
                if host.mod_screen_open(MOD) {
                    *client() = Some(Open::new(pos, view));
                }
            }
            VIEW_CHANNEL => {
                if let (Some(view), Some(open)) = (StationView::decode(text), client().as_mut()) {
                    open.view = view;
                }
            }
            _ => {}
        }
    }

    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        if !host.window() {
            return;
        }
        let messages = std::mem::take(&mut *OUTBOX.lock().unwrap_or_else(PoisonError::into_inner));
        for message in messages {
            let _ = host.net_send_to_server(EDIT_CHANNEL, message.encode().as_bytes());
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return };
        if !shown(&*host) {
            // The screen went without telling us (another screen took its place).
            *c = None;
            send(Message::Closed);
            return;
        }
        if open.close {
            *c = None;
            drop(c);
            host.mod_screen_close(MOD);
            return;
        }
        open.tick();
    }

    /// `removed()`: what goes to the server when the screen closes.
    fn screen_closing(&self, _: &dyn Host, event: &mut ScreenClosingEvent<'_>) {
        if event.screen.id != "mod" {
            return;
        }
        let Some(open) = client().take() else { return };
        if open.kind == Kind::Station {
            for message in open.station.closing(&open.name.value(), &open.train_name.value(), open.pos) {
                send(message);
            }
        }
        send(Message::Closed);
    }

    fn screen_render_post(&self, host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>) {
        if event.screen.id != "mod" || !shown(host) {
            return;
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return };
        open.size = [event.batch.width as i32, event.batch.height as i32];
        draw(host, event, open);
    }

    fn screen_mouse_button_pressed_pre(&self, host: &dyn Host, event: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || !shown(host) {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return Flow::Continue };
        if open.size[0] == 0 {
            return Flow::Cancel;
        }
        click(open, event.screen.mouse, event.button);
        Flow::Cancel
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn Host, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || !shown(host) {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return Flow::Continue };
        let (left, top) = open.corner();
        if open.kind == Kind::Station && station_layout::DOOR_CONTROL.contains(left, top, event.screen.mouse) {
            open.station.step_door(if event.scroll[1] > 0.0 { 1 } else { -1 });
        }
        Flow::Cancel
    }

    fn screen_char_typed_pre(&self, host: &dyn Host, event: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || !shown(host) {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return Flow::Continue };
        if open.name.focused {
            open.name.insert(event.c);
        } else if open.train_name.focused {
            open.train_name.insert(event.c);
        }
        Flow::Cancel
    }

    fn screen_key_pressed_pre(&self, host: &dyn Host, event: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if event.screen.id != "mod" || !shown(host) {
            return Flow::Continue;
        }
        let mut c = client();
        let Some(open) = c.as_mut() else { return Flow::Continue };
        let editing = open.name.focused || open.train_name.focused;
        if !editing || event.key == "key.keyboard.escape" {
            return Flow::Continue;
        }
        // `StationScreen.keyPressed`: Enter ends the edit and sends the name.
        if event.key.ends_with("enter") {
            if open.name.focused {
                open.name.focus(false);
                if let Some(m) = open.station.sync_station(&open.name.value(), &open.view.name, open.pos) {
                    send(m);
                }
            } else {
                open.train_name.focus(false);
                if let Some(m) = open.station.sync_train(&open.train_name.value()) {
                    send(m);
                }
            }
            return Flow::Cancel;
        }
        let field = if open.name.focused { &mut open.name } else { &mut open.train_name };
        field.key(event.key);
        Flow::Cancel
    }
}

/// A left or right click at `mouse`.
fn click(open: &mut Open, mouse: [f32; 2], button: i32) {
    let (left, top) = open.corner();
    let hit = |r: rules::Rect| r.contains(left, top, mouse);
    match open.kind {
        Kind::Station => {
            let focus_before = (open.name.focused, open.train_name.focused);
            open.name.focus(false);
            open.train_name.focus(false);
            if hit(station_layout::CONFIRM) {
                open.close = true;
            } else if hit(station_layout::NAME_STRIP) && !focus_before.0 {
                open.name.focus(true);
                open.name.select_all();
            } else if open.station.displayed.is_some() && hit(station_layout::TRAIN_NAME_STRIP) && !focus_before.1 {
                open.train_name.focus(true);
                open.train_name.select_all();
            } else if open.station.new_train.visible && open.station.new_train.active && hit(station_layout::NEW_TRAIN) {
                open.to_assembly();
            } else if open.station.disassemble.visible && open.station.disassemble.active && hit(station_layout::DISASSEMBLE) {
                // `disassembleTrainButton`'s callback is the assembly one: entering assembly mode takes the train apart.
                open.to_assembly();
            } else if open.station.drop_schedule.visible && open.station.drop_schedule.active && hit(station_layout::DROP_SCHEDULE) {
                send(Message::Station { pos: open.pos, edit: rules::drop_schedule() });
            } else if hit(station_layout::DOOR_CONTROL) {
                open.station.click_door(button == 1);
            }
        }
        Kind::Assembly => {
            if hit(assembly_layout::CONFIRM) {
                open.close = true;
            } else if hit(assembly_layout::QUIT) {
                send(Message::Station { pos: open.pos, edit: rules::configure(false, &open.view.name, None) });
                open.to_station();
            } else if open.assembly.toggle_active && hit(assembly_layout::TOGGLE) {
                send(Message::Station { pos: open.pos, edit: rules::try_assemble() });
            }
        }
    }
}

// --- drawing -------------------------------------------------------------------------------------------------------

const DIM: u32 = 0x60_101010;
const PANEL: u32 = 0xFF_D8CCB4;
const FRAME: u32 = 0xFF_8C7A5E;
const BUTTON: u32 = 0xFF_B3A488;
const BUTTON_HOVER: u32 = 0xFF_E6DABF;
const BUTTON_OFF: u32 = 0xFF_9C9686;
const TEXTBOX: u32 = 0xFF_34312D;
const NAME_INK: u32 = 0x59_2424;
const GREY_INK: u32 = 0x7A_7A7A;
const TRAIN_INK: u32 = 0xC6_C6C6;

fn rect(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, x: i32, y: i32, w: i32, h: i32, argb: u32) {
    host.fill(event.batch, [x as f32, y as f32, w as f32, h as f32], argb);
}

/// A button: returns whether the pointer is on it.
#[allow(clippy::too_many_arguments)]
fn button(
    host: &dyn Host,
    event: &mut ScreenRenderPostEvent<'_>,
    corner: (i32, i32),
    at: rules::Rect,
    on: bool,
    label: &str,
) -> bool {
    let (x, y) = (corner.0 + at.x, corner.1 + at.y);
    let hovered = on && at.contains(corner.0, corner.1, event.screen.mouse);
    rect(host, event, x - 1, y - 1, at.w + 2, at.h + 2, FRAME);
    rect(host, event, x, y, at.w, at.h, if !on { BUTTON_OFF } else if hovered { BUTTON_HOVER } else { BUTTON });
    let ink = if on { 0x30_2820 } else { 0x77_7366 };
    host.text(event.batch, label, [x as f32 + (at.w as f32 - host.text_width(label)) / 2.0, y as f32 + (at.h as f32 - 8.0) / 2.0], ink, false);
    hovered
}

/// An `EditBox` line: the text, the selection and the cursor.
fn field(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, f: &TextField, x: i32, y: i32, ink: u32) {
    let text = f.value();
    if f.is_selected() {
        rect(host, event, x, y - 1, host.text_width(&text) as i32 + 1, 10, 0xA0_4C7FFF);
    }
    host.text(event.batch, &text, [x as f32, y as f32], ink, false);
    if f.focused {
        let before: String = text.chars().take(f.cursor()).collect();
        rect(host, event, x + host.text_width(&before) as i32, y - 1, 1, 10, 0xFF_000000 | ink);
    }
}

fn draw(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, open: &mut Open) {
    let [w, h] = open.size;
    rect(host, event, 0, 0, w, h, DIM);
    let corner = open.corner();
    let (left, top) = corner;
    let height = open.height();
    rect(host, event, left - 2, top - 2, rules::STATION_WIDTH + 4, height + 4, FRAME);
    rect(host, event, left, top, rules::STATION_WIDTH, height, PANEL);
    let mouse = event.screen.mouse;
    let mut tip: Option<Vec<String>> = None;
    match open.kind {
        Kind::Station => draw_station(host, event, open, corner, &mut tip),
        Kind::Assembly => draw_assembly(host, event, open, corner, &mut tip),
    }
    if let Some(lines) = tip {
        let lines: Vec<Text> = lines.into_iter().map(Text::literal).collect();
        host.gui_tooltip(event.batch, &lines, mouse);
    }
}

fn draw_station(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, open: &mut Open, corner: (i32, i32), tip: &mut Option<Vec<String>>) {
    let (left, top) = corner;
    // The station's name, centred in the strip along the top.
    let name = open.name.value();
    let nx = rules::name_box_x(left, rules::STATION_WIDTH, host.text_width(&name), station_layout::NAME_BOX.w);
    field(host, event, &open.name, nx, top + 4, NAME_INK);
    if !open.name.focused {
        // The hint that it can be edited (`STATION_EDIT_NAME`).
        host.text(event.batch, "(edit)", [(nx + host.text_width(&name) as i32 + 5) as f32, (top + 4) as f32], 0xA0_8F77, false);
    }

    match open.station.displayed.clone() {
        None => {
            let idle = rules::say("idle", &[]);
            host.text(event.batch, &idle, [left as f32 + 97.0 - host.text_width(&idle) / 2.0, top as f32 + 47.0], GREY_INK, false);
        }
        Some(train) => {
            let position = open.station.position;
            let icon_end = {
                let strip = rules::icon_strip(&train.spacings, open.view.train_backwards, position, rules::STATION_WIDTH);
                for piece in &strip {
                    let alpha = ((piece.alpha.clamp(0.0, 1.0)) * 255.0) as u32;
                    let colour = if piece.length_or_engine == rules::ENGINE { 0x6E_3B3B } else { 0x8A_7B5E };
                    let x = left + position as i32 + piece.x;
                    rect(host, event, x, top + rules::station_layout::ICON_ROW_Y, piece.width, 10, (alpha << 24) | colour);
                    rect(host, event, x + 1, top + rules::station_layout::ICON_ROW_Y + 2, piece.width - 2, 3, (alpha << 24) | 0xD8_D2C0);
                }
                strip.last().map_or(0, |p| p.x + p.width)
            };
            // The text box with the train's name, and the tip of its speech bubble under the icon.
            rect(host, event, left + 21, top + 42, 150, 48, TEXTBOX);
            rect(host, event, left + rules::speech_x(position, icon_end) as i32, top + 38, 8, 6, TEXTBOX);
            let tname = open.train_name.value();
            let tx = rules::name_box_x(left, rules::STATION_WIDTH, host.text_width(&tname), station_layout::TRAIN_NAME_BOX.w);
            field(host, event, &open.train_name, tx, top + 47, TRAIN_INK);
            if !open.train_name.focused {
                host.text(event.batch, "(edit)", [(tx + host.text_width(&tname) as i32 + 5).min(left + 156) as f32, (top + 47) as f32], 0x8A_8A8A, false);
            }
        }
    }

    // The buttons.
    let st = &open.station;
    if st.new_train.visible && button(host, event, corner, station_layout::NEW_TRAIN, st.new_train.active, "+") {
        *tip = Some(StationScreen::assembly_tooltip_lines(&open.view, "create_train"));
    }
    if st.disassemble.visible && button(host, event, corner, station_layout::DISASSEMBLE, st.disassemble.active, "-") {
        *tip = Some(StationScreen::assembly_tooltip_lines(&open.view, "disassemble_train"));
    }
    if st.drop_schedule.visible
        && button(host, event, corner, station_layout::DROP_SCHEDULE, st.drop_schedule.active, "S")
        && let Some(key) = StationScreen::drop_schedule_tooltip(&open.view)
    {
        *tip = Some(vec![rules::say(key, &[])]);
    }
    button(host, event, corner, station_layout::CONFIRM, true, "OK");

    // "Onboard Door Control".
    host.item(event.batch, "create:train_door", [(left + station_layout::DOOR_ITEM.0) as f32, (top + station_layout::DOOR_ITEM.1) as f32], 16.0);
    let door = open.station.door;
    if button(host, event, corner, station_layout::DOOR_CONTROL, true, door.long()) {
        *tip = Some(vec!["Onboard Door Control".to_owned()]);
    }
}

fn draw_assembly(host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>, open: &mut Open, corner: (i32, i32), tip: &mut Option<Vec<String>>) {
    let (left, top) = corner;
    let title = rules::say("assembly_title", &[]);
    host.text(event.batch, &title, [left as f32 + rules::STATION_WIDTH as f32 / 2.0 - host.text_width(&title) / 2.0, top as f32 + 4.0], 0x0E_2233, false);
    let measure = |s: &str| host.text_width(s);
    for line in rules::assembly_body(&open.view, &measure) {
        let rows = match line.wrap {
            Some(width) => rules::wrap(&line.text, width as f32, &measure),
            None => vec![line.text.clone()],
        };
        for (i, row) in rows.iter().enumerate() {
            let x = if line.centred { left as f32 + line.x as f32 - host.text_width(row) / 2.0 } else { (left + line.x) as f32 };
            host.text(event.batch, row, [x, (top + line.y + 9 * i as i32) as f32], line.color, false);
        }
    }
    if button(host, event, corner, assembly_layout::QUIT, true, "X") {
        *tip = Some(vec![rules::say("cancel", &[])]);
    }
    if button(host, event, corner, assembly_layout::TOGGLE, open.assembly.toggle_active, "Go") {
        *tip = Some(vec![rules::say("assemble_train", &[])]);
    }
    if button(host, event, corner, assembly_layout::CONFIRM, true, "-") {
        *tip = Some(vec![rules::say("close", &[])]);
    }
}

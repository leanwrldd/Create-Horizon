//! The Train Station's two screens: `StationScreen` (the station's name, the train standing at it, its name, "Onboard Door
//! Control", the buttons to enter assembly mode, disassemble and retrieve the Schedule) and `AssemblyScreen` (the bogey count,
//! the failure, "Assemble Train" and "Cancel Assembly"), with what they exchange with the server: `StationEditPacket` and
//! `TrainEditPacket.Serverbound`. Everything that isn't drawing is here: the text field, the state each screen keeps from tick
//! to tick (`tickTrainDisplay`, `updateAssemblyTooltip`), the train icons (`TrainIconType`), the layout, the messages and the
//! view of the station the server sends the screen.
//!
//! Ported from `content/trains/station/{AbstractStationScreen,StationScreen,AssemblyScreen,StationEditPacket,TrainEditPacket}.java`
//! and `content/trains/entity/TrainIconType.java`. The English is `create.station.*` of Create's `en_us.json`.
//!
//! Differences from Create, for the README:
//! - Java's screens read the block entity, the station and the imminent train straight from the client's copy of the railways;
//!   a client here has no graphs, so the server sends the same facts as a [`StationView`] while the screen is open.
//! - The Color-on-Maps scroll (`colorTypeScroll`) is only shown when FTB Chunks, JourneyMap or Xaero's World Map is loaded
//!   (`mapModsPresent`); none of them runs on Horizon, so it is never shown and the colour is sent back as it was.
//! - `AssemblyScreen.displayedTrain` is never assigned in Create 6.0.10, so its icon-type scroll is never shown and its
//!   `removed()` never sends an icon. The scroll's rule is ported as [`icon_step`] and the icon can be set with `/train icon`.
//! - The door control's `SelectionScrollInput` is a button that steps through the modes on a click (back with a right click)
//!   and on the wheel.
//! - The window is drawn in flat rectangles (the textures are Create's art, which the mod doesn't carry), and the train's icon
//!   as plain blocks of the icon's real width.

use super::edge_blocks::StationEdit;
use super::train_doors::DoorControl;

// --- the window ----------------------------------------------------------------------------------------------

/// `AllGuiTextures.STATION` (200 x 127) and `STATION_ASSEMBLING` (200 x 178).
pub const STATION_WIDTH: i32 = 200;
pub const STATION_HEIGHT: i32 = 127;
pub const ASSEMBLY_HEIGHT: i32 = 178;
/// `nameBox.setMaxLength(25)` and `trainNameBox.setMaxLength(35)`.
pub const STATION_NAME_LENGTH: usize = 25;
pub const TRAIN_NAME_LENGTH: usize = 35;
/// `leavingAnimation = 80`.
pub const LEAVING_TICKS: i32 = 80;

/// A widget's place, in GUI pixels from the window's corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Whether the point is in the rectangle once the window sits at `(left, top)`.
    pub fn contains(&self, left: i32, top: i32, point: [f32; 2]) -> bool {
        let (x, y) = (point[0], point[1]);
        x >= (left + self.x) as f32 && x < (left + self.x + self.w) as f32 && y >= (top + self.y) as f32 && y < (top + self.y + self.h) as f32
    }
}

/// `IconButton` is 18 x 18 and `WideIconButton` 26 x 18.
const ICON: i32 = 18;
const WIDE: i32 = 26;

/// Where `StationScreen.init` puts its widgets.
pub mod station_layout {
    use super::{Rect, ICON, STATION_WIDTH, WIDE};
    /// `confirmButton` of `AbstractStationScreen.init`: `x + width - 33, y + height - 24`.
    pub const CONFIRM: Rect = Rect::new(STATION_WIDTH - 33, super::STATION_HEIGHT - 24, ICON, ICON);
    /// `newTrainButton` and `disassembleTrainButton` share the row of `dropScheduleButton`.
    pub const NEW_TRAIN: Rect = Rect::new(84, 65, WIDE, ICON);
    pub const DISASSEMBLE: Rect = Rect::new(94, 65, WIDE, ICON);
    pub const DROP_SCHEDULE: Rect = Rect::new(73, 65, ICON, ICON);
    /// `DoorControl.createWidget(x + 35, y + 102, ..)`: a 53 x 16 scroll input.
    pub const DOOR_CONTROL: Rect = Rect::new(35, 102, 53, 16);
    /// The click areas `mouseClicked` gives the two names: the whole top strip and the train name's line.
    pub const NAME_STRIP: Rect = Rect::new(0, 0, STATION_WIDTH, 14);
    pub const TRAIN_NAME_STRIP: Rect = Rect::new(25, 45, 143, 13);
    /// `nameBox` (`x + 23, y + 4`, width `background.getWidth() - 20`, 10 high) and `trainNameBox` (`x + 23, y + 47`, width
    /// `background.getWidth() - 75`).
    pub const NAME_BOX: Rect = Rect::new(23, 4, STATION_WIDTH - 20, 10);
    pub const TRAIN_NAME_BOX: Rect = Rect::new(23, 47, STATION_WIDTH - 75, 10);
    /// The room the icons run in: the first piece starts at the window's corner plus the train's position.
    pub const ICON_ROW_Y: i32 = 20;
    /// The door's item (`AllBlocks.TRAIN_DOOR`) is drawn at `x + 14, y + 103`.
    pub const DOOR_ITEM: (i32, i32) = (14, 103);
}

/// Where `AssemblyScreen.init` puts its widgets.
pub mod assembly_layout {
    use super::{Rect, ICON, WIDE};
    const BY: i32 = super::ASSEMBLY_HEIGHT - 24;
    pub const CONFIRM: Rect = Rect::new(super::STATION_WIDTH - 33, BY, ICON, ICON);
    /// `quitAssembly` (`x + 73`) and `toggleAssemblyButton` (`x + 94`).
    pub const QUIT: Rect = Rect::new(73, BY, ICON, ICON);
    pub const TOGGLE: Rect = Rect::new(94, BY, WIDE, ICON);
    /// `iconTypeScroll`.
    pub const ICON_SCROLL: Rect = Rect::new(4, 17, 162, 14);
}

// --- the text -------------------------------------------------------------------------------------------------

/// `create.station.<key>` in Create's English; `None` for a key not listed.
pub fn text(key: &str) -> Option<&'static str> {
    Some(match key {
        "assemble_train" => "Assemble Train",
        "assembly_title" => "Train Assembly",
        "cancel" => "Cancel Assembly",
        "carriage_number" => "Carriage %1$s:",
        "close" => "Close Window",
        "create_train" => "Create new Train",
        "disassemble_train" => "Disassemble Train",
        "failed" => "Assembly Failed",
        "how_to" => "Use Train Casing on highlighted Tracks to create bogeys.",
        "how_to_1" => "Remove bogeys by breaking the block on top.",
        "how_to_2" => "Build carriages attached to one or two bogeys each.",
        "icon_type" => "Icon Type",
        "idle" => "Station is Idle",
        "more_bogeys" => "%1$s Bogeys",
        "no_assembly_curve" => "Cannot build trains",
        "no_assembly_curve_1" => "on curved tracks",
        "no_assembly_diagonal" => "Cannot build trains",
        "no_assembly_diagonal_1" => "on diagonal tracks",
        "no_bogeys" => "No Bogeys",
        "one_bogey" => "1 Bogey",
        "remove_auto_schedule" => "Discard Auto-Schedule",
        "remove_schedule" => "Retrieve Schedule",
        "retry" => "Resolve this and retry",
        "train_map_color" => "Color on Maps",
        "train_not_aligned" => "Cannot disassemble,",
        "train_not_aligned_1" => "not all carriages aligned",
        _ => return None,
    })
}

/// `CreateLang.translateDirect("station." + key, args)`.
pub fn say(key: &str, args: &[String]) -> String {
    let mut out = text(key).unwrap_or(key).to_owned();
    for (i, arg) in args.iter().enumerate() {
        out = out.replace(&format!("%{}$s", i + 1), arg);
    }
    out
}

/// `Font.split` at `width`: greedy breaks at spaces (a word wider than the line stands alone).
pub fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if line.is_empty() || measure(&candidate) <= width {
            line = candidate;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

// --- the train icons --------------------------------------------------------------------------------------------

/// `TrainIconType`: an icon on the `assemble.png` sheet, `x` and `y` its first (engine-less) strip's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconType {
    pub id: &'static str,
    pub x: i32,
    pub y: i32,
}

/// `TrainIconType`'s static block, in the order they are registered.
pub const ICON_TYPES: [IconType; 3] = [
    IconType { id: "create:traditional", x: 2, y: 205 },
    IconType { id: "create:electric", x: 2, y: 216 },
    IconType { id: "create:modern", x: 2, y: 227 },
];

/// `TrainIconType.ENGINE` and `FLIPPED_ENGINE`.
pub const ENGINE: i32 = -1;
pub const FLIPPED_ENGINE: i32 = -2;

/// `TrainIconType.byId`: an id nobody registered is the default (`traditional`).
pub fn icon_type(id: &str) -> IconType {
    ICON_TYPES.iter().copied().find(|i| i.id == id).unwrap_or(ICON_TYPES[0])
}

/// The index of `id` in [`ICON_TYPES`] (the scroll's state).
pub fn icon_index(id: &str) -> usize {
    ICON_TYPES.iter().position(|i| i.id == id).unwrap_or(0)
}

/// `iconTypeScroll.withRange(0, size)`: the scroll's state `index` clamped to the registered ones.
pub fn icon_at(index: i32) -> IconType {
    ICON_TYPES[index.clamp(0, ICON_TYPES.len() as i32 - 1) as usize]
}

/// `iconTypeScroll.withStepFunction(ctx -> -standardStep(ctx))`: the wheel's step is reversed.
pub fn icon_step(standard: i32) -> i32 {
    -standard
}

/// A word for `/train icon <word>`: `traditional` or `create:traditional`.
pub fn icon_by_word(word: &str) -> Option<IconType> {
    let word = word.trim().to_ascii_lowercase();
    ICON_TYPES.iter().copied().find(|i| i.id == word || i.id.strip_prefix("create:") == Some(word.as_str()))
}

/// `TrainIconType.getIconWidth`.
pub fn icon_width(length_or_engine: i32) -> i32 {
    if length_or_engine == FLIPPED_ENGINE || length_or_engine == ENGINE {
        19
    } else if length_or_engine < 3 {
        7
    } else if length_or_engine < 9 {
        13
    } else {
        19
    }
}

/// `TrainIconType.getIconOffset`: where the piece starts on the sheet, from the type's `x`.
pub fn icon_offset(length_or_engine: i32) -> i32 {
    if length_or_engine == FLIPPED_ENGINE {
        0
    } else if length_or_engine == ENGINE {
        62
    } else if length_or_engine < 3 {
        34
    } else if length_or_engine < 9 {
        20
    } else {
        42
    }
}

/// `TrainIconType.render`'s blit source: `(sheet x, sheet y, width, height)`.
pub fn icon_source(icon: IconType, length_or_engine: i32) -> (i32, i32, i32, i32) {
    (icon.x + icon_offset(length_or_engine), icon.y, icon_width(length_or_engine), 10)
}

/// `AbstractStationScreen.getTrainIconWidth`: the engine, then every carriage after the first (the last of a double ended
/// train is a flipped engine), each with a pixel between. `spacings` is each carriage's `bogeySpacing`.
pub fn train_icon_width(spacings: &[i32], double_ended: bool) -> i32 {
    let mut w = icon_width(ENGINE);
    if spacings.len() <= 1 {
        return w;
    }
    for i in 1..spacings.len() {
        if i == spacings.len() - 1 && double_ended {
            w += icon_width(FLIPPED_ENGINE) + 1;
            break;
        }
        w += icon_width(spacings[i]) + 1;
    }
    w
}

/// One piece of the icon `StationScreen.renderWindow` draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconPiece {
    pub length_or_engine: i32,
    /// From the train's position.
    pub x: i32,
    pub width: i32,
    /// `setShaderColor(1, 1, 1, alpha)`: the fade at both ends of the window.
    pub alpha: f32,
}

/// The pieces of the displayed train, the last carriage first and the engine at the end (`StationScreen.renderWindow`).
pub fn icon_strip(spacings: &[i32], backwards: bool, position: f32, bg_width: i32) -> Vec<IconPiece> {
    let fade = |offset: i32| ((position + offset as f32 - 10.0) / 30.0).min((bg_width as f32 - 40.0 - position - offset as f32) / 30.0).min(1.0);
    let mut pieces = Vec::new();
    let mut offset = 0;
    let n = spacings.len();
    for i in (1..n).rev() {
        let carriage = if backwards { n - i - 1 } else { i };
        let width = icon_width(spacings[carriage]);
        pieces.push(IconPiece { length_or_engine: spacings[carriage], x: offset, width, alpha: fade(offset) });
        offset += width + 1;
    }
    pieces.push(IconPiece { length_or_engine: ENGINE, x: offset, width: icon_width(ENGINE), alpha: fade(offset) });
    pieces
}

/// The x of the speech bubble's tip (`ms.translate(Mth.clamp(position + offset - 13, 25, 159), 0, 0)`), with `offset` the end
/// of the icon strip.
pub fn speech_x(position: f32, icon_end: i32) -> f32 {
    (position + icon_end as f32 - 13.0).clamp(25.0, 159.0)
}

// --- the text field ------------------------------------------------------------------------------------------------

/// A single line of editable text (the part of `EditBox` the station screens use).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextField {
    chars: Vec<char>,
    cursor: usize,
    pub max_len: usize,
    pub focused: bool,
    /// `setHighlightPos(0)` after the cursor went to the end: the whole text is selected, and the next edit replaces it.
    selected: bool,
}

impl TextField {
    pub fn new(max_len: usize) -> TextField {
        TextField { max_len, ..TextField::default() }
    }

    /// `setValue`: cut to the length and the cursor at the end.
    pub fn set(&mut self, value: &str) {
        self.chars = value.chars().filter(|c| !c.is_control()).take(self.max_len).collect();
        self.cursor = self.chars.len();
        self.selected = false;
    }

    /// `setHighlightPos(0)` with the cursor at the end: select everything.
    pub fn select_all(&mut self) {
        self.cursor = self.chars.len();
        self.selected = !self.chars.is_empty();
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    fn replace_selection(&mut self) {
        if self.selected {
            self.chars.clear();
            self.cursor = 0;
            self.selected = false;
        }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// `charTyped`: a printable character is put at the cursor when there is room.
    pub fn insert(&mut self, c: char) -> bool {
        if c.is_control() {
            return false;
        }
        self.replace_selection();
        if self.chars.len() >= self.max_len {
            return false;
        }
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.selected {
            self.replace_selection();
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.selected {
            self.replace_selection();
            return true;
        }
        if self.cursor >= self.chars.len() {
            return false;
        }
        self.chars.remove(self.cursor);
        true
    }

    pub fn left(&mut self) {
        self.selected = false;
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.selected = false;
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.selected = false;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.selected = false;
        self.cursor = self.chars.len();
    }

    /// `EditBox.setFocused`; unfocused, the cursor goes to the end (`tick`'s `setCursorPosition(length)`).
    pub fn focus(&mut self, on: bool) {
        self.focused = on;
        if !on {
            self.cursor = self.chars.len();
            self.selected = false;
        }
    }

    /// Reacts to a key by its vanilla name; true when the key was the field's.
    pub fn key(&mut self, key: &str) -> bool {
        match key {
            "key.keyboard.backspace" => {
                self.backspace();
            }
            "key.keyboard.delete" => {
                self.delete();
            }
            "key.keyboard.left" => self.left(),
            "key.keyboard.right" => self.right(),
            "key.keyboard.home" => self.home(),
            "key.keyboard.end" => self.end(),
            _ => return false,
        }
        true
    }
}

/// `StationScreen.nameBoxX`: the box is moved so the text stays centred (`+ 10` for the pencil).
pub fn name_box_x(gui_left: i32, bg_width: i32, text_width: f32, box_width: i32) -> i32 {
    gui_left + bg_width / 2 - ((text_width.min(box_width as f32) + 10.0) / 2.0) as i32
}

// --- what the server tells the screen --------------------------------------------------------------------------------

/// The train imminent at the station (`getImminent()`), as the screen needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrainInfo {
    /// The train's id (a number in text).
    pub id: String,
    pub name: String,
    pub icon: String,
    /// Each carriage's `bogeySpacing`.
    pub spacings: Vec<i32>,
    pub double_ended: bool,
    /// `navigation.distanceToDestination`.
    pub distance: f32,
    pub map_color: i32,
}

/// What `AbstractStationScreen` reads off the block entity and the train.
#[derive(Clone, Debug, PartialEq)]
pub struct StationView {
    /// `station.name`.
    pub name: String,
    pub assembling: bool,
    pub door_control: DoorControl,
    /// `blockEntity.bogeyCount`.
    pub bogey_count: i32,
    pub on_curve: bool,
    pub orthogonal: bool,
    pub train_present: bool,
    pub train_can_disassemble: bool,
    pub train_has_schedule: bool,
    pub train_has_auto_schedule: bool,
    pub train_backwards: bool,
    /// `blockEntity.lastException`'s text and `failedCarriageIndex` (-1 for none).
    pub exception: Option<(String, i32)>,
    pub train: Option<TrainInfo>,
}

impl Default for StationView {
    fn default() -> StationView {
        StationView {
            name: String::new(),
            assembling: false,
            door_control: DoorControl::All,
            bogey_count: 0,
            on_curve: false,
            orthogonal: true,
            train_present: false,
            train_can_disassemble: false,
            train_has_schedule: false,
            train_has_auto_schedule: false,
            train_backwards: false,
            exception: None,
            train: None,
        }
    }
}

const SEP: char = '\u{1f}';

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn flag(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

impl StationView {
    /// The text the server sends (`create:station_view`): `view`, then the fields.
    pub fn encode(&self) -> String {
        let mut f: Vec<String> = vec![
            "view".into(),
            clean(&self.name),
            flag(self.assembling).into(),
            self.door_control.constant().into(),
            self.bogey_count.to_string(),
            flag(self.on_curve).into(),
            flag(self.orthogonal).into(),
            flag(self.train_present).into(),
            flag(self.train_can_disassemble).into(),
            flag(self.train_has_schedule).into(),
            flag(self.train_has_auto_schedule).into(),
            flag(self.train_backwards).into(),
        ];
        match &self.exception {
            Some((text, index)) => {
                f.push("1".into());
                f.push(clean(text));
                f.push(index.to_string());
            }
            None => f.push("0".into()),
        }
        match &self.train {
            Some(t) => {
                f.push("1".into());
                f.push(clean(&t.id));
                f.push(clean(&t.name));
                f.push(clean(&t.icon));
                f.push(t.spacings.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
                f.push(flag(t.double_ended).into());
                f.push(t.distance.to_string());
                f.push(t.map_color.to_string());
            }
            None => f.push("0".into()),
        }
        f.join(&SEP.to_string())
    }

    pub fn decode(text: &str) -> Option<StationView> {
        let mut it = text.split(SEP);
        if it.next()? != "view" {
            return None;
        }
        let mut next = || it.next();
        let name = next()?.to_owned();
        let assembling = next()? == "1";
        let door_control = DoorControl::read(Some(next()?));
        let bogey_count = next()?.parse().ok()?;
        let on_curve = next()? == "1";
        let orthogonal = next()? == "1";
        let train_present = next()? == "1";
        let train_can_disassemble = next()? == "1";
        let train_has_schedule = next()? == "1";
        let train_has_auto_schedule = next()? == "1";
        let train_backwards = next()? == "1";
        let exception = if next()? == "1" {
            let text = next()?.to_owned();
            Some((text, next()?.parse().ok()?))
        } else {
            None
        };
        let train = if next()? == "1" {
            let id = next()?.to_owned();
            let name = next()?.to_owned();
            let icon = next()?.to_owned();
            let spacings = next()?.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<Vec<i32>>>()?;
            let double_ended = next()? == "1";
            let distance = next()?.parse().ok()?;
            let map_color = next()?.parse().ok()?;
            Some(TrainInfo { id, name, icon, spacings, double_ended, distance, map_color })
        } else {
            None
        };
        Some(StationView {
            name,
            assembling,
            door_control,
            bogey_count,
            on_curve,
            orthogonal,
            train_present,
            train_can_disassemble,
            train_has_schedule,
            train_has_auto_schedule,
            train_backwards,
            exception,
            train,
        })
    }
}

// --- what the screen tells the server -------------------------------------------------------------------------------------

/// `StationEditPacket.dropSchedule`.
pub fn drop_schedule() -> StationEdit {
    StationEdit { drop_schedule: true, ..StationEdit::default() }
}

/// `StationEditPacket.tryAssemble`.
pub fn try_assemble() -> StationEdit {
    StationEdit { try_assemble: Some(true), ..StationEdit::default() }
}

/// `StationEditPacket.tryDisassemble`.
pub fn try_disassemble() -> StationEdit {
    StationEdit { try_assemble: Some(false), ..StationEdit::default() }
}

/// `StationEditPacket.configure(pos, assemble, name, doorControl)`.
pub fn configure(assemble: bool, name: &str, door: Option<DoorControl>) -> StationEdit {
    StationEdit { assembly_mode: assemble, name: Some(name.to_owned()), door_control: door, ..StationEdit::default() }
}

/// `TrainEditPacket.Serverbound`'s fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainEdit {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub map_color: i32,
}

/// `TrainEditPacket.handleSided`: a blank name keeps the old one; the icon (an unknown id is the default) and the colour are set.
pub fn apply_train_edit(name: &mut String, icon: &mut String, color: &mut i32, edit: &TrainEdit) {
    if !edit.name.trim().is_empty() {
        *name = edit.name.clone();
    }
    *icon = icon_type(&edit.icon).id.to_owned();
    *color = edit.map_color;
}

/// What a screen can say to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// A `StationEditPacket` for the station at `pos`.
    Station { pos: [i32; 3], edit: StationEdit },
    /// A `TrainEditPacket.Serverbound`.
    Train(TrainEdit),
    /// The screen was closed: stop sending the view.
    Closed,
}

impl Message {
    pub fn encode(&self) -> String {
        let sep = SEP.to_string();
        match self {
            Message::Closed => "closed".to_owned(),
            Message::Train(t) => ["train".to_owned(), clean(&t.id), clean(&t.name), clean(&t.icon), t.map_color.to_string()].join(&sep),
            Message::Station { pos, edit } => [
                "station".to_owned(),
                pos[0].to_string(),
                pos[1].to_string(),
                pos[2].to_string(),
                flag(edit.drop_schedule).into(),
                flag(edit.assembly_mode).into(),
                match edit.try_assemble {
                    None => "0",
                    Some(true) => "1",
                    Some(false) => "2",
                }
                .into(),
                edit.door_control.map_or(String::new(), |d| d.constant().to_owned()),
                clean(edit.name.as_deref().unwrap_or("")),
            ]
            .join(&sep),
        }
    }

    pub fn decode(text: &str) -> Option<Message> {
        let mut it = text.split(SEP);
        match it.next()? {
            "closed" => Some(Message::Closed),
            "train" => {
                let id = it.next()?.to_owned();
                let name = it.next()?.to_owned();
                let icon = it.next()?.to_owned();
                let map_color = it.next()?.parse().ok()?;
                Some(Message::Train(TrainEdit { id, name, icon, map_color }))
            }
            "station" => {
                let pos = [it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?];
                let drop_schedule = it.next()? == "1";
                let assembly_mode = it.next()? == "1";
                let try_assemble = match it.next()? {
                    "1" => Some(true),
                    "2" => Some(false),
                    _ => None,
                };
                let door = it.next()?;
                let name = it.next()?;
                Some(Message::Station {
                    pos,
                    edit: StationEdit {
                        drop_schedule,
                        assembly_mode,
                        try_assemble,
                        door_control: (!door.is_empty()).then(|| DoorControl::read(Some(door))),
                        name: (!name.is_empty()).then(|| name.to_owned()),
                    },
                })
            }
            _ => None,
        }
    }
}

// --- StationScreen ---------------------------------------------------------------------------------------------------------

/// A widget that is shown and may be clicked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Button {
    pub active: bool,
    pub visible: bool,
}

/// What `StationScreen.tickTrainDisplay` asks the screen to do with the train's name box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameBox {
    /// A train arrived: add the box with its name.
    Show(String),
    /// The train is gone: remove it.
    Hide,
}

/// `AssemblyScreen.tickTrainDisplay`'s and `StationScreen`'s tooltip for `updateAssemblyTooltip(key)`.
pub fn assembly_tooltip(view: &StationView) -> Option<&'static str> {
    if view.on_curve {
        Some("no_assembly_curve")
    } else if !view.orthogonal {
        Some("no_assembly_diagonal")
    } else if view.train_present && !view.train_can_disassemble {
        Some("train_not_aligned")
    } else {
        None
    }
}

/// `StationScreen`'s state between ticks.
#[derive(Clone, Debug)]
pub struct StationScreen {
    /// `displayedTrain`.
    pub displayed: Option<TrainInfo>,
    pub leaving: i32,
    /// `trainPosition`.
    pub position: f32,
    pub new_train: Button,
    pub disassemble: Button,
    pub drop_schedule: Button,
    pub train_name_active: bool,
    /// `doorControl`.
    pub door: DoorControl,
    /// `switchingToAssemblyMode`.
    pub switching: bool,
}

impl StationScreen {
    pub fn new(door: DoorControl) -> StationScreen {
        StationScreen {
            displayed: None,
            leaving: 0,
            position: 0.0,
            new_train: Button { active: true, visible: true },
            disassemble: Button { active: false, visible: false },
            drop_schedule: Button { active: false, visible: false },
            train_name_active: false,
            door,
            switching: false,
        }
    }

    fn target_pos(icon_width: i32) -> f32 {
        let mut target = (STATION_WIDTH / 2 - icon_width / 2) as f32;
        if icon_width > 130 {
            target -= (icon_width - 130) as f32;
        }
        target
    }

    /// `tickTrainDisplay`.
    pub fn tick_train_display(&mut self, view: &StationView) -> Option<NameBox> {
        let Some(shown) = self.displayed.clone() else {
            let mut name_box = None;
            if self.train_name_active {
                self.train_name_active = false;
                name_box = Some(NameBox::Hide);
            }
            self.leaving = 0;
            self.new_train = Button { active: view.orthogonal, visible: true };
            if let Some(imminent) = view.train.clone() {
                self.new_train = Button { active: false, visible: false };
                self.disassemble = Button { active: false, visible: true };
                self.drop_schedule = Button { active: view.train_has_schedule, visible: true };
                self.train_name_active = true;
                let icon_width = train_icon_width(&imminent.spacings, imminent.double_ended);
                let target = StationScreen::target_pos(icon_width);
                let f = if view.train_present { 0.0 } else { imminent.distance / 15.0 };
                self.position = target - (target + 5.0) * f;
                name_box = Some(NameBox::Show(imminent.name.clone()));
                self.displayed = Some(imminent);
            }
            return name_box;
        };

        let icon_width = train_icon_width(&shown.spacings, shown.double_ended);
        let target = StationScreen::target_pos(icon_width);

        if self.leaving > 0 {
            self.disassemble.active = false;
            let f = 1.0 - self.leaving as f32 / LEAVING_TICKS as f32;
            self.position = target + f * f * f * (STATION_WIDTH as f32 - target + 5.0);
            self.leaving -= 1;
            if self.leaving > 0 {
                return None;
            }
            self.displayed = None;
            self.disassemble.visible = false;
            self.drop_schedule = Button { active: false, visible: false };
            return None;
        }

        if view.train.as_ref().map(|t| &t.id) != Some(&shown.id) {
            self.leaving = LEAVING_TICKS;
            return None;
        }
        // The train is read live: its name, icon and length follow.
        if let Some(live) = view.train.clone() {
            self.displayed = Some(live);
        }
        let shown = self.displayed.clone().unwrap_or(shown);
        let at_station = view.train_present;
        self.disassemble.active = at_station && view.train_can_disassemble && view.orthogonal;
        self.drop_schedule.active = view.train_has_schedule;
        let f = if at_station { 0.0 } else { shown.distance / 30.0 };
        let icon_width = train_icon_width(&shown.spacings, shown.double_ended);
        let target = StationScreen::target_pos(icon_width);
        self.position = target - (target + icon_width as f32) * f;
        None
    }

    /// The tooltip key of the schedule button: what it does with the train's schedule.
    pub fn drop_schedule_tooltip(view: &StationView) -> Option<&'static str> {
        view.train_has_schedule.then_some(if view.train_has_auto_schedule { "remove_auto_schedule" } else { "remove_schedule" })
    }

    /// The tooltip lines of "new train" / "disassemble": the reason one can't be used, else their own name.
    pub fn assembly_tooltip_lines(view: &StationView, own: &'static str) -> Vec<String> {
        match assembly_tooltip(view) {
            None => vec![say(own, &[])],
            Some(key) => vec![say(key, &[]), say(&format!("{key}_1"), &[])],
        }
    }

    /// The next door mode when the control is clicked (`SelectionScrollInput`: a step of `delta`, clamped to the list).
    pub fn step_door(&mut self, delta: i32) {
        let all = DoorControl::ALL;
        let at = all.iter().position(|d| *d == self.door).unwrap_or(0) as i32;
        self.door = all[(at + delta).clamp(0, all.len() as i32 - 1) as usize];
    }

    /// A click on the door control: forward, and round to the start after the last (a button has no wheel to go back with).
    pub fn click_door(&mut self, back: bool) {
        let all = DoorControl::ALL;
        let at = all.iter().position(|d| *d == self.door).unwrap_or(0) as i32;
        let n = all.len() as i32;
        self.door = all[(at + if back { -1 } else { 1 }).rem_euclid(n) as usize];
    }

    /// `removed()`: what goes to the server when the screen closes, `name` being the station name field.
    pub fn closing(&self, name: &str, train_name: &str, pos: [i32; 3]) -> Vec<Message> {
        let mut out = vec![Message::Station { pos, edit: configure(self.switching, name, Some(self.door)) }];
        if let Some(train) = &self.displayed
            && !self.switching
        {
            out.push(Message::Train(TrainEdit {
                id: train.id.clone(),
                name: train_name.to_owned(),
                icon: train.icon.clone(),
                map_color: train.map_color,
            }));
        }
        out
    }

    /// `syncTrainNameAndColor`: sent when the name differs from the train's.
    pub fn sync_train(&self, train_name: &str) -> Option<Message> {
        let train = self.displayed.as_ref()?;
        (train_name != train.name).then(|| {
            Message::Train(TrainEdit { id: train.id.clone(), name: train_name.to_owned(), icon: train.icon.clone(), map_color: train.map_color })
        })
    }

    /// `syncStationName`: sent (with the door) when the name differs from the station's.
    pub fn sync_station(&self, name: &str, station_name: &str, pos: [i32; 3]) -> Option<Message> {
        (name != station_name).then(|| Message::Station { pos, edit: configure(false, name, Some(self.door)) })
    }
}

// --- AssemblyScreen ----------------------------------------------------------------------------------------------------------

/// What `AssemblyScreen.tick` decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssemblyTick {
    Stay,
    /// A train is imminent: send `configure(false, name)` and open `StationScreen`.
    BackToStation,
}

/// `AssemblyScreen`'s state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssemblyScreen {
    /// `toggleAssemblyButton.active`.
    pub toggle_active: bool,
}

impl AssemblyScreen {
    pub fn tick(&mut self, view: &StationView) -> AssemblyTick {
        if view.train.is_some() {
            return AssemblyTick::BackToStation;
        }
        self.toggle_active = view.bogey_count > 0;
        AssemblyTick::Stay
    }

    /// The count line: `no_bogeys`, `one_bogey` or `more_bogeys`.
    pub fn bogey_text(count: i32) -> String {
        match count {
            0 => say("no_bogeys", &[]),
            1 => say("one_bogey", &[]),
            n => say("more_bogeys", &[n.to_string()]),
        }
    }
}

/// `AssemblyScreen.renderWindow`'s lines under the title: `(text, x from the centre line or the left, y, colour)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// True: the line is centred on `x`.
    pub centred: bool,
    pub x: i32,
    pub y: i32,
    pub color: u32,
    /// Wrap at this width, if any.
    pub wrap: Option<i32>,
}

/// The body of the assembly window: the failure (`failed`, the carriage, the message, `retry`) or the count and the help.
pub fn assembly_body(view: &StationView, measure: &dyn Fn(&str) -> f32) -> Vec<Line> {
    let line = |text: String, centred: bool, x: i32, y: i32, color: u32, wrap: Option<i32>| Line { text, centred, x, y, color, wrap };
    if let Some((message, carriage)) = &view.exception {
        let mut out = vec![line(say("failed", &[]), true, 97, 47, 0x77_5B5B, None)];
        let mut offset = 0;
        if *carriage != -1 {
            out.push(line(say("carriage_number", &[carriage.to_string()]), false, 30, 67, 0x7A_7A7A, None));
            offset += 10;
        }
        out.push(line(message.clone(), false, 30, 67 + offset, 0x77_5B5B, Some(134)));
        offset += wrap(message, 134.0, measure).len() as i32 * 9 + 5;
        out.push(line(say("retry", &[]), false, 30, 67 + offset, 0x7A_7A7A, Some(134)));
        return out;
    }
    vec![
        line(AssemblyScreen::bogey_text(view.bogey_count), true, 97, 47, 0x7A_7A7A, None),
        line(say("how_to", &[]), false, 28, 62, 0x7A_7A7A, Some(134)),
        line(say("how_to_1", &[]), false, 28, 94, 0x7A_7A7A, Some(134)),
        line(say("how_to_2", &[]), false, 28, 117, 0x7A_7A7A, Some(138)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn train(spacings: &[i32]) -> TrainInfo {
        TrainInfo {
            id: "7".into(),
            name: "Express".into(),
            icon: "create:electric".into(),
            spacings: spacings.to_vec(),
            double_ended: false,
            distance: 30.0,
            map_color: 3,
        }
    }

    fn with_train(spacings: &[i32]) -> StationView {
        StationView { train: Some(train(spacings)), train_present: true, train_can_disassemble: true, ..StationView::default() }
    }

    #[test]
    fn icon_widths_follow_the_bogey_spacing() {
        assert_eq!([icon_width(ENGINE), icon_width(FLIPPED_ENGINE), icon_width(2), icon_width(3), icon_width(8), icon_width(9)], [19, 19, 7, 13, 13, 19]);
        assert_eq!([icon_offset(FLIPPED_ENGINE), icon_offset(ENGINE), icon_offset(2), icon_offset(5), icon_offset(12)], [0, 62, 34, 20, 42]);
    }

    #[test]
    fn train_icon_width_counts_the_engine_and_the_carriages_after_the_first() {
        assert_eq!(train_icon_width(&[5], false), 19);
        assert_eq!(train_icon_width(&[5, 5], false), 19 + 13 + 1);
        assert_eq!(train_icon_width(&[5, 2, 9], false), 19 + 7 + 1 + 19 + 1);
        // The last carriage of a double ended train is an engine.
        assert_eq!(train_icon_width(&[5, 2, 9], true), 19 + 7 + 1 + 19 + 1);
        assert_eq!(train_icon_width(&[5, 5, 2], true), 19 + 13 + 1 + 19 + 1);
    }

    #[test]
    fn the_icon_strip_runs_from_the_last_carriage_to_the_engine() {
        let strip = icon_strip(&[5, 2, 9], false, 40.0, STATION_WIDTH);
        assert_eq!(strip.iter().map(|p| (p.length_or_engine, p.x)).collect::<Vec<_>>(), vec![(9, 0), (2, 20), (ENGINE, 28)]);
        // Backwards takes them from the front (`size - i - 1`), as Create does.
        let back = icon_strip(&[5, 2, 9], true, 40.0, STATION_WIDTH);
        assert_eq!(back.iter().map(|p| p.length_or_engine).collect::<Vec<_>>(), vec![5, 2, ENGINE]);
        // The ends fade: inside the window it is solid, near the edge it is not.
        assert_eq!(strip[0].alpha, 1.0);
        assert!(icon_strip(&[5, 5], false, -10.0, STATION_WIDTH)[0].alpha < 1.0);
    }

    #[test]
    fn the_speech_bubble_stays_inside_the_window() {
        assert_eq!(speech_x(0.0, 10), 25.0);
        assert_eq!(speech_x(150.0, 50), 159.0);
        assert_eq!(speech_x(60.0, 30), 77.0);
    }

    #[test]
    fn icons_are_found_by_id_or_word() {
        assert_eq!(icon_type("create:modern").y, 227);
        assert_eq!(icon_type("nothing").id, "create:traditional");
        assert_eq!(icon_index("create:electric"), 1);
        assert_eq!(icon_by_word("Modern").map(|i| i.id), Some("create:modern"));
        assert_eq!(icon_by_word("create:electric").map(|i| i.id), Some("create:electric"));
        assert_eq!(icon_by_word("steam"), None);
        assert_eq!(icon_at(-3).id, "create:traditional");
        assert_eq!(icon_at(9).id, "create:modern");
        assert_eq!(icon_step(1), -1);
        assert_eq!(icon_source(icon_type("create:electric"), ENGINE), (64, 216, 19, 10));
    }

    #[test]
    fn the_text_field_edits_like_an_edit_box() {
        let mut f = TextField::new(5);
        f.set("abcdefgh");
        assert_eq!(f.value(), "abcde");
        assert_eq!(f.cursor(), 5);
        assert!(!f.insert('x'));
        assert!(f.backspace());
        assert!(f.insert('Z'));
        assert_eq!(f.value(), "abcdZ");
        f.home();
        assert!(!f.backspace());
        assert!(f.delete());
        assert_eq!(f.value(), "bcdZ");
        f.right();
        assert!(f.insert('-'));
        assert_eq!(f.value(), "b-cdZ");
        assert!(!f.insert('\n'));
        assert!(f.key("key.keyboard.end"));
        assert_eq!(f.cursor(), 5);
        assert!(!f.key("key.keyboard.a"));
        f.focus(true);
        f.home();
        f.focus(false);
        assert_eq!(f.cursor(), 5);
    }

    #[test]
    fn a_click_into_the_box_selects_the_text_and_the_next_edit_replaces_it() {
        let mut f = TextField::new(10);
        f.set("Central");
        f.focus(true);
        f.select_all();
        assert!(f.is_selected());
        assert!(f.insert('X'));
        assert_eq!(f.value(), "X");
        f.set("Central");
        f.select_all();
        assert!(f.backspace());
        assert_eq!(f.value(), "");
        f.set("Central");
        f.select_all();
        f.left();
        assert!(!f.is_selected());
        assert_eq!(f.value(), "Central");
        // An empty box has nothing to select.
        let mut empty = TextField::new(3);
        empty.select_all();
        assert!(!empty.is_selected());
    }

    #[test]
    fn the_name_box_keeps_its_text_centred() {
        assert_eq!(name_box_x(10, 200, 40.0, 180), 10 + 100 - 25);
        // A long name is held to the width of the box.
        assert_eq!(name_box_x(10, 200, 400.0, 180), 10 + 100 - 95);
    }

    #[test]
    fn a_view_survives_the_wire() {
        let mut view = with_train(&[5, 2, 9]);
        view.name = "Central".into();
        view.assembling = true;
        view.door_control = DoorControl::West;
        view.bogey_count = 4;
        view.exception = Some(("No Bogeys".into(), 2));
        view.train.as_mut().unwrap().distance = 12.5;
        assert_eq!(StationView::decode(&view.encode()), Some(view));
        let idle = StationView::default();
        assert_eq!(StationView::decode(&idle.encode()), Some(idle));
        assert_eq!(StationView::decode("view"), None);
        assert_eq!(StationView::decode("x"), None);
    }

    #[test]
    fn messages_survive_the_wire() {
        for edit in [
            drop_schedule(),
            try_assemble(),
            try_disassemble(),
            configure(true, "Central", Some(DoorControl::North)),
            configure(false, "Hub", None),
        ] {
            let message = Message::Station { pos: [3, -4, 5], edit };
            assert_eq!(Message::decode(&message.encode()), Some(message));
        }
        let train = Message::Train(TrainEdit { id: "9".into(), name: "Fast".into(), icon: "create:modern".into(), map_color: 2 });
        assert_eq!(Message::decode(&train.encode()), Some(train));
        assert_eq!(Message::decode(&Message::Closed.encode()), Some(Message::Closed));
        assert_eq!(Message::decode("nothing"), None);
        assert_eq!(Message::decode("station\u{1f}1"), None);
    }

    #[test]
    fn the_packets_are_built_as_create_builds_them() {
        assert_eq!(drop_schedule(), StationEdit { drop_schedule: true, ..StationEdit::default() });
        assert_eq!(try_assemble().try_assemble, Some(true));
        assert_eq!(try_disassemble().try_assemble, Some(false));
        let c = configure(true, "A", Some(DoorControl::None));
        assert!(c.assembly_mode && !c.drop_schedule && c.try_assemble.is_none());
        assert_eq!(c.name.as_deref(), Some("A"));
    }

    #[test]
    fn a_train_edit_keeps_the_name_when_blank() {
        let (mut name, mut icon, mut color) = ("Old".to_owned(), "create:traditional".to_owned(), 0);
        apply_train_edit(&mut name, &mut icon, &mut color, &TrainEdit { id: "1".into(), name: "  ".into(), icon: "create:modern".into(), map_color: 5 });
        assert_eq!((name.as_str(), icon.as_str(), color), ("Old", "create:modern", 5));
        apply_train_edit(&mut name, &mut icon, &mut color, &TrainEdit { id: "1".into(), name: "New".into(), icon: "bogus".into(), map_color: 1 });
        assert_eq!((name.as_str(), icon.as_str(), color), ("New", "create:traditional", 1));
    }

    #[test]
    fn the_tooltip_names_the_reason_a_train_cant_be_built_or_taken_apart() {
        let mut view = StationView::default();
        assert_eq!(assembly_tooltip(&view), None);
        view.orthogonal = false;
        assert_eq!(assembly_tooltip(&view), Some("no_assembly_diagonal"));
        view.on_curve = true;
        assert_eq!(assembly_tooltip(&view), Some("no_assembly_curve"));
        view = StationView { train_present: true, train_can_disassemble: false, ..StationView::default() };
        assert_eq!(assembly_tooltip(&view), Some("train_not_aligned"));
        assert_eq!(StationScreen::assembly_tooltip_lines(&view, "disassemble_train"), vec!["Cannot disassemble,", "not all carriages aligned"]);
        assert_eq!(StationScreen::assembly_tooltip_lines(&StationView::default(), "create_train"), vec!["Create new Train"]);
        assert_eq!(StationScreen::drop_schedule_tooltip(&StationView::default()), None);
        let auto = StationView { train_has_schedule: true, train_has_auto_schedule: true, ..StationView::default() };
        assert_eq!(StationScreen::drop_schedule_tooltip(&auto), Some("remove_auto_schedule"));
        let manual = StationView { train_has_schedule: true, ..StationView::default() };
        assert_eq!(StationScreen::drop_schedule_tooltip(&manual), Some("remove_schedule"));
    }

    #[test]
    fn an_idle_station_offers_a_new_train_only_on_orthogonal_track() {
        let mut screen = StationScreen::new(DoorControl::All);
        let mut view = StationView::default();
        assert_eq!(screen.tick_train_display(&view), None);
        assert_eq!(screen.new_train, Button { active: true, visible: true });
        view.orthogonal = false;
        screen.tick_train_display(&view);
        assert_eq!(screen.new_train, Button { active: false, visible: true });
    }

    #[test]
    fn a_train_arriving_swaps_the_buttons_and_shows_its_name() {
        let mut screen = StationScreen::new(DoorControl::All);
        let mut view = with_train(&[5, 5]);
        view.train_has_schedule = true;
        assert_eq!(screen.tick_train_display(&view), Some(NameBox::Show("Express".into())));
        assert_eq!(screen.new_train, Button { active: false, visible: false });
        assert_eq!(screen.disassemble, Button { active: false, visible: true });
        assert_eq!(screen.drop_schedule, Button { active: true, visible: true });
        assert!(screen.train_name_active);
        // Standing at the station: at its place, and the next tick lets it be disassembled.
        let width = train_icon_width(&[5, 5], false);
        assert_eq!(screen.position, (STATION_WIDTH / 2 - width / 2) as f32);
        screen.tick_train_display(&view);
        assert!(screen.disassemble.active);
        // Misaligned carriages stop it.
        view.train_can_disassemble = false;
        screen.tick_train_display(&view);
        assert!(!screen.disassemble.active);
    }

    #[test]
    fn a_train_still_coming_is_drawn_off_to_the_left() {
        let mut screen = StationScreen::new(DoorControl::All);
        let mut view = with_train(&[5, 5]);
        view.train_present = false;
        view.train.as_mut().unwrap().distance = 15.0;
        screen.tick_train_display(&view);
        // distance / 15 = 1: `target - (target + 5)`.
        assert_eq!(screen.position, -5.0);
        view.train.as_mut().unwrap().distance = 0.0;
        screen.tick_train_display(&view);
        assert_eq!(screen.position, (STATION_WIDTH / 2 - train_icon_width(&[5, 5], false) / 2) as f32);
    }

    #[test]
    fn a_train_that_leaves_slides_out_for_eighty_ticks() {
        let mut screen = StationScreen::new(DoorControl::All);
        let view = with_train(&[5, 5]);
        screen.tick_train_display(&view);
        let gone = StationView::default();
        screen.tick_train_display(&gone);
        assert_eq!(screen.leaving, LEAVING_TICKS);
        let mut last = screen.position;
        for _ in 0..LEAVING_TICKS - 1 {
            screen.tick_train_display(&gone);
            assert!(screen.position >= last);
            last = screen.position;
        }
        assert!(screen.displayed.is_some());
        screen.tick_train_display(&gone);
        assert!(screen.displayed.is_none());
        assert!(!screen.disassemble.visible && !screen.drop_schedule.visible);
        // The name box goes on the next tick, and the new-train button comes back.
        assert_eq!(screen.tick_train_display(&gone), Some(NameBox::Hide));
        assert!(screen.new_train.visible);
    }

    #[test]
    fn a_different_train_makes_the_old_one_leave() {
        let mut screen = StationScreen::new(DoorControl::All);
        screen.tick_train_display(&with_train(&[5, 5]));
        let mut other = with_train(&[5, 5]);
        other.train.as_mut().unwrap().id = "8".into();
        screen.tick_train_display(&other);
        assert_eq!(screen.leaving, LEAVING_TICKS);
    }

    #[test]
    fn the_door_control_steps_and_wraps() {
        let mut screen = StationScreen::new(DoorControl::All);
        screen.click_door(false);
        assert_eq!(screen.door, DoorControl::North);
        screen.click_door(true);
        screen.click_door(true);
        assert_eq!(screen.door, DoorControl::None);
        screen.click_door(false);
        assert_eq!(screen.door, DoorControl::All);
        screen.step_door(-1);
        assert_eq!(screen.door, DoorControl::All);
        screen.step_door(2);
        assert_eq!(screen.door, DoorControl::East);
    }

    #[test]
    fn closing_sends_the_name_the_door_and_the_train() {
        let mut screen = StationScreen::new(DoorControl::South);
        screen.tick_train_display(&with_train(&[5, 5]));
        let sent = screen.closing("Central", "Express 2", [1, 2, 3]);
        assert_eq!(
            sent,
            vec![
                Message::Station { pos: [1, 2, 3], edit: configure(false, "Central", Some(DoorControl::South)) },
                Message::Train(TrainEdit { id: "7".into(), name: "Express 2".into(), icon: "create:electric".into(), map_color: 3 }),
            ]
        );
        // Going on to the assembly screen asks to enter assembly mode and leaves the train alone.
        screen.switching = true;
        let sent = screen.closing("Central", "Express 2", [1, 2, 3]);
        assert_eq!(sent, vec![Message::Station { pos: [1, 2, 3], edit: configure(true, "Central", Some(DoorControl::South)) }]);
    }

    #[test]
    fn names_are_sent_only_when_they_changed() {
        let mut screen = StationScreen::new(DoorControl::All);
        assert_eq!(screen.sync_train("x"), None);
        screen.tick_train_display(&with_train(&[5, 5]));
        assert_eq!(screen.sync_train("Express"), None);
        assert!(screen.sync_train("Slow").is_some());
        assert_eq!(screen.sync_station("A", "A", [0; 3]), None);
        assert_eq!(screen.sync_station("B", "A", [0; 3]), Some(Message::Station { pos: [0; 3], edit: configure(false, "B", Some(DoorControl::All)) }));
    }

    #[test]
    fn the_assembly_screen_goes_back_when_a_train_appears() {
        let mut screen = AssemblyScreen::default();
        let mut view = StationView { bogey_count: 2, assembling: true, ..StationView::default() };
        assert_eq!(screen.tick(&view), AssemblyTick::Stay);
        assert!(screen.toggle_active);
        view.bogey_count = 0;
        screen.tick(&view);
        assert!(!screen.toggle_active);
        view.train = Some(train(&[5, 5]));
        assert_eq!(screen.tick(&view), AssemblyTick::BackToStation);
    }

    #[test]
    fn bogeys_are_counted_in_words() {
        assert_eq!(AssemblyScreen::bogey_text(0), "No Bogeys");
        assert_eq!(AssemblyScreen::bogey_text(1), "1 Bogey");
        assert_eq!(AssemblyScreen::bogey_text(4), "4 Bogeys");
    }

    #[test]
    fn words_are_wrapped_greedily() {
        let measure = |s: &str| s.chars().count() as f32;
        assert_eq!(wrap("aa bb cc dd", 5.0, &measure), vec!["aa bb", "cc dd"]);
        assert_eq!(wrap("toolongword x", 4.0, &measure), vec!["toolongword", "x"]);
        assert_eq!(wrap("", 4.0, &measure), vec![String::new()]);
    }

    #[test]
    fn the_assembly_body_shows_the_failure_below_the_title() {
        let measure = |s: &str| s.chars().count() as f32 * 6.0;
        let body = assembly_body(&StationView { bogey_count: 3, ..StationView::default() }, &measure);
        assert_eq!(body[0].text, "3 Bogeys");
        assert_eq!(body.len(), 4);
        let failed = StationView { exception: Some(("Carriages too long to fit".into(), 1)), ..StationView::default() };
        let body = assembly_body(&failed, &measure);
        assert_eq!(body[0].text, "Assembly Failed");
        assert_eq!(body[1].text, "Carriage 1:");
        assert_eq!((body[2].y, body[2].color), (77, 0x77_5B5B));
        // 134 pixels is 22 characters: the message takes two lines, and the retry text follows them.
        assert_eq!(body[3].y, 77 + 2 * 9 + 5);
        let first = assembly_body(&StationView { exception: Some(("Bad".into(), -1)), ..StationView::default() }, &measure);
        assert_eq!(first[1].y, 67);
    }

    #[test]
    fn rectangles_hit_inside_the_window() {
        let r = station_layout::NEW_TRAIN;
        assert!(r.contains(100, 50, [100.0 + 84.0, 50.0 + 65.0]));
        assert!(!r.contains(100, 50, [100.0 + 84.0 + 26.0, 50.0 + 65.0]));
        assert_eq!(assembly_layout::TOGGLE.y, ASSEMBLY_HEIGHT - 24);
    }
}

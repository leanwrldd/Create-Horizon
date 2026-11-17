//! Create's value settings board: the bar you hold right-click to open on a value box and slide along
//! (`ValueSettingsBoard`, `ValueSettingsScreen`, `ValueSettingsFormatter`), and what each kind of box does with
//! the value you let go on (`ScrollValueBehaviour`, `ScrollOptionBehaviour`, `KineticScrollValueBehaviour`,
//! `FilteringBehaviour.setValueSettings`). Plain numbers and no game types: the screen
//! (`native/api/src/machines/value_box_screen.rs`) reads the pointer, draws what `Layout` says and sends the
//! `Settings` it ends on.
//!
//! Known differences from Create, for the README:
//! - The bar is drawn with flat rectangles, not Create's brass-framed textures (they are read from Create's own
//!   jar in Create and this port doesn't import that texture sheet), and an option's icon is its name.
//! - The pointer isn't warped to the current value when the board opens (Horizon has no call to move the
//!   cursor): the bar starts where the pointer is, which is where the click was, and the current value is
//!   marked with a tick on the bar.

/// `ValueSettingsBehaviour.ValueSettings`: the row (the bar of an up/down pair, the mode of a filter's amount)
/// and the value along it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub row: i32,
    pub value: i32,
}

/// What a box's value is, which says what its board looks like and how a settings change is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// `ScrollValueBehaviour`: one bar from 0 to `max`, every ten marked (`createBoard`: `max`, 10, "Value").
    Scroll { min: i32, max: i32, default: i32 },
    /// `KineticScrollValueBehaviour` (Creative Motor, Rotation Speed Controller): two bars of 256, the top
    /// clockwise, every 32 marked; the value is signed (`createBoard`: 256, 32, rows "⟳" and "⟲").
    Kinetic { min: i32, max: i32, default: i32 },
    /// `ScrollOptionBehaviour`: a pick from `options` (language keys), each marked (`createBoard`: `max`, 1,
    /// "Select").
    Options { options: &'static [&'static str] },
    /// `FilteringBehaviour` with a count (`showCount`): "Up to" and "Exactly" bars of the filter's stack size,
    /// every 16 marked.
    Amount,
}

/// What a board is made of before it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub max_value: i32,
    pub milestone_interval: i32,
    /// The row labels, in order (Create's are literal "Value", "Select" and the arrows, or translated).
    pub rows: Vec<String>,
    /// `ScrollOptionSettingsFormatter`: the board is a row of option icons rather than a bar.
    pub icon_mode: bool,
}

/// `ValueSettingsBehaviour.createBoard`. `max_stack` is the filter's item stack size (64 with none), for an
/// `Amount` board; `translate` words a language key.
pub fn board(behaviour: Behaviour, max_stack: i32, translate: &dyn Fn(&str) -> String) -> Board {
    match behaviour {
        Behaviour::Scroll { max, .. } => Board {
            max_value: max,
            milestone_interval: 10,
            rows: vec!["Value".to_owned()],
            icon_mode: false,
        },
        // `KineticScrollValueBehaviour.createBoard` always uses 256, whatever the box's own range.
        Behaviour::Kinetic { .. } => Board {
            max_value: 256,
            milestone_interval: 32,
            rows: vec!["\u{27f3}".to_owned(), "\u{27f2}".to_owned()],
            icon_mode: false,
        },
        Behaviour::Options { options } => Board {
            max_value: options.len() as i32 - 1,
            milestone_interval: 1,
            rows: vec!["Select".to_owned()],
            icon_mode: true,
        },
        Behaviour::Amount => Board {
            max_value: max_stack,
            milestone_interval: 16,
            rows: vec![translate("create.logistics.filter.up_to"), translate("create.logistics.filter.exactly")],
            icon_mode: false,
        },
    }
}

/// The stored numbers a box's settings come from: the scroll value, or a filter's amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Values {
    /// `ScrollValue`.
    pub scroll: i32,
    /// `FilterAmount`.
    pub count: i32,
    /// `UpTo`.
    pub up_to: bool,
    /// The filter item's stack size (64 with none): `FilteringBehaviour.getMaxStackSize`.
    pub max_stack: i32,
}

impl Default for Values {
    fn default() -> Self {
        Self { scroll: 0, count: 64, up_to: true, max_stack: 64 }
    }
}

/// `ValueSettingsBehaviour.getValueSettings`.
pub fn settings_of(behaviour: Behaviour, v: &Values) -> Settings {
    match behaviour {
        Behaviour::Scroll { .. } | Behaviour::Options { .. } => Settings { row: 0, value: v.scroll },
        // `KineticScrollValueBehaviour.getValueSettings`: the row is 0 for a negative (clockwise) value.
        Behaviour::Kinetic { .. } => Settings { row: i32::from(v.scroll >= 0), value: v.scroll.abs() },
        // `FilteringBehaviour.getValueSettings`: an amount of 0 means a full stack.
        Behaviour::Amount => Settings {
            row: i32::from(!v.up_to),
            value: if v.count == 0 { v.max_stack } else { v.count },
        },
    }
}

/// What `setValueSettings` makes of `settings`, `None` when the box already has them (it plays no sound and
/// saves nothing then). The scroll value goes through `ScrollValueBehaviour.setValue`'s clamp to
/// `min..=max`.
pub fn apply(behaviour: Behaviour, v: &Values, settings: Settings) -> Option<Values> {
    if settings == settings_of(behaviour, v) {
        return None;
    }
    let mut next = *v;
    match behaviour {
        Behaviour::Scroll { min, max, .. } | Behaviour::Kinetic { min, max, .. } => {
            let value = if matches!(behaviour, Behaviour::Kinetic { .. }) {
                let value = settings.value.max(1);
                if settings.row == 0 { -value } else { value }
            } else {
                settings.value
            };
            next.scroll = value.clamp(min, max);
        }
        Behaviour::Options { options } => next.scroll = settings.value.clamp(0, options.len() as i32 - 1),
        Behaviour::Amount => {
            next.count = settings.value.clamp(1, v.max_stack.max(1));
            next.up_to = settings.row == 0;
        }
    }
    (next != *v).then_some(next)
}

/// `FilteringBehaviour.getCountLabelForValueBox`: what the filter's box shows next to its item (nothing when
/// the count isn't shown, `*` for an up-to of a full stack).
pub fn count_label(shown: bool, v: &Values) -> String {
    if !shown {
        return String::new();
    }
    if v.up_to && v.max_stack == v.count {
        "*".to_owned()
    } else {
        v.count.to_string()
    }
}

/// `ValueSettingsFormatter.format` of `settings`: the text on the board's cursor. `translate` words a key.
pub fn format(behaviour: Behaviour, max_stack: i32, settings: Settings, translate: &dyn Fn(&str) -> String) -> String {
    match behaviour {
        Behaviour::Scroll { .. } => settings.value.to_string(),
        Behaviour::Kinetic { .. } => {
            let arrow = if settings.row == 0 { "\u{27f3}" } else { "\u{27f2}" };
            format!("{}{arrow}", settings.value.max(1))
        }
        Behaviour::Options { options } => {
            let key = options.get(settings.value.max(0) as usize).copied().unwrap_or("");
            translate(key)
        }
        Behaviour::Amount => {
            if settings.row == 0 && settings.value == max_stack {
                return translate("create.logistics.filter.any_amount_short");
            }
            let sign = if settings.row == 0 { "\u{2264}" } else { "=" };
            format!("{sign}{}", settings.value.max(1))
        }
    }
}

/// What the box shows on itself while the block is looked at (`ScrollValueRenderer`'s `TextValueBox`):
/// `KineticScrollValueBehaviour` shows the speed without its sign (`withFormatter(v -> abs)`).
pub fn box_text(behaviour: Behaviour, v: &Values) -> String {
    match behaviour {
        Behaviour::Kinetic { .. } => v.scroll.abs().to_string(),
        _ => v.scroll.to_string(),
    }
}

// --- the screen's layout -------------------------------------------------------------------------------

/// Where the parts of the board are on the screen, in GUI pixels (`ValueSettingsScreen.init` and
/// `getCoordinateOfValue`).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub board: Board,
    pub gui_left: i32,
    pub gui_top: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub max_label_width: i32,
    pub value_bar_width: i32,
    pub milestone_size: i32,
    /// Pixels of the bar for each value: 2, or 1 when it has more than 128.
    pub scale: i32,
}

impl Layout {
    /// `ValueSettingsScreen.init` on a screen of `screen` pixels; `label_width` is the widest row label in
    /// font pixels.
    pub fn new(board: Board, label_width: i32, screen: [i32; 2]) -> Self {
        let max_value = board.max_value;
        let milestone_size = if board.icon_mode { 8 } else { 4 };
        let milestone_count = max_value / board.milestone_interval + 1;
        let scale = if max_value > 128 { 1 } else { 2 };
        let max_label_width = if board.icon_mode { -18 } else { label_width };
        let value_bar_width = (max_value + 1) * scale + 1 + milestone_count * milestone_size;
        let window_width = (max_label_width + 14) + (value_bar_width + 10);
        let window_height = board.rows.len() as i32 * 11;
        Self {
            gui_left: (screen[0] - window_width) / 2,
            gui_top: (screen[1] - window_height) / 2,
            window_width,
            window_height,
            max_label_width,
            value_bar_width,
            milestone_size,
            scale,
            board,
        }
    }

    /// `getCoordinateOfValue`: the centre of the cursor for `column` of `row`.
    pub fn coordinate_of(&self, row: i32, column: i32) -> [f32; 2] {
        let interval = self.board.milestone_interval;
        let mut x = self.gui_left as f32
            + (((column.max(1) - 1) / interval) * self.milestone_size) as f32
            + (column * self.scale) as f32
            + 1.5;
        x += (self.max_label_width + 14 + 4) as f32;
        if column % interval == 0 {
            x += (self.milestone_size / 2) as f32;
        }
        if column > 0 {
            x += self.milestone_size as f32;
        }
        let y = self.gui_top as f32 + (row as f32 + 0.5) * 11.0 - 0.5;
        [x, y]
    }

    /// `getClosestCoordinate`: the setting nearest the pointer. With `milestones_only` (Shift held) only the
    /// marked values count.
    pub fn closest(&self, mouse: [f32; 2], milestones_only: bool) -> Settings {
        let interval = self.board.milestone_interval;
        let rows = self.board.rows.len() as i32;
        let mut row = 0;
        let mut best = f32::MAX;
        while row < rows {
            let diff = (self.coordinate_of(row, 0)[1] - mouse[1]).abs();
            if best < diff {
                break;
            }
            best = diff;
            row += 1;
        }
        row -= 1;
        let row = row.max(0);

        let mut column = 0;
        best = f32::MAX;
        while column <= self.board.max_value {
            let at = if milestones_only { column * interval } else { column };
            let diff = (self.coordinate_of(row, at)[0] - mouse[0]).abs();
            if best < diff {
                break;
            }
            best = diff;
            column += 1;
        }
        column -= 1;
        let column = column.max(0);
        Settings {
            row,
            value: if milestones_only { (column * interval).min(self.board.max_value) } else { column },
        }
    }

    /// `ValueSettingsScreen.mouseScrolled`: the value one wheel step (or one milestone with Shift) from `from`.
    pub fn scrolled(&self, from: Settings, steps: f64, milestones: bool) -> Settings {
        let step = if milestones { self.board.milestone_interval } else { 1 };
        let value = (from.value + steps.signum() as i32 * step).clamp(0, self.board.max_value);
        Settings { row: from.row, value }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(key: &str) -> String {
        match key {
            "create.logistics.filter.up_to" => "Up to".into(),
            "create.logistics.filter.exactly" => "Exactly".into(),
            "create.logistics.filter.any_amount_short" => "Any".into(),
            other => other.rsplit('.').next().unwrap_or(other).to_owned(),
        }
    }

    const MODES: &[&str] = &["create.x.split", "create.x.forced_split", "create.x.round_robin"];

    #[test]
    fn a_scroll_value_board_runs_zero_to_max_with_every_tenth_marked() {
        let b = board(Behaviour::Scroll { min: 0, max: 64, default: 0 }, 64, &words);
        assert_eq!((b.max_value, b.milestone_interval, b.icon_mode), (64, 10, false));
        assert_eq!(b.rows, vec!["Value"]);
    }

    #[test]
    fn a_motor_board_is_two_bars_of_256_with_arrows() {
        let b = board(Behaviour::Kinetic { min: -256, max: 256, default: 16 }, 64, &words);
        assert_eq!((b.max_value, b.milestone_interval), (256, 32));
        assert_eq!(b.rows.len(), 2);
        let layout = Layout::new(b, 10, [320, 180]);
        // 256 > 128: one pixel a value.
        assert_eq!(layout.scale, 1);
    }

    #[test]
    fn an_option_board_is_a_row_of_icons() {
        let b = board(Behaviour::Options { options: MODES }, 64, &words);
        assert_eq!((b.max_value, b.milestone_interval, b.icon_mode), (2, 1, true));
        let layout = Layout::new(b, 40, [320, 180]);
        assert_eq!((layout.max_label_width, layout.milestone_size), (-18, 8));
    }

    #[test]
    fn a_filter_amount_board_has_up_to_and_exactly_in_stack_sizes() {
        let b = board(Behaviour::Amount, 16, &words);
        assert_eq!((b.max_value, b.milestone_interval), (16, 16));
        assert_eq!(b.rows, vec!["Up to", "Exactly"]);
    }

    #[test]
    fn the_motor_sign_is_the_row() {
        let b = Behaviour::Kinetic { min: -256, max: 256, default: 16 };
        let v = |scroll| Values { scroll, ..Values::default() };
        assert_eq!(settings_of(b, &v(16)), Settings { row: 1, value: 16 });
        assert_eq!(settings_of(b, &v(-16)), Settings { row: 0, value: 16 });
        // Row 0 is clockwise: negative.
        assert_eq!(apply(b, &v(16), Settings { row: 0, value: 32 }).unwrap().scroll, -32);
        assert_eq!(apply(b, &v(16), Settings { row: 1, value: 64 }).unwrap().scroll, 64);
        // Zero can't be set: `Math.max(1, value)`.
        assert_eq!(apply(b, &v(16), Settings { row: 1, value: 0 }).unwrap().scroll, 1);
        // Nothing changes: nothing is saved.
        assert_eq!(apply(b, &v(16), Settings { row: 1, value: 16 }), None);
    }

    #[test]
    fn a_scroll_value_is_clamped_to_its_range() {
        let b = Behaviour::Scroll { min: 1, max: 16, default: 5 };
        let v = Values { scroll: 5, ..Values::default() };
        assert_eq!(apply(b, &v, Settings { row: 0, value: 0 }).unwrap().scroll, 1);
        assert_eq!(apply(b, &v, Settings { row: 0, value: 9 }).unwrap().scroll, 9);
        assert_eq!(apply(b, &v, Settings { row: 0, value: 99 }).unwrap().scroll, 16);
        assert_eq!(apply(b, &v, Settings { row: 0, value: 5 }), None);
    }

    #[test]
    fn an_option_is_one_of_the_list() {
        let b = Behaviour::Options { options: MODES };
        let v = Values { scroll: 0, ..Values::default() };
        assert_eq!(apply(b, &v, Settings { row: 0, value: 2 }).unwrap().scroll, 2);
        assert_eq!(apply(b, &v, Settings { row: 0, value: 7 }).unwrap().scroll, 2);
        assert_eq!(format(b, 64, Settings { row: 0, value: 1 }, &words), "forced_split");
    }

    #[test]
    fn a_filter_amount_is_one_to_the_stack_size() {
        let b = Behaviour::Amount;
        let v = Values { count: 64, up_to: true, max_stack: 16, scroll: 0 };
        assert_eq!(settings_of(b, &v), Settings { row: 0, value: 64 });
        let exact = apply(b, &v, Settings { row: 1, value: 5 }).unwrap();
        assert_eq!((exact.count, exact.up_to), (5, false));
        // `Mth.clamp(value, 1, getMaxStackSize())`.
        assert_eq!(apply(b, &v, Settings { row: 0, value: 0 }).unwrap().count, 1);
        assert_eq!(apply(b, &v, Settings { row: 0, value: 40 }).unwrap().count, 16);
        // An amount of 0 (an old save) reads as a full stack.
        let old = Values { count: 0, ..v };
        assert_eq!(settings_of(b, &old), Settings { row: 0, value: 16 });
    }

    #[test]
    fn the_amount_is_worded_as_create_does() {
        let b = Behaviour::Amount;
        assert_eq!(format(b, 64, Settings { row: 0, value: 64 }, &words), "Any");
        assert_eq!(format(b, 64, Settings { row: 0, value: 12 }, &words), "\u{2264}12");
        assert_eq!(format(b, 64, Settings { row: 1, value: 12 }, &words), "=12");
        assert_eq!(format(b, 64, Settings { row: 1, value: 0 }, &words), "=1");
        let motor = Behaviour::Kinetic { min: -256, max: 256, default: 16 };
        assert_eq!(format(motor, 64, Settings { row: 0, value: 48 }, &words), "48\u{27f3}");
        assert_eq!(format(motor, 64, Settings { row: 1, value: 0 }, &words), "1\u{27f2}");
        assert_eq!(box_text(motor, &Values { scroll: -48, ..Values::default() }), "48");
    }

    #[test]
    fn the_count_label_is_a_star_for_any_amount() {
        let v = Values { count: 64, up_to: true, max_stack: 64, scroll: 0 };
        assert_eq!(count_label(true, &v), "*");
        assert_eq!(count_label(true, &Values { count: 12, ..v }), "12");
        assert_eq!(count_label(true, &Values { up_to: false, ..v }), "64");
        assert_eq!(count_label(false, &v), "");
    }

    #[test]
    fn the_layout_is_createss() {
        // A scroll board of 0 to 64 with the label "Value" (width 26) on a 320 x 180 screen.
        let b = board(Behaviour::Scroll { min: 0, max: 64, default: 0 }, 64, &words);
        let l = Layout::new(b, 26, [320, 180]);
        // 7 milestones (0, 10 .. 60) of 4 pixels, 65 values of 2: 130 + 1 + 28 = 159.
        assert_eq!(l.value_bar_width, 159);
        assert_eq!(l.window_width, 26 + 14 + 159 + 10);
        assert_eq!(l.window_height, 11);
        assert_eq!(l.gui_left, (320 - 209) / 2);
        assert_eq!(l.gui_top, (180 - 11) / 2);
        // The cursor for 0 is on the first milestone: left + 1.5 + (26 + 14 + 4) + (4 / 2).
        let [x, y] = l.coordinate_of(0, 0);
        assert_eq!(x, l.gui_left as f32 + 1.5 + 44.0 + 2.0);
        assert_eq!(y, l.gui_top as f32 + 5.0);
        // Value 1 sits past that milestone: + 4 (milestone size), 2 per value.
        let [x1, _] = l.coordinate_of(0, 1);
        assert_eq!(x1, l.gui_left as f32 + 0.0 + 2.0 + 1.5 + 44.0 + 4.0);
    }

    #[test]
    fn the_pointer_picks_the_nearest_value() {
        let b = board(Behaviour::Scroll { min: 0, max: 64, default: 0 }, 64, &words);
        let l = Layout::new(b, 26, [320, 180]);
        for value in [0, 1, 9, 10, 11, 37, 64] {
            let [x, y] = l.coordinate_of(0, value);
            assert_eq!(l.closest([x, y], false), Settings { row: 0, value }, "value {value}");
            // A pixel either side still rounds to it.
            assert_eq!(l.closest([x + 0.4, y + 3.0], false).value, value);
        }
        // Far to the right is the maximum, far to the left zero.
        assert_eq!(l.closest([1000.0, 90.0], false).value, 64);
        assert_eq!(l.closest([-10.0, 90.0], false).value, 0);
    }

    #[test]
    fn two_rows_are_picked_by_height() {
        let b = board(Behaviour::Kinetic { min: -256, max: 256, default: 16 }, 64, &words);
        let l = Layout::new(b, 8, [640, 360]);
        let [x, y0] = l.coordinate_of(0, 48);
        let [_, y1] = l.coordinate_of(1, 48);
        assert_eq!(l.closest([x, y0], false), Settings { row: 0, value: 48 });
        assert_eq!(l.closest([x, y1], false), Settings { row: 1, value: 48 });
        assert_eq!(l.closest([x, y1 + 40.0], false).row, 1);
        assert_eq!(l.closest([x, y0 - 40.0], false).row, 0);
    }

    #[test]
    fn shift_snaps_to_the_marked_values() {
        let b = board(Behaviour::Scroll { min: 0, max: 64, default: 0 }, 64, &words);
        let l = Layout::new(b, 26, [320, 180]);
        let [x, y] = l.coordinate_of(0, 23);
        // Nearest marked values are 20 and 30: 23 is closer to 20.
        assert_eq!(l.closest([x, y], true).value, 20);
        let [x, y] = l.coordinate_of(0, 27);
        assert_eq!(l.closest([x, y], true).value, 30);
        // Past the end it stops at the maximum, not the next milestone.
        assert_eq!(l.closest([1000.0, y], true).value, 64);
    }

    #[test]
    fn the_wheel_steps_by_one_or_a_milestone() {
        let b = board(Behaviour::Scroll { min: 0, max: 64, default: 0 }, 64, &words);
        let l = Layout::new(b, 26, [320, 180]);
        let at = Settings { row: 0, value: 30 };
        assert_eq!(l.scrolled(at, 1.0, false).value, 31);
        assert_eq!(l.scrolled(at, -1.0, false).value, 29);
        assert_eq!(l.scrolled(at, 1.0, true).value, 40);
        assert_eq!(l.scrolled(Settings { row: 0, value: 64 }, 1.0, false).value, 64);
        assert_eq!(l.scrolled(Settings { row: 0, value: 3 }, -1.0, true).value, 0);
    }
}

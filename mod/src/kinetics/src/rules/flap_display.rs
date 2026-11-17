//! The Display Board's rules (`FlapDisplayBlock`, `FlapDisplayBlockEntity`, `FlapDisplayLayout`,
//! `FlapDisplaySection`, `FlapDisplayRenderer`, `DisplayBoardTarget`): how boards join into one,
//! how a line of text is cut into sections of flaps, how a display link fills them and how the
//! flaps spin before they settle.
//!
//! Known differences from Create, for the README:
//! - The flap animation is a pure function of the game time since the text changed (Create ticks
//!   random state on each client): the same rules (`FlapDisplaySection.tick`) are run again from the
//!   start with a random generator seeded from the board, so every client and the server agree.
//! - Create's `"number"` flap cycle names no language key (it would show the key's text); it is
//!   treated as the numeric cycle here.
//! - A section of one flap (the fluid units) spins by dimming, not by the `-` and `_` frames.

use crate::Pos;

/// `FlapDisplaySection.MONOSPACE` and `WIDE_MONOSPACE`: font units a flap takes.
pub const MONOSPACE: f32 = 7.0;
pub const WIDE_MONOSPACE: f32 = 9.0;
/// `kinetics.mediumSpeed` (`SpeedLevel.MEDIUM`), the speed a board needs to take text.
pub const MIN_SPEED: f32 = 30.0;
/// `FlapDisplayBlockEntity.setLazyTickRate(10)`: how often a board looks at its neighbours.
pub const LAZY_TICKS: i64 = 10;
/// A line's colour when no dye was used (`getLineColor`).
pub const DEFAULT_COLOUR: u32 = 0xD3_C6BA;

/// `FlapDisplayBlockEntity.isSpeedRequirementFulfilled` (`getMinimumRequiredSpeedLevel` is MEDIUM).
pub fn speed_fulfilled(rpm: f32) -> bool {
    rpm.abs() >= MIN_SPEED
}

/// `FlapDisplaySection.getFlapCycle`: the characters a flap shows while it spins, from
/// `flap_display.cycles.<key>` in Create's language file.
pub fn cycle_options(cycle: &str) -> Vec<String> {
    let list = match cycle {
        "alphabet" => " ;A;B;C;D;E;F;G;H;I;J;K;L;M;N;O;P;Q;R;S;T;U;V;X;Y;Z",
        "arrival_time" => "   ; min;now;15s;30s;45s",
        "fluid_units" => "mB;B ",
        "instant" => " ; ",
        "numeric" | "number" => " ;0;1;2;3;4;5;6;7;8;9",
        "pixel" => "\u{2588};\u{2593};\u{2592}",
        "shortened_numbers" => " ;K;M",
        other => other,
    };
    list.split(';').map(str::to_owned).collect()
}

/// `FlapDisplaySection`, with the text already cut to the flaps (`text`) and what it was before
/// the last change (`prev`, `since`) in place of Create's client-side state.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub size: f32,
    pub cycle: String,
    pub single_flap: bool,
    pub gap: bool,
    pub right_aligned: bool,
    pub wide: bool,
    /// The text last given (`component`), before it is cut.
    pub component: Option<String>,
    /// What the flaps show now, as many characters as flaps (or one for a single flap).
    pub text: String,
    /// What they showed before the last change.
    pub prev: String,
    /// The game time of the last change that moved a flap.
    pub since: Option<i64>,
}

impl Section {
    pub fn new(width: f32, cycle: &str, single_flap: bool, gap: bool) -> Self {
        let mut section = Section {
            size: width,
            cycle: cycle.to_owned(),
            single_flap,
            gap,
            right_aligned: false,
            wide: false,
            component: None,
            text: String::new(),
            prev: String::new(),
            since: None,
        };
        section.text = " ".repeat(section.flaps());
        section.prev = section.text.clone();
        section
    }

    pub fn right_aligned(mut self) -> Self {
        self.right_aligned = true;
        self
    }

    pub fn wide_flaps(mut self) -> Self {
        self.wide = true;
        self
    }

    /// How many flaps (`spinning.length`).
    pub fn flaps(&self) -> usize {
        if self.single_flap { 1 } else { (self.size / MONOSPACE).max(0.0) as usize }
    }

    /// `setText` and `refresh(true)`: the text a display link or a name tag gave, cut to the flaps
    /// (`FlapDisplaySection.refresh`), starting the spin of the flaps that change.
    pub fn set_text(&mut self, text: &str, now: i64) {
        self.component = Some(text.to_owned());
        let flaps = self.flaps();
        let new = if self.single_flap {
            text.to_owned()
        } else {
            let mut cut = if self.right_aligned { text.trim().to_owned() } else { text.to_owned() };
            cut = cut.to_uppercase();
            let kept: String = cut.chars().take(flaps).collect();
            let whitespace = " ".repeat(flaps - kept.chars().count());
            if self.right_aligned { whitespace + &kept } else { kept + &whitespace }
        };
        if new != self.text {
            self.prev = std::mem::replace(&mut self.text, new);
            self.since = Some(now);
        }
    }

    /// Which flaps move after the last change: those whose character differs.
    pub fn changed(&self) -> Vec<bool> {
        if self.single_flap {
            return vec![self.prev != self.text];
        }
        let (prev, text): (Vec<char>, Vec<char>) = (self.prev.chars().collect(), self.text.chars().collect());
        (0..self.flaps()).map(|i| prev.get(i) != text.get(i)).collect()
    }

    pub fn options(&self) -> Vec<String> {
        cycle_options(&self.cycle)
    }

    /// The width in font units this section takes in a line (`size` and its gap).
    pub fn advance(&self) -> f32 {
        self.size + if self.gap { 8.0 } else { 1.0 }
    }
}

/// `FlapDisplayLayout`: the sections of one line.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub key: String,
    pub sections: Vec<Section>,
}

impl Layout {
    /// `new FlapDisplayLayout(maxCharCount)`.
    pub fn new(max_chars: i32) -> Self {
        let mut layout = Layout { key: String::new(), sections: Vec::new() };
        layout.load_default(max_chars);
        layout
    }

    pub fn load_default(&mut self, max_chars: i32) {
        self.configure(
            "Default",
            vec![Section::new(max_chars.max(0) as f32 * MONOSPACE, "alphabet", false, false)],
        );
    }

    pub fn is_layout(&self, key: &str) -> bool {
        self.key == key
    }

    pub fn configure(&mut self, key: &str, sections: Vec<Section>) {
        self.key = key.to_owned();
        self.sections = sections;
    }

    /// The width of the line (`FlapDisplayRenderer`'s `w`).
    pub fn width(&self) -> f32 {
        self.sections.iter().map(Section::advance).sum()
    }
}

/// What a controller block entity keeps about its board.
#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub x_size: i32,
    pub y_size: i32,
    pub lines: Vec<Layout>,
    /// The dye of each line (`colour`), `None` for white.
    pub colour: Vec<Option<String>>,
    pub glowing: Vec<bool>,
    /// Lines set by hand (`manualLines`): a link leaves them alone.
    pub manual: Vec<bool>,
    /// The display link that wrote each line first (`DisplayTarget.reserve`).
    pub reserved: Vec<Option<Pos>>,
}

impl Board {
    pub fn new(x_size: i32, y_size: i32) -> Self {
        let rows = (y_size * 2).max(0) as usize;
        Board {
            x_size,
            y_size,
            lines: (0..rows).map(|_| Layout::new(max_char_count(x_size, 0))).collect(),
            colour: vec![None; rows],
            glowing: vec![false; rows],
            manual: vec![false; rows],
            reserved: vec![None; rows],
        }
    }

    pub fn max_char_count(&self, gaps: i32) -> i32 {
        max_char_count(self.x_size, gaps)
    }

    /// `applyTextManually`: a name tag's text on a line, or `None` to clear it.
    pub fn apply_text_manually(&mut self, line: usize, text: Option<&str>, now: i64) {
        if line >= self.lines.len() {
            return;
        }
        let max = self.max_char_count(0);
        let layout = &mut self.lines[line];
        if !layout.is_layout("Default") {
            layout.load_default(max);
        }
        let Some(section) = layout.sections.first_mut() else {
            return;
        };
        match text {
            None => {
                self.manual[line] = false;
                section.set_text("", now);
            }
            Some(text) => {
                self.manual[line] = true;
                section.set_text(text, now);
            }
        }
    }

    /// `DisplayTarget.reserve`: the first line a link writes is its own.
    pub fn reserve(&mut self, line: usize, link: Pos) {
        if line == 0 {
            return;
        }
        if let Some(slot) = self.reserved.get_mut(line) {
            *slot = Some(link);
        }
    }

    /// `DisplayBoardTarget.isReserved`: another link that is still there wrote the line, or it was
    /// set by hand. A reservation whose link is gone is dropped.
    pub fn is_reserved(&mut self, line: usize, link: Pos, link_exists: &dyn Fn(Pos) -> bool) -> bool {
        let by_link = match self.reserved.get(line).copied().flatten() {
            Some(other) if other != link && link_exists(other) => true,
            Some(_) => {
                self.reserved[line] = None;
                false
            }
            None => false,
        };
        by_link || self.manual.get(line).copied().unwrap_or(false)
    }
}

/// `FlapDisplayBlockEntity.getMaxCharCount`.
pub fn max_char_count(x_size: i32, gaps: i32) -> i32 {
    ((x_size as f32 * 16.0 - 2.0 - 4.0 * gaps as f32) / 3.5) as i32
}

/// `FlapDisplayBlockEntity.getLineIndexAt`: the line of a board whose top block is at height
/// `block_y` that a click at height `y` is on, clamped as Create does.
pub fn line_index_at(block_y: i32, y: f64, y_size: i32) -> usize {
    (2.0 * (f64::from(block_y) - y + 1.0)).floor().clamp(0.0, f64::from(y_size * 2)) as usize
}

/// `DisplayBoardTarget.acceptFlapText`: writes the rows `text` (each a list of the texts of a
/// line's sections) from `line` down, stopping at a line another link owns or one set by hand.
/// `load_layout` is the source's `loadFlapDisplayLayout` for row `i`; `single_line` is whether the
/// source is a `SingleLineDisplaySource` (it leaves the lines below alone).
pub fn accept_flap_text(
    board: &mut Board,
    line: usize,
    text: &[Vec<String>],
    single_line: bool,
    link: Pos,
    link_exists: &dyn Fn(Pos) -> bool,
    now: i64,
    load_layout: &dyn Fn(&mut Layout, usize),
) {
    let mut i = 0;
    while i + line < board.lines.len() {
        let row = i + line;
        if i == 0 {
            board.reserve(row, link);
        }
        if i > 0 && board.is_reserved(row, link, link_exists) {
            break;
        }
        if i >= text.len() {
            if single_line {
                break;
            }
            board.apply_text_manually(row, None, now);
            i += 1;
            continue;
        }
        load_layout(&mut board.lines[row], i);
        let sections = &mut board.lines[row].sections;
        for (index, section) in sections.iter_mut().enumerate() {
            let Some(piece) = text[i].get(index) else {
                break;
            };
            section.set_text(piece, now);
        }
        i += 1;
    }
}

/// `FlapDisplayBlock.updateColumn`: whether the board joins the one above (`up`) and below
/// (`down`) it. `is_board(step, dy)` says whether the block `step` blocks along the row from this
/// one and `dy` up is a board facing the same way (the block itself, at 0, 0, always is).
/// Every board along the whole row must have a partner above for a block to join upwards.
pub fn update_column(is_board: impl Fn(i32, i32) -> bool) -> (bool, bool) {
    let join = |dy: i32| {
        for movement in [1, -1] {
            let mut step = 0;
            for _ in 0..1000 {
                let here = is_board(step, 0);
                let beside = is_board(step, dy);
                step += movement;
                if !here && !beside {
                    break;
                }
                if here && beside {
                    continue;
                }
                return false;
            }
        }
        true
    };
    (join(1), join(-1))
}

/// What `FlapDisplayBlockEntity.updateControllerStatus` finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sizes {
    pub controller: bool,
    pub x_size: i32,
    pub y_size: i32,
}

/// `updateControllerStatus`: a board that doesn't join upwards and has no equal block on its left
/// is the controller of the block of boards that runs right (`right_same(n)`: the block `n` to the
/// right is equal) and down (`down(n)`: the block `n` below has `down` set).
pub fn controller_sizes(
    up: bool,
    left_same: bool,
    right_same: impl Fn(i32) -> bool,
    down: impl Fn(i32) -> bool,
) -> Sizes {
    let controller = !up && !left_same;
    let (mut x_size, mut y_size) = (1, 1);
    if controller {
        for x in 1..32 {
            if !right_same(x) {
                break;
            }
            x_size += 1;
        }
        for y in 0..32 {
            if !down(y) {
                break;
            }
            y_size += 1;
        }
    }
    Sizes { controller, x_size, y_size }
}

// --- the spin ----------------------------------------------------------------------------------

/// `RandomSource.nextInt` stand-in: a small generator, seeded per section.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }

    fn next(&mut self) -> u32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// `RandomSource.nextInt(bound)`.
    pub fn below(&mut self, bound: i32) -> i32 {
        if bound <= 1 { 0 } else { (self.next() % bound as u32) as i32 }
    }
}

/// `FlapDisplaySection.tick`'s `max`: how many ticks a spin can last.
pub fn spin_limit(options: usize) -> i32 {
    4.max((options as f32 * 1.75) as i32)
}

/// A seed for the spin of one section: the board, line and section, and when the text changed.
pub fn spin_seed(controller: Pos, line: usize, section: usize, since: i64) -> u64 {
    let mut h = since as u64;
    for v in [controller[0], controller[1], controller[2], line as i32, section as i32] {
        h = h.wrapping_mul(0x0000_0100_0000_01B3).wrapping_add(v as u32 as u64 + 0x9E37);
    }
    h
}

/// The spin of a section `ticks` ticks after its text changed, by `FlapDisplaySection.tick`
/// run from the start: which flaps still turn, the section's `spinningTicks`, and how many turned
/// in the last tick (what makes the clatter). `instant` is a board turning faster than 128 RPM.
pub fn spin_after(
    changed: &[bool],
    options: usize,
    ticks: i32,
    instant: bool,
    seed: u64,
) -> (Vec<bool>, i32, usize) {
    let mut spinning = changed.to_vec();
    let mut spinning_ticks = 0;
    let mut flaps = 0;
    let mut rng = Rng::new(seed);
    let max = spin_limit(options);
    for _ in 0..ticks.min(max + 2) {
        flaps = 0;
        if spinning_ticks > max {
            break;
        }
        spinning_ticks += 1;
        if spinning_ticks < 2 {
            // The first tick only starts the count; the second reports every flap.
            continue;
        }
        let n = spinning.len();
        for i in 0..n {
            let chance = (8 - spinning_ticks).clamp(1, 10);
            let mut keep = !instant && rng.below(chance * max / 4) != 0;
            keep &= max > 5 || spinning_ticks < 2;
            spinning[i] &= keep;
            if i > 0 && rng.below(3) > 0 {
                spinning[i - 1] &= keep;
            }
            if i + 1 < n && rng.below(3) > 0 {
                spinning[i + 1] &= keep;
            }
            if spinning_ticks > max {
                spinning[i] = false;
            }
            if spinning[i] {
                flaps += 1;
            }
        }
    }
    (spinning, spinning_ticks, flaps)
}

/// The character a spinning flap shows (`FlapDisplayRenderOutput.accept`): the cycle's character
/// at the moment, turning to `-` and `_` between. `time` is the game time with the partial tick,
/// 0 for a board that is paused.
pub fn spinning_glyph(
    options: &[String],
    spinning_ticks: i32,
    time: f32,
    char_index: usize,
    line_index: usize,
    paused: bool,
) -> char {
    let speed = if spinning_ticks > 5 && spinning_ticks < 20 { 1.75 } else { 2.5 };
    let cycle = time / speed + char_index as f32 * 16.83 + line_index as f32 * 0.75;
    let partial = cycle - cycle.floor();
    let shown = options
        .get(cycle as usize % options.len().max(1))
        .and_then(|o| o.chars().next())
        .unwrap_or(' ');
    if paused {
        shown
    } else if partial > 0.5 {
        if partial > 0.75 { '_' } else { '-' }
    } else {
        shown
    }
}

/// Where a line starts across the board in font units (`FlapDisplayRenderer`).
pub fn line_start(x_size: i32, line_width: f32) -> f32 {
    x_size as f32 * 16.0 - line_width / 2.0 + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_holds_as_many_characters_as_the_board_is_wide() {
        assert_eq!(max_char_count(1, 0), 4);
        assert_eq!(max_char_count(2, 0), 8);
        assert_eq!(max_char_count(4, 1), 16);
        let board = Board::new(3, 2);
        assert_eq!(board.lines.len(), 4);
        assert_eq!(board.lines[0].sections[0].flaps(), 13);
    }

    #[test]
    fn text_is_upper_cased_cut_and_padded() {
        let mut s = Section::new(7.0 * 5.0, "alphabet", false, false);
        s.set_text("abc", 10);
        assert_eq!(s.text, "ABC  ");
        s.set_text("abcdefgh", 11);
        assert_eq!(s.text, "ABCDE");
        let mut right = Section::new(7.0 * 5.0, "numeric", false, false).right_aligned();
        right.set_text("  42 ", 12);
        assert_eq!(right.text, "   42");
        assert_eq!(right.since, Some(12));
    }

    #[test]
    fn only_the_flaps_that_change_spin() {
        let mut s = Section::new(7.0 * 4.0, "alphabet", false, false);
        s.set_text("abcd", 5);
        assert_eq!(s.changed(), vec![true; 4]);
        s.set_text("abxd", 9);
        assert_eq!(s.changed(), vec![false, false, true, false]);
        assert_eq!(s.since, Some(9));
        // The same text again leaves the animation as it was.
        s.set_text("abxd", 20);
        assert_eq!(s.since, Some(9));
    }

    #[test]
    fn a_single_flap_changes_as_a_whole() {
        let mut s = Section::new(14.0, "fluid_units", true, true);
        assert_eq!(s.flaps(), 1);
        s.set_text("mB", 1);
        assert_eq!(s.text, "mB");
        assert_eq!(s.changed(), vec![true]);
    }

    #[test]
    fn a_manual_line_is_reserved_and_a_cleared_one_is_not() {
        let mut board = Board::new(2, 1);
        board.apply_text_manually(1, Some("hi"), 0);
        assert!(board.manual[1]);
        assert!(board.is_reserved(1, [0, 0, 0], &|_| true));
        board.apply_text_manually(1, None, 1);
        assert!(!board.manual[1]);
        assert!(!board.is_reserved(1, [0, 0, 0], &|_| true));
        // Another link owns a line while it exists; once it is gone the line is free.
        board.reserve(1, [9, 9, 9]);
        assert!(board.is_reserved(1, [0, 0, 0], &|p| p == [9, 9, 9]));
        assert!(!board.is_reserved(1, [0, 0, 0], &|_| false));
        assert_eq!(board.reserved[1], None);
        // Line 0 is never reserved.
        board.reserve(0, [9, 9, 9]);
        assert_eq!(board.reserved[0], None);
    }

    #[test]
    fn a_link_writes_lines_down_the_board_until_one_is_taken() {
        let mut board = Board::new(2, 2);
        board.apply_text_manually(2, Some("mine"), 0);
        let text = vec![vec!["one".to_owned()], vec!["two".to_owned()], vec!["three".to_owned()]];
        accept_flap_text(&mut board, 0, &text, false, [1, 1, 1], &|_| true, 5, &|_, _| {});
        assert_eq!(board.lines[0].sections[0].text.trim(), "ONE");
        assert_eq!(board.lines[1].sections[0].text.trim(), "TWO");
        // Line 2 was set by hand, so the third row stops there.
        assert_eq!(board.lines[2].sections[0].text.trim(), "MINE");
        // A shorter list clears the lines below, unless the source is a single line.
        let short = vec![vec!["x".to_owned()]];
        accept_flap_text(&mut board, 0, &short, false, [1, 1, 1], &|_| true, 6, &|_, _| {});
        assert_eq!(board.lines[1].sections[0].text.trim(), "");
        let mut other = Board::new(2, 2);
        other.apply_text_manually(1, Some("keep"), 0);
        other.manual[1] = false;
        accept_flap_text(&mut other, 0, &short, true, [1, 1, 1], &|_| true, 6, &|_, _| {});
        assert_eq!(other.lines[1].sections[0].text.trim(), "KEEP");
    }

    #[test]
    fn boards_join_in_columns_only_when_whole_rows_match() {
        // A single board alone.
        assert_eq!(update_column(|s, dy| s == 0 && dy == 0), (false, false));
        // Two boards in a row with two above them: both join up.
        let wide = |s: i32, dy: i32| (0..2).contains(&s) && (0..=1).contains(&dy);
        assert_eq!(update_column(wide), (true, false));
        // Only one board above a row of two: the row doesn't join.
        let ragged = |s: i32, dy: i32| (0..2).contains(&s) && dy == 0 || s == 0 && dy == 1;
        assert_eq!(update_column(ragged), (false, false));
    }

    #[test]
    fn the_top_left_board_is_the_controller_and_counts_the_block() {
        let sizes = controller_sizes(false, false, |n| n < 3, |n| n < 1);
        assert_eq!(sizes, Sizes { controller: true, x_size: 3, y_size: 2 });
        let inner = controller_sizes(false, true, |_| true, |_| true);
        assert_eq!(inner, Sizes { controller: false, x_size: 1, y_size: 1 });
        assert!(!controller_sizes(true, false, |_| true, |_| true).controller);
    }

    #[test]
    fn a_click_picks_the_line_by_its_height() {
        // A board two blocks high whose top block is at y 10: its lines are 10.5 and 10 (top block),
        // 9.5 and 9 (lower).
        assert_eq!(line_index_at(10, 10.9, 2), 0);
        assert_eq!(line_index_at(10, 10.2, 2), 1);
        assert_eq!(line_index_at(10, 9.7, 2), 2);
        assert_eq!(line_index_at(10, 9.1, 2), 3);
        assert_eq!(line_index_at(10, 5.0, 2), 4);
    }

    #[test]
    fn the_spin_ends_and_is_the_same_every_time() {
        let changed = vec![true; 6];
        let first = spin_after(&changed, 26, 5, false, 7);
        assert_eq!(first, spin_after(&changed, 26, 5, false, 7));
        // At the start every flap turns; long after, none does.
        assert_eq!(spin_after(&changed, 26, 0, false, 7).0, changed);
        let end = spin_after(&changed, 26, 200, false, 7);
        assert!(end.0.iter().all(|s| !s));
        assert_eq!(end.1, spin_limit(26) + 1);
        // A fast board settles at once.
        let instant = spin_after(&changed, 26, 3, true, 7);
        assert!(instant.0.iter().all(|s| !s));
        // An unchanged flap never turns.
        let some = spin_after(&[true, false, true], 26, 2, false, 3);
        assert!(!some.0[1]);
    }

    #[test]
    fn a_spinning_flap_shows_the_cycle_then_dashes() {
        let options = cycle_options("numeric");
        assert_eq!(options.len(), 11);
        // Paused: always a cycle character.
        assert_eq!(spinning_glyph(&options, 3, 0.0, 0, 0, true), ' ');
        let seen: Vec<char> = (0..40)
            .map(|t| spinning_glyph(&options, 3, t as f32 * 0.5, 2, 1, false))
            .collect();
        assert!(seen.contains(&'-') && seen.contains(&'_'));
        assert_eq!(spin_limit(26), 45);
        assert_eq!(spin_limit(3), 5);
    }

    #[test]
    fn layouts_hold_their_width() {
        let mut layout = Layout::new(8);
        assert!(layout.is_layout("Default"));
        assert_eq!(layout.width(), 8.0 * 7.0 + 1.0);
        layout.configure(
            "x",
            vec![Section::new(14.0, "alphabet", false, true), Section::new(14.0, "numeric", false, false)],
        );
        assert_eq!(layout.width(), 14.0 + 8.0 + 14.0 + 1.0);
        assert_eq!(line_start(2, layout.width()), 32.0 - layout.width() / 2.0 + 1.0);
        assert!(speed_fulfilled(-30.0));
        assert!(!speed_fulfilled(29.0));
    }
}

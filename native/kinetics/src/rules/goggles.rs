//! The text and the box of Engineer's Goggles' overlay (`GoggleOverlayRenderer`,
//! `RemovedGuiUtils.drawHoveringText`, Catnip's `LangBuilder`, `LangNumberFormat` and
//! `TooltipHelper`): what a line of the tooltip is made of, how Create's language keys turn into
//! text, how a number is written, where the box goes on the screen and what it is drawn with.
//!
//! No game types. Which lines a block shows is `goggles_kinetic`, `goggles_blocks` and
//! `goggles_overlay`; the game glue (`machines/goggles.rs`) reads the world, calls them and draws
//! what they return.
//!
//! Known differences from Create, for the README:
//! - The box fades in as Create's does by sliding in from the side (`hoverTicks` counts frames),
//!   but Create's alpha fade (`colorBackground.scaleAlpha(fade)`, whose result it throws away)
//!   does nothing there either, so it is not drawn.
//! - The box's border is Create's default (vanilla tooltip colours); the custom colours and the
//!   offsets of Create's client config (`/create overlay`) are constants here.
//! - A line is cut by words at 200 pixels with the font's own widths, as `TooltipHelper` does;
//!   Java's `BreakIterator` is stood in for by breaking after spaces and after a hyphen between
//!   letters.

use std::collections::BTreeMap;

// --- ChatFormatting's colours -----------------------------------------------------------------

pub const DARK_GREEN: u32 = 0x00AA00;
pub const DARK_RED: u32 = 0xAA0000;
pub const GOLD: u32 = 0xFFAA00;
pub const GRAY: u32 = 0xAAAAAA;
pub const DARK_GRAY: u32 = 0x555555;
pub const BLUE: u32 = 0x5555FF;
pub const GREEN: u32 = 0x55FF55;
pub const AQUA: u32 = 0x55FFFF;
pub const RED: u32 = 0xFF5555;
pub const LIGHT_PURPLE: u32 = 0xFF55FF;
pub const YELLOW: u32 = 0xFFFF55;
pub const WHITE: u32 = 0xFFFFFF;

// --- lines ------------------------------------------------------------------------------------

/// A run of text in one colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub text: String,
    /// `0xRRGGBB`.
    pub color: u32,
    /// `ChatFormatting.STRIKETHROUGH`.
    pub strike: bool,
}

/// One line of the tooltip. An empty one is `CommonComponents.EMPTY`: a blank line.
pub type Line = Vec<Seg>;

/// The line's text with no colours.
pub fn plain(line: &[Seg]) -> String {
    line.iter().map(|seg| seg.text.as_str()).collect()
}

#[derive(Clone, Debug, Default)]
struct Part {
    text: String,
    color: Option<u32>,
    strike: bool,
}

/// Catnip's `LangBuilder`: components added one after the other, with a style on the whole.
/// A style on a part wins over the style on the builder, as a child's does in a `Component`.
#[derive(Clone, Debug, Default)]
pub struct Builder {
    parts: Vec<Part>,
    color: Option<u32>,
    strike: bool,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `LangBuilder.text`.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.parts.push(Part { text: text.into(), ..Part::default() });
        self
    }

    /// `LangBuilder.text(ChatFormatting, String)`.
    pub fn text_in(mut self, color: u32, text: impl Into<String>) -> Self {
        self.parts.push(Part { text: text.into(), color: Some(color), strike: false });
        self
    }

    /// `LangBuilder.space`.
    pub fn space(self) -> Self {
        self.text(" ")
    }

    /// `CreateLang.number`: a number as `LangNumberFormat` writes it.
    pub fn number(self, value: f64) -> Self {
        self.text(number(value))
    }

    /// `LangBuilder.translate`: Create's key (`create.<key>`) with its arguments.
    pub fn tr(self, lang: &Lang, key: &str, args: &[String]) -> Self {
        let text = lang.tr(key, args);
        self.text(text)
    }

    /// `LangBuilder.translate` with arguments that carry a style of their own (a `Component` with
    /// `withStyle`): each is `(text, colour)`.
    pub fn tr_styled(mut self, lang: &Lang, key: &str, args: &[(String, Option<u32>)]) -> Self {
        let plain: Vec<String> = args.iter().map(|(text, _)| text.clone()).collect();
        let Some(format) = lang.get(&format!("create.{key}")) else {
            return self.text(format!("create.{key}"));
        };
        for (text, arg) in format_parts(format, &plain) {
            let color = arg.and_then(|i| args[i].1);
            self.parts.push(Part { text, color, strike: false });
        }
        self
    }

    /// `LangBuilder.style(ChatFormatting)`: a colour for the parts that have none of their own.
    pub fn style(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    /// `LangBuilder.style(ChatFormatting.STRIKETHROUGH)`.
    pub fn strikethrough(mut self) -> Self {
        self.strike = true;
        self
    }

    /// `LangBuilder.add(LangBuilder)`: the other builder's parts, each with the style the other
    /// gave the whole (a part's own style first).
    pub fn add(mut self, other: Builder) -> Self {
        for part in other.parts {
            self.parts.push(Part {
                text: part.text,
                color: part.color.or(other.color),
                strike: part.strike || other.strike,
            });
        }
        self
    }

    /// The builder as a line: the colours resolved, white where none was given.
    pub fn line(self) -> Line {
        self.parts
            .into_iter()
            .filter(|part| !part.text.is_empty())
            .map(|part| Seg {
                text: part.text,
                color: part.color.or(self.color).unwrap_or(WHITE),
                strike: part.strike || self.strike,
            })
            .collect()
    }

    /// `LangBuilder.forGoggles(tooltip)`.
    pub fn for_goggles(self, tooltip: &mut Vec<Line>) {
        self.for_goggles_at(tooltip, 0);
    }

    /// `LangBuilder.forGoggles(tooltip, indents)`: four spaces and one for each indent in front.
    pub fn for_goggles_at(self, tooltip: &mut Vec<Line>, indents: usize) {
        let mut line = vec![Seg { text: " ".repeat(4 + indents), color: WHITE, strike: false }];
        line.extend(self.line());
        tooltip.push(line);
    }
}

/// `CommonComponents.EMPTY` as a line.
pub fn blank() -> Line {
    Vec::new()
}

// --- the language -----------------------------------------------------------------------------

/// Create's language file (`assets/create/lang/en_us.json`), with vanilla's names beside it.
#[derive(Clone, Debug, Default)]
pub struct Lang {
    entries: BTreeMap<String, String>,
}

impl Lang {
    pub fn new(entries: BTreeMap<String, String>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// `Component.translatable(key, args)` as a string: a key the language lacks is shown as it is.
    pub fn translate(&self, key: &str, args: &[String]) -> String {
        match self.get(key) {
            Some(format) => format_args(format, args),
            None => key.to_owned(),
        }
    }

    /// `CreateLang.translate(key, args)`: the key in Create's namespace.
    pub fn tr(&self, key: &str, args: &[String]) -> String {
        self.translate(&format!("create.{key}"), args)
    }

    /// `ItemStack.getHoverName` for an item id (`minecraft:iron_ingot`): `item.<namespace>.<path>`,
    /// then `block.<namespace>.<path>` for a block item.
    pub fn item_name(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        for kind in ["item", "block"] {
            if let Some(name) = self.get(&format!("{kind}.{namespace}.{path}")) {
                return name.to_owned();
            }
        }
        title_case(path)
    }

    /// `FluidStack.getHoverName` for a fluid id (`minecraft:water`): its fluid type's description id.
    pub fn fluid_name(&self, id: &str) -> String {
        // A potion fluid is named by its potion (`PotionFluidType.getDescriptionId`).
        if let Some(potion) = super::potion_fluid::parse(id) {
            let key = super::potion_fluid::name_key(&potion);
            return self.get(&key).map_or_else(|| "Potion".to_owned(), str::to_owned);
        }
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        for kind in ["fluid_type", "block", "fluid"] {
            if let Some(name) = self.get(&format!("{kind}.{namespace}.{path}")) {
                return name.to_owned();
            }
        }
        title_case(path)
    }

    /// `Block.getName` for a block id.
    pub fn block_name(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        self.get(&format!("block.{namespace}.{path}"))
            .map_or_else(|| title_case(path), str::to_owned)
    }
}

/// `iron_ingot` as "Iron Ingot", for an id the language doesn't know.
fn title_case(path: &str) -> String {
    path.split(['_', '/'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect::<String>()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Minecraft's `TranslatableContents` formatting: `%s`, `%1$s` and `%%`.
fn format_args(format: &str, args: &[String]) -> String {
    format_parts(format, args).into_iter().map(|(text, _)| text).collect()
}

/// The pieces of a formatted translation: the text between the placeholders (no argument) and
/// each placeholder's argument (its index).
fn format_parts(format: &str, args: &[String]) -> Vec<(String, Option<usize>)> {
    let mut parts: Vec<(String, Option<usize>)> = Vec::new();
    let mut out = String::new();
    let mut next = 0;
    let flush = |out: &mut String, parts: &mut Vec<(String, Option<usize>)>| {
        if !out.is_empty() {
            parts.push((std::mem::take(out), None));
        }
    };
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some('s') => {
                chars.next();
                flush(&mut out, &mut parts);
                parts.push((args.get(next).cloned().unwrap_or_default(), Some(next)));
                next += 1;
            }
            Some(d) if d.is_ascii_digit() => {
                let mut digits = String::new();
                while let Some(&d) = chars.peek() {
                    if !d.is_ascii_digit() {
                        break;
                    }
                    digits.push(d);
                    chars.next();
                }
                if chars.peek() == Some(&'$') {
                    chars.next();
                    if chars.peek() == Some(&'s') {
                        chars.next();
                        let index: usize = digits.parse().unwrap_or(0);
                        flush(&mut out, &mut parts);
                        let at = index.saturating_sub(1);
                        parts.push((args.get(at).cloned().unwrap_or_default(), Some(at)));
                        continue;
                    }
                }
                out.push('%');
                out.push_str(&digits);
            }
            _ => out.push('%'),
        }
    }
    flush(&mut out, &mut parts);
    parts
}

// --- numbers ----------------------------------------------------------------------------------

/// `LangNumberFormat.format`: at most two decimals, none when whole, thousands grouped with a
/// comma (en_us); near zero is `0`.
pub fn number(value: f64) -> String {
    let value = if value.abs() < 1.0e-5 { 0.0 } else { value };
    let fixed = format!("{value:.2}");
    let (sign, digits) = fixed.strip_prefix('-').map_or(("", fixed.as_str()), |d| ("-", d));
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let fraction = fraction.trim_end_matches('0');
    let sign = if grouped == "0" && fraction.is_empty() { "" } else { sign };
    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
}

/// `TooltipHelper.makeProgressBar`: ` ███▒▒ `.
pub fn progress_bar(length: usize, filled: usize) -> String {
    let mut bar = String::from(" ");
    for _ in 0..filled {
        bar.push('\u{2588}');
    }
    for _ in 0..length.saturating_sub(filled) {
        bar.push('\u{2592}');
    }
    bar.push(' ');
    bar
}

// --- cutting a hint into lines -----------------------------------------------------------------

/// `TooltipHelper.MAX_WIDTH_PER_LINE`.
pub const MAX_WIDTH_PER_LINE: f32 = 200.0;

/// Where `BreakIterator.getLineInstance` would break `text`: after each run of spaces, and after a
/// hyphen that stands between two letters.
fn words(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        let end_of = |j: usize| chars.get(j).map_or(text.len(), |(k, _)| *k);
        if c == ' ' {
            let mut j = i;
            while j < chars.len() && chars[j].1 == ' ' {
                j += 1;
            }
            out.push(&text[start..end_of(j)]);
            start = end_of(j);
            i = j;
            continue;
        }
        if c == '-'
            && i > 0
            && chars[i - 1].1.is_alphanumeric()
            && chars.get(i + 1).is_some_and(|(_, next)| next.is_alphanumeric())
        {
            out.push(&text[start..end_of(i + 1)]);
            start = end_of(i + 1);
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// `TooltipHelper.cutTextComponent(text, primary, highlight, indent)`: the text broken into lines
/// of at most [`MAX_WIDTH_PER_LINE`] (`width` measures a string, an underscore not counted),
/// the words between underscores in `highlight` and the rest in `primary`; the highlight carries
/// from one line to the next. Each line begins with `indent` spaces.
pub fn cut_text(
    text: &str,
    primary: u32,
    highlight: u32,
    indent: usize,
    width: &dyn Fn(&str) -> f32,
) -> Vec<Line> {
    // Hard wrap.
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0.0;
    for word in words(text) {
        let new_width = width(&word.replace('_', ""));
        if used + new_width > MAX_WIDTH_PER_LINE {
            if used > 0.0 {
                lines.push(std::mem::take(&mut current));
                used = 0.0;
            } else {
                lines.push(word.to_owned());
                continue;
            }
        }
        current.push_str(word);
        used += new_width;
    }
    if used > 0.0 {
        lines.push(current);
    }
    // Format: each underscore flips between the primary and the highlight.
    let mut out = Vec::with_capacity(lines.len());
    let mut highlighted = false;
    for line in lines {
        let mut segs = Line::new();
        if indent > 0 {
            segs.push(Seg { text: " ".repeat(indent), color: primary, strike: false });
        }
        for part in line.split('_') {
            if !part.is_empty() {
                segs.push(Seg {
                    text: part.to_owned(),
                    color: if highlighted { highlight } else { primary },
                    strike: false,
                });
            }
            highlighted = !highlighted;
        }
        highlighted = !highlighted;
        out.push(segs);
    }
    out
}

/// `TooltipHelper.cutTextComponent(text, Palette.GRAY_AND_WHITE)`.
pub fn cut_gray_and_white(text: &str, width: &dyn Fn(&str) -> f32) -> Vec<Line> {
    cut_text(text, GRAY, WHITE, 0, width)
}

/// `CreateLang.builder().add(component).forGoggles(tooltip)` for each line of `lines`.
pub fn push_for_goggles(tooltip: &mut Vec<Line>, lines: Vec<Line>) {
    for line in lines {
        let mut full = vec![Seg { text: "    ".to_owned(), color: WHITE, strike: false }];
        full.extend(line);
        tooltip.push(full);
    }
}

// --- the box ----------------------------------------------------------------------------------

/// `CClient.overlayOffsetX` and `overlayOffsetY`.
pub const OFFSET: [i32; 2] = [20, 0];
/// `BoxElement.COLOR_VANILLA_BACKGROUND.scaleAlpha(.75f)`: `0xf0100010` at three quarters alpha.
pub const BACKGROUND: u32 = 0xB410_0010;
/// `BoxElement.COLOR_VANILLA_BORDER`: the top and bottom of the border's gradient.
pub const BORDER_TOP: u32 = 0x5050_00FF;
pub const BORDER_BOTTOM: u32 = 0x5028_007F;
/// The frames a box takes to slide in (`hoverTicks / 24`).
pub const FADE_FRAMES: f32 = 24.0;

/// A rectangle (`[x, y, width, height]`) filled from `top` at its top edge to `bottom` at its
/// bottom edge (`GuiGraphics.fillGradient`), colours as `0xAARRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub rect: [f32; 4],
    pub top: u32,
    pub bottom: u32,
}

/// Where the box and what is in it go.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The rectangles to fill, in order: the background, then the border.
    pub rects: Vec<Rect>,
    /// Where the first line's text begins; the next lines are [`Layout::line_y`] below.
    pub text: [f32; 2],
    /// Where the goggles icon goes (16 pixels square).
    pub icon: [f32; 2],
}

impl Layout {
    /// The top of line `index`: ten pixels each, with a two pixel gap after the first
    /// (`RemovedGuiUtils`'s `titleLinesCount`).
    pub fn line_y(&self, index: usize) -> f32 {
        self.text[1] + index as f32 * 10.0 + if index > 0 { 2.0 } else { 0.0 }
    }
}

/// The height of the text of `lines` lines: 8 for one, then the gap and ten for each more.
pub fn text_height(lines: usize) -> f32 {
    if lines > 1 { 8.0 + 2.0 + (lines - 1) as f32 * 10.0 } else { 8.0 }
}

/// How far a box that has been up for `frames` frames is still to slide in
/// (`pow(1 - fade, 3) * signum(overlayOffsetX + .5) * 8`).
pub fn slide(frames: f32) -> f32 {
    let fade = (frames / FADE_FRAMES).clamp(0.0, 1.0);
    if fade >= 1.0 {
        return 0.0;
    }
    (1.0 - fade).powi(3) * if OFFSET[0] as f32 + 0.5 < 0.0 { -8.0 } else { 8.0 }
}

/// `GoggleOverlayRenderer.renderOverlay`'s placing and `RemovedGuiUtils.drawHoveringText`'s box,
/// for a screen of `screen` GUI pixels and text `text_width` wide with `lines` lines.
pub fn layout(screen: [f32; 2], text_width: f32, lines: usize) -> Layout {
    let [width, height] = screen;
    let text_height = text_height(lines);
    let mut pos_x = width / 2.0 + OFFSET[0] as f32;
    let mut pos_y = height / 2.0 + OFFSET[1] as f32;
    pos_x = pos_x.min(width - text_width - 20.0);
    pos_y = pos_y.min(height - text_height - 20.0);
    // `drawHoveringText` with the mouse at (pos_x, pos_y).
    let mut x = pos_x + 12.0;
    if x + text_width + 4.0 > width {
        x = pos_x - 16.0 - text_width;
    }
    let mut y = pos_y - 12.0;
    if y < 4.0 {
        y = 4.0;
    } else if y + text_height + 4.0 > height {
        y = height - text_height - 4.0;
    }
    let (tw, th) = (text_width, text_height);
    let fill = |x0: f32, y0: f32, x1: f32, y1: f32, top: u32, bottom: u32| Rect {
        rect: [x0, y0, x1 - x0, y1 - y0],
        top,
        bottom,
    };
    let bg = BACKGROUND;
    let rects = vec![
        fill(x - 3.0, y - 4.0, x + tw + 3.0, y - 3.0, bg, bg),
        fill(x - 3.0, y + th + 3.0, x + tw + 3.0, y + th + 4.0, bg, bg),
        fill(x - 3.0, y - 3.0, x + tw + 3.0, y + th + 3.0, bg, bg),
        fill(x - 4.0, y - 3.0, x - 3.0, y + th + 3.0, bg, bg),
        fill(x + tw + 3.0, y - 3.0, x + tw + 4.0, y + th + 3.0, bg, bg),
        fill(x - 3.0, y - 2.0, x - 2.0, y + th + 2.0, BORDER_TOP, BORDER_BOTTOM),
        fill(x + tw + 2.0, y - 2.0, x + tw + 3.0, y + th + 2.0, BORDER_TOP, BORDER_BOTTOM),
        fill(x - 3.0, y - 3.0, x + tw + 3.0, y - 2.0, BORDER_TOP, BORDER_TOP),
        fill(x - 3.0, y + th + 2.0, x + tw + 3.0, y + th + 3.0, BORDER_BOTTOM, BORDER_BOTTOM),
    ];
    Layout { rects, text: [x, y], icon: [pos_x + 10.0, pos_y - 16.0] }
}

/// The colour at row `row` of `rows` of a vertical gradient from `top` to `bottom`
/// (`0xAARRGGBB`, each channel mixed).
pub fn gradient(top: u32, bottom: u32, row: usize, rows: usize) -> u32 {
    if rows <= 1 || top == bottom {
        return top;
    }
    let t = row as f32 / (rows - 1) as f32;
    let mix = |shift: u32| {
        let a = ((top >> shift) & 0xFF) as f32;
        let b = ((bottom >> shift) & 0xFF) as f32;
        (a + (b - a) * t).round() as u32
    };
    mix(24) << 24 | mix(16) << 16 | mix(8) << 8 | mix(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang() -> Lang {
        Lang::new(BTreeMap::from([
            ("create.gui.goggles.kinetic_stats".to_owned(), "Kinetic Stats:".to_owned()),
            ("create.tooltip.analogStrength".to_owned(), "Analog Strength: %1$s/15".to_owned()),
            ("create.chute.contains".to_owned(), "Contains: %1$s x%2$s".to_owned()),
            ("item.create.cogwheel".to_owned(), "Cogwheel".to_owned()),
            ("block.minecraft.water".to_owned(), "Water".to_owned()),
        ]))
    }

    #[test]
    fn numbers_are_written_as_lang_number_format_does() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.000001), "0");
        assert_eq!(number(5.0), "5");
        assert_eq!(number(1234.5), "1,234.5");
        assert_eq!(number(16384.0), "16,384");
        assert_eq!(number(1234567.891), "1,234,567.89");
        assert_eq!(number(0.1), "0.1");
        assert_eq!(number(2.25), "2.25");
        assert_eq!(number(-12.5), "-12.5");
        // Ties go to the even digit (`RoundingMode.HALF_EVEN`).
        assert_eq!(number(0.125), "0.12");
        assert_eq!(number(0.375), "0.38");
        // A float's noise (`0.1f` as a double) disappears at two decimals.
        assert_eq!(number(f64::from(0.1f32)), "0.1");
    }

    #[test]
    fn translations_fill_in_their_arguments() {
        let lang = lang();
        assert_eq!(lang.tr("gui.goggles.kinetic_stats", &[]), "Kinetic Stats:");
        assert_eq!(
            lang.tr("tooltip.analogStrength", &["7".to_owned()]),
            "Analog Strength: 7/15"
        );
        assert_eq!(
            lang.tr("chute.contains", &["Cogwheel".to_owned(), "3".to_owned()]),
            "Contains: Cogwheel x3"
        );
        // A key that isn't there is shown as it is, as Minecraft does.
        assert_eq!(lang.tr("nothing.here", &[]), "create.nothing.here");
        assert_eq!(format_args("100%% of %s", &["it".to_owned()]), "100% of it");
    }

    #[test]
    fn names_come_from_the_language_or_the_id() {
        let lang = lang();
        assert_eq!(lang.item_name("create:cogwheel"), "Cogwheel");
        assert_eq!(lang.fluid_name("minecraft:water"), "Water");
        assert_eq!(lang.item_name("minecraft:iron_ingot"), "Iron Ingot");
        assert_eq!(lang.fluid_name("create:honey"), "Honey");
    }

    #[test]
    fn a_builder_styles_the_parts_that_have_no_style_of_their_own() {
        // `CreateLang.number(x).translate("generic.unit.stress").style(AQUA).space()
        //  .add(CreateLang.translate("gui.goggles.at_current_speed").style(DARK_GRAY))`
        let line = Builder::new()
            .number(512.0)
            .text("su")
            .style(AQUA)
            .space()
            .add(Builder::new().text("at current speed").style(DARK_GRAY))
            .line();
        assert_eq!(
            line,
            vec![
                Seg { text: "512".into(), color: AQUA, strike: false },
                Seg { text: "su".into(), color: AQUA, strike: false },
                Seg { text: " ".into(), color: AQUA, strike: false },
                Seg { text: "at current speed".into(), color: DARK_GRAY, strike: false },
            ]
        );
        let struck = Builder::new().text("x").style(DARK_GRAY).strikethrough().line();
        assert!(struck[0].strike);
        assert_eq!(struck[0].color, DARK_GRAY);
    }

    #[test]
    fn for_goggles_indents_by_four_and_one_more_for_each_level() {
        let mut tooltip = Vec::new();
        Builder::new().text("a").for_goggles(&mut tooltip);
        Builder::new().text("b").for_goggles_at(&mut tooltip, 1);
        assert_eq!(plain(&tooltip[0]), "    a");
        assert_eq!(plain(&tooltip[1]), "     b");
    }

    #[test]
    fn the_progress_bar_is_filled_then_empty() {
        assert_eq!(progress_bar(3, 0), " \u{2592}\u{2592}\u{2592} ");
        assert_eq!(progress_bar(3, 2), " \u{2588}\u{2588}\u{2592} ");
        assert_eq!(progress_bar(3, 3), " \u{2588}\u{2588}\u{2588} ");
    }

    fn six(text: &str) -> f32 {
        text.chars().count() as f32 * 6.0
    }

    #[test]
    fn underscores_highlight_and_a_long_hint_is_cut_by_words() {
        let hint = "It appears that this contraption is _overstressed_. Add more sources or _slow_ \
                    _down_ the components with a high _stress_ _impact_.";
        let lines = cut_gray_and_white(hint, &six);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(six(&plain(line)) <= MAX_WIDTH_PER_LINE + 6.0, "{}", plain(line));
        }
        let joined: String = lines.iter().map(|l| plain(l)).collect();
        assert_eq!(joined, hint.replace('_', ""));
        let words: Vec<(&str, u32)> = lines
            .iter()
            .flatten()
            .filter(|seg| seg.color == WHITE)
            .map(|seg| (seg.text.as_str(), seg.color))
            .collect();
        let highlighted: Vec<&str> = words.iter().map(|w| w.0).collect();
        assert_eq!(highlighted, ["overstressed", "slow", "down", "stress", "impact"]);
    }

    #[test]
    fn the_highlight_carries_over_a_line_break() {
        // 36 characters a line at six pixels: the highlighted span runs over the break.
        let wide = |s: &str| s.chars().count() as f32 * 34.0;
        let lines = cut_text("aa _bb cc_ dd", GRAY, WHITE, 0, &wide);
        let colours: Vec<Vec<u32>> =
            lines.iter().map(|l| l.iter().map(|s| s.color).collect()).collect();
        // "aa " | "bb " (highlighted) | "cc" (highlighted) + " dd".
        assert_eq!(plain(&lines[0]), "aa ");
        assert_eq!(colours[1][0], WHITE);
        assert_eq!(colours[2][0], WHITE);
        assert_eq!(*colours[2].last().unwrap(), GRAY);
    }

    #[test]
    fn a_hyphen_between_letters_is_a_place_to_break() {
        assert_eq!(words("Right-click the bearing"), ["Right-", "click ", "the ", "bearing"]);
        assert_eq!(words("a - b"), ["a ", "- ", "b"]);
    }

    #[test]
    fn the_box_sits_right_of_the_crosshair_and_stays_on_screen() {
        // A 427 x 240 screen (a 1920 x 1080 window at scale 4.5): the text starts 32 pixels right
        // of the middle (`+20 offset`, `+12`).
        let layout = layout([427.0, 240.0], 100.0, 3);
        assert_eq!(layout.text, [213.5 + 20.0 + 12.0, 120.0 - 12.0]);
        assert_eq!(layout.icon, [213.5 + 20.0 + 10.0, 120.0 - 16.0]);
        assert_eq!(text_height(3), 8.0 + 2.0 + 20.0);
        // A box too wide for the right side is pulled back (`min(posX, width - w - 20)`).
        let wide = super::layout([427.0, 240.0], 300.0, 3);
        assert_eq!(wide.text[0], 427.0 - 300.0 - 20.0 + 12.0);
        // Too tall for the bottom: pulled up.
        let tall = super::layout([427.0, 240.0], 100.0, 20);
        let h = text_height(20);
        assert_eq!(tall.text[1], 240.0 - h - 20.0 - 12.0);
        // Taller than the screen: held at the top.
        assert_eq!(super::layout([427.0, 100.0], 100.0, 12).text[1], 4.0);
    }

    #[test]
    fn the_box_has_a_background_and_a_gradient_border() {
        let layout = layout([400.0, 200.0], 50.0, 1);
        let [x, y] = layout.text;
        // The middle: from three left of the text to three right, three above to three below.
        let middle = layout.rects[2];
        assert_eq!(middle.rect, [x - 3.0, y - 3.0, 56.0, 14.0]);
        assert_eq!(middle.top, BACKGROUND);
        // The side borders go from the top colour to the bottom one.
        assert_eq!(layout.rects[5].top, BORDER_TOP);
        assert_eq!(layout.rects[5].bottom, BORDER_BOTTOM);
        assert_eq!(layout.rects.len(), 9);
    }

    #[test]
    fn lines_after_the_first_are_ten_apart_with_a_gap_after_the_title() {
        let layout = layout([400.0, 200.0], 50.0, 3);
        let y = layout.text[1];
        assert_eq!(layout.line_y(0), y);
        assert_eq!(layout.line_y(1), y + 12.0);
        assert_eq!(layout.line_y(2), y + 22.0);
    }

    #[test]
    fn the_box_slides_in_over_24_frames() {
        assert_eq!(slide(0.0), 8.0);
        assert_eq!(slide(24.0), 0.0);
        assert_eq!(slide(100.0), 0.0);
        assert!(slide(12.0) < slide(6.0));
    }

    #[test]
    fn a_gradient_goes_from_one_colour_to_the_other() {
        assert_eq!(gradient(0x5050_00FF, 0x5028_007F, 0, 10), 0x5050_00FF);
        assert_eq!(gradient(0x5050_00FF, 0x5028_007F, 9, 10), 0x5028_007F);
        assert_eq!(gradient(0xFF00_0000, 0xFF00_0000, 3, 10), 0xFF00_0000);
    }
}

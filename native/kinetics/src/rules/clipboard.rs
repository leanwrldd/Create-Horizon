//! The Clipboard: `ClipboardContent`, `ClipboardEntry` and what `ClipboardScreen` lets a player do
//! with the pages (add an entry, tick it, remove it, turn to another page), as plain rules. The screen
//! itself needs client UI Horizon doesn't give a mod, so the edits come from the `/clipboard` command
//! (`parse`) and the page is shown as chat lines (`render`).
//!
//! Create saves the content as the item component `create:clipboard_content` with the fields `type`,
//! `pages` (a list of pages, each a list of entries `checked`, `text`, `icon`, `item_amount`),
//! `read_only` and `previously_opened_page`.

pub const COMPONENT: &str = "create:clipboard_content";
pub const CLIPBOARD: &str = "create:clipboard";
/// What a page holds in `ClipboardScreen`: seven lines fit the board.
pub const MAX_ENTRIES_PER_PAGE: usize = 7;
/// What one entry's text may hold (`EditBox`, about a line of the board).
pub const MAX_TEXT: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Entry {
    pub checked: bool,
    pub text: String,
    /// `ClipboardEntry.icon` and `itemAmount`: an item the entry shows, as an id, and how many.
    pub icon: Option<(String, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Content {
    pub pages: Vec<Vec<Entry>>,
    pub read_only: bool,
    /// `previously_opened_page`: the page the clipboard is open at.
    pub page: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Add(String),
    Check(usize),
    Uncheck(usize),
    Toggle(usize),
    Remove(usize),
    /// Replace an entry's text.
    Edit(usize, String),
    /// Turn to this page (1-based).
    Page(usize),
    NewPage,
    RemovePage,
    Clear,
    Show,
}

/// `ClipboardScreen`'s `Clipboard Actions` as one line each, for the usage and the help.
pub const USAGES: [&str; 11] = [
    "clipboard add <text:text>",
    "clipboard check <entry:integer>",
    "clipboard uncheck <entry:integer>",
    "clipboard toggle <entry:integer>",
    "clipboard remove <entry:integer>",
    "clipboard edit <entry:integer> <text:text>",
    "clipboard page <number:integer>",
    "clipboard newpage",
    "clipboard removepage",
    "clipboard clear",
    "clipboard show",
];

/// A `/clipboard` line without the slash, to an edit.
pub fn parse(line: &str) -> Result<Edit, String> {
    let rest = line.trim().strip_prefix("clipboard").ok_or_else(|| "not a clipboard command".to_owned())?;
    let rest = rest.trim();
    let (action, args) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b.trim()));
    let number = |text: &str| -> Result<usize, String> {
        text.split_whitespace()
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .ok_or_else(|| "give an entry number, from 1".to_owned())
    };
    match action {
        "add" if !args.is_empty() => Ok(Edit::Add(args.to_owned())),
        "check" => Ok(Edit::Check(number(args)?)),
        "uncheck" => Ok(Edit::Uncheck(number(args)?)),
        "toggle" => Ok(Edit::Toggle(number(args)?)),
        "remove" => Ok(Edit::Remove(number(args)?)),
        "edit" => {
            let n = number(args)?;
            let text = args.split_once(char::is_whitespace).map_or("", |(_, t)| t.trim());
            if text.is_empty() { Err("give the new text".to_owned()) } else { Ok(Edit::Edit(n, text.to_owned())) }
        }
        "page" => Ok(Edit::Page(number(args)?)),
        "newpage" => Ok(Edit::NewPage),
        "removepage" => Ok(Edit::RemovePage),
        "clear" => Ok(Edit::Clear),
        "show" | "" => Ok(Edit::Show),
        _ => Err(format!("unknown action '{action}'")),
    }
}

/// What an edit did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Reply {
    pub changed: bool,
    pub error: Option<String>,
}

fn fail(message: &str) -> Reply {
    Reply { changed: false, error: Some(message.to_owned()) }
}

impl Content {
    /// The page being shown, creating the first one.
    fn current(&mut self) -> &mut Vec<Entry> {
        if self.pages.is_empty() {
            self.pages.push(Vec::new());
        }
        self.page = self.page.min(self.pages.len() - 1);
        &mut self.pages[self.page]
    }

    pub fn is_empty(&self) -> bool {
        self.pages.iter().all(Vec::is_empty)
    }
}

/// Applies an edit to the content.
pub fn apply(content: &mut Content, edit: Edit) -> Reply {
    if content.read_only && edit != Edit::Show && !matches!(edit, Edit::Page(_)) {
        return fail("This clipboard is read only");
    }
    let ok = Reply { changed: true, error: None };
    match edit {
        Edit::Show => Reply::default(),
        Edit::Add(text) => {
            let page = content.current();
            if page.len() >= MAX_ENTRIES_PER_PAGE {
                return fail("This page is full: /clipboard newpage");
            }
            let text: String = text.chars().take(MAX_TEXT).collect();
            // `ClipboardEntry`: an entry's text starting with `[x]` is already ticked.
            let (checked, text) = match text.strip_prefix("[x]") {
                Some(rest) => (true, rest.trim().to_owned()),
                None => (false, text),
            };
            page.push(Entry { checked, text, icon: None });
            ok
        }
        Edit::Check(n) | Edit::Uncheck(n) | Edit::Toggle(n) | Edit::Remove(n) | Edit::Edit(n, _) => {
            let page = content.current();
            if n > page.len() {
                return fail("No such entry on this page");
            }
            match edit {
                Edit::Check(_) => page[n - 1].checked = true,
                Edit::Uncheck(_) => page[n - 1].checked = false,
                Edit::Toggle(_) => page[n - 1].checked = !page[n - 1].checked,
                Edit::Remove(_) => {
                    page.remove(n - 1);
                }
                Edit::Edit(_, text) => page[n - 1].text = text.chars().take(MAX_TEXT).collect(),
                _ => {}
            }
            ok
        }
        Edit::Page(n) => {
            if n == 0 || n > content.pages.len().max(1) {
                return fail("No such page");
            }
            content.page = n - 1;
            ok
        }
        Edit::NewPage => {
            content.current();
            content.pages.push(Vec::new());
            content.page = content.pages.len() - 1;
            ok
        }
        Edit::RemovePage => {
            if content.pages.len() <= 1 {
                content.pages = vec![Vec::new()];
                content.page = 0;
            } else {
                let at = content.page.min(content.pages.len() - 1);
                content.pages.remove(at);
                content.page = at.min(content.pages.len() - 1);
            }
            ok
        }
        Edit::Clear => {
            *content = Content::default();
            ok
        }
    }
}

/// The open page as lines of text: a heading, then one line a checkbox.
pub fn render(content: &Content) -> Vec<String> {
    let pages = content.pages.len().max(1);
    let page = content.page.min(pages - 1);
    let mut lines = vec![format!("Clipboard, page {}/{}", page + 1, pages)];
    match content.pages.get(page) {
        Some(entries) if !entries.is_empty() => {
            for (i, entry) in entries.iter().enumerate() {
                let item = entry.icon.as_ref().map_or(String::new(), |(id, n)| format!(" ({n} x {id})"));
                lines.push(format!("{}. {} {}{}", i + 1, if entry.checked { "[x]" } else { "[ ]" }, entry.text, item));
            }
        }
        _ => lines.push("(empty: /clipboard add <text>)".to_owned()),
    }
    lines
}

/// `ClipboardBlockItem`'s item or block: `create:clipboard`.
pub fn is_clipboard(id: &str) -> bool {
    id == CLIPBOARD
}

// ---- the placed block's state -----------------------------------------------------------------

/// `AttachFace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Floor,
    Wall,
    Ceiling,
}

impl Face {
    pub fn name(self) -> &'static str {
        match self {
            Face::Floor => "floor",
            Face::Wall => "wall",
            Face::Ceiling => "ceiling",
        }
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` and `ClipboardBlock`: placed on the
/// top of a block it lies on the floor, on the underside it hangs from the ceiling, on a side it
/// stands on the wall facing out. `clicked` is the normal of the face clicked and `player_facing` the
/// horizontal direction the player looks along. The facing is a horizontal name.
pub fn placement(clicked: [i32; 3], player_facing: &str) -> (Face, String) {
    let opposite = |f: &str| match f {
        "north" => "south",
        "south" => "north",
        "east" => "west",
        _ => "east",
    };
    match clicked {
        [0, 1, 0] => (Face::Floor, opposite(player_facing).to_owned()),
        [0, -1, 0] => (Face::Ceiling, opposite(player_facing).to_owned()),
        [0, 0, -1] => (Face::Wall, "north".to_owned()),
        [0, 0, 1] => (Face::Wall, "south".to_owned()),
        [1, 0, 0] => (Face::Wall, "east".to_owned()),
        _ => (Face::Wall, "west".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(content: &mut Content, text: &str) -> Reply {
        apply(content, Edit::Add(text.to_owned()))
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse("clipboard add buy iron"), Ok(Edit::Add("buy iron".into())));
        assert_eq!(parse("clipboard check 2"), Ok(Edit::Check(2)));
        assert_eq!(parse("clipboard edit 3 new words"), Ok(Edit::Edit(3, "new words".into())));
        assert_eq!(parse("clipboard"), Ok(Edit::Show));
        assert_eq!(parse("clipboard page 2"), Ok(Edit::Page(2)));
        assert!(parse("clipboard check").is_err());
        assert!(parse("clipboard check 0").is_err());
        assert!(parse("clipboard add").is_err());
        assert!(parse("clipboard dance").is_err());
    }

    #[test]
    fn entries_are_added_ticked_and_removed() {
        let mut c = Content::default();
        assert!(add(&mut c, "milk").changed);
        add(&mut c, "[x] eggs");
        assert!(c.pages[0][1].checked);
        assert_eq!(c.pages[0][1].text, "eggs");
        assert!(apply(&mut c, Edit::Check(1)).changed);
        assert!(c.pages[0][0].checked);
        apply(&mut c, Edit::Toggle(1));
        assert!(!c.pages[0][0].checked);
        apply(&mut c, Edit::Remove(1));
        assert_eq!(c.pages[0].len(), 1);
        assert!(apply(&mut c, Edit::Check(9)).error.is_some());
    }

    #[test]
    fn a_page_holds_seven_entries_then_a_new_page_is_needed() {
        let mut c = Content::default();
        for i in 0..MAX_ENTRIES_PER_PAGE {
            assert!(add(&mut c, &format!("entry {i}")).changed);
        }
        assert!(add(&mut c, "one more").error.is_some());
        apply(&mut c, Edit::NewPage);
        assert_eq!(c.page, 1);
        assert!(add(&mut c, "one more").changed);
        assert_eq!(c.pages.len(), 2);
        apply(&mut c, Edit::Page(1));
        assert_eq!(c.page, 0);
        assert!(apply(&mut c, Edit::Page(3)).error.is_some());
    }

    #[test]
    fn removing_the_last_page_leaves_an_empty_one() {
        let mut c = Content::default();
        add(&mut c, "a");
        apply(&mut c, Edit::RemovePage);
        assert!(c.is_empty());
        assert_eq!(c.pages.len(), 1);
        apply(&mut c, Edit::NewPage);
        apply(&mut c, Edit::RemovePage);
        assert_eq!(c.pages.len(), 1);
    }

    #[test]
    fn a_read_only_clipboard_can_only_be_read_and_turned() {
        let mut c = Content { read_only: true, ..Content::default() };
        assert!(add(&mut c, "no").error.is_some());
        assert!(!apply(&mut c, Edit::Show).changed);
        c.pages = vec![vec![], vec![]];
        assert!(apply(&mut c, Edit::Page(2)).changed);
        assert!(apply(&mut c, Edit::Clear).error.is_some());
    }

    #[test]
    fn the_page_renders_a_checkbox_a_line() {
        let mut c = Content::default();
        assert_eq!(render(&c).len(), 2);
        add(&mut c, "milk");
        apply(&mut c, Edit::Check(1));
        add(&mut c, "eggs");
        let lines = render(&c);
        assert_eq!(lines[0], "Clipboard, page 1/1");
        assert_eq!(lines[1], "1. [x] milk");
        assert_eq!(lines[2], "2. [ ] eggs");
    }

    #[test]
    fn clearing_empties_everything() {
        let mut c = Content::default();
        add(&mut c, "a");
        apply(&mut c, Edit::NewPage);
        apply(&mut c, Edit::Clear);
        assert_eq!(c, Content::default());
    }

    #[test]
    fn long_text_is_cut() {
        let mut c = Content::default();
        add(&mut c, &"x".repeat(200));
        assert_eq!(c.pages[0][0].text.chars().count(), MAX_TEXT);
    }

    #[test]
    fn placement_picks_the_surface() {
        assert_eq!(placement([0, 1, 0], "north"), (Face::Floor, "south".into()));
        assert_eq!(placement([0, -1, 0], "east"), (Face::Ceiling, "west".into()));
        assert_eq!(placement([0, 0, -1], "south"), (Face::Wall, "north".into()));
        assert_eq!(placement([1, 0, 0], "west"), (Face::Wall, "east".into()));
        assert_eq!(placement([-1, 0, 0], "east"), (Face::Wall, "west".into()));
    }
}

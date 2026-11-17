//! Writing to the block a Display Link points at (`DisplayTarget` and the targets under
//! `displayLink/target`): signs, lecterns, nixie tubes and display boards. What goes where is
//! `create_kinetics::rules::display_link` and `flap_display`; this reads and changes the blocks.
//!
//! A sign or a lectern is a vanilla block entity, which a mod can't open: it is read with
//! `Host::block_entity_tag` and written with `Host::set_block_entity` (API 627, as `/data merge`
//! does). The lines a link has written first on a sign or lectern are remembered here, not in the
//! block entity's persistent data as Create does, so they are forgotten when the game stops.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::Pos;
use create_kinetics::rules::display_link::{
    Config, Reading, Source, Stats, Target, lectern_pages, provide_text, sign_writes,
};
use crate::compat::{Compound, Hooks, Host, Tag};

use super::{flap_display, nixie_tube};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const LINK: &str = "create:display_link";

/// Who wrote the first line of each sign or lectern run (`DisplayTarget.reserve`): the target's
/// dimension, position and line, then the link.
static RESERVED: Mutex<BTreeMap<(String, Pos, usize), Pos>> = Mutex::new(BTreeMap::new());

fn reserve(dimension: &str, target: Pos, line: usize, link: Pos) {
    if line == 0 {
        return;
    }
    if let Ok(mut map) = RESERVED.lock() {
        map.insert((dimension.to_owned(), target, line), link);
    }
}

/// `DisplayTarget.isReserved`: another link, still standing, wrote the line.
fn reserved(host: &dyn Host, dimension: &str, target: Pos, line: usize, link: Pos) -> bool {
    let Ok(mut map) = RESERVED.lock() else {
        return false;
    };
    let key = (dimension.to_owned(), target, line);
    match map.get(&key).copied() {
        Some(other) if other != link && host.block(other) == LINK => true,
        Some(_) => {
            map.remove(&key);
            false
        }
        None => false,
    }
}

/// `DisplayTarget.get(level, pos)`.
pub fn target_at(host: &dyn Host, pos: Pos) -> Option<Target> {
    Target::of_block(&host.block(pos), false)
}

/// `provideStats` of the target at `pos`.
pub fn stats(host: &dyn Host, pos: Pos, target: Target) -> Stats {
    match target {
        Target::Sign => Stats::SIGN,
        Target::Lectern => Stats::LECTERN,
        Target::DisplayBoard => flap_display::stats(host, pos),
        Target::NixieTube => Stats { rows: 1, columns: nixie_tube::row_of(host, pos).len() * 2 },
    }
}

/// `DisplaySource.transferData`: gives a target the text a source made of its reading.
#[allow(clippy::too_many_arguments)]
pub fn deliver(
    host: &mut dyn Host,
    link: Pos,
    target_pos: Pos,
    target: Target,
    line: usize,
    source: Source,
    config: &mut Config,
    reading: &Reading,
) {
    let stats = stats(&*host, target_pos, target);
    match target {
        Target::DisplayBoard => {
            flap_display::deliver(host, link, target_pos, line, source, config, reading);
        }
        Target::NixieTube => {
            let text = provide_text(source, config, reading, &stats, target);
            let first = text.into_iter().next().unwrap_or_default();
            for (index, at) in nixie_tube::row_of(&*host, target_pos).into_iter().enumerate() {
                nixie_tube::display_text(host, at, &first, index);
            }
        }
        Target::Sign => {
            let text = provide_text(source, config, reading, &stats, target);
            write_sign(host, link, target_pos, line, &text);
        }
        Target::Lectern => {
            let text = provide_text(source, config, reading, &stats, target);
            write_lectern(host, link, target_pos, line, &text);
        }
    }
}

// --- signs -----------------------------------------------------------------------------------------

fn string_tag(text: &str) -> Tag {
    Tag::String(text.into())
}

/// One side of a sign with line `index` set to `content`, as `SignText.setMessage` does: colour and
/// glow stay as they were.
fn sign_side(side: Option<&Tag>, writes: &[(usize, String)]) -> Tag {
    let mut compound = match side {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    let mut messages: Vec<Tag> = match compound.get(&"messages".into()) {
        Some(Tag::List(list)) if list.len() == 4 => list.clone(),
        _ => vec![string_tag(""); 4],
    };
    for (index, content) in writes {
        messages[*index] = string_tag(content);
    }
    compound.insert("messages".into(), Tag::List(messages));
    // Text typed in the game keeps a filtered copy; a message changed here has none to keep.
    compound.remove(&"filtered_messages".into());
    Tag::Compound(compound)
}

/// `SignDisplayTarget.acceptText`.
fn write_sign(host: &mut dyn Host, link: Pos, pos: Pos, line: usize, text: &[String]) {
    let dimension = host.dimension();
    let Some(Tag::Compound(mut tag)) = host.block_entity_tag(dimension, pos) else {
        return;
    };
    let writes = sign_writes(
        text,
        line,
        |l| reserve(dimension, pos, l, link),
        |l| reserved(&*host, dimension, pos, l, link),
    );
    if writes.is_empty() {
        return;
    }
    for side in ["front_text", "back_text"] {
        let updated = sign_side(tag.get(&side.into()), &writes);
        tag.insert(side.into(), updated);
    }
    host.set_block_entity(dimension, pos, &to_snbt(&Tag::Compound(tag)));
}

// --- lecterns --------------------------------------------------------------------------------------

/// A page's text, from a plain string or a `{raw}` compound.
fn page_text(page: &Tag) -> String {
    match page {
        Tag::String(text) => text.to_string_lossy(),
        Tag::Compound(c) => match c.get(&"raw".into()) {
            Some(Tag::String(text)) => text.to_string_lossy(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn component<'a>(book: &'a Compound, name: &str) -> Option<&'a Compound> {
    match book.get(&"components".into()) {
        Some(Tag::Compound(components)) => match components.get(&name.into()) {
            Some(Tag::Compound(content)) => Some(content),
            _ => None,
        },
        _ => None,
    }
}

fn raw(text: &str) -> Tag {
    Tag::compound([("raw", string_tag(text))])
}

/// `LecternDisplayTarget.acceptText`: pages of the book on the lectern; a writable book is signed
/// ("Printed Book" by "Data Gatherer") first.
fn write_lectern(host: &mut dyn Host, link: Pos, pos: Pos, line: usize, text: &[String]) {
    let dimension = host.dimension();
    let Some(Tag::Compound(tag)) = host.block_entity_tag(dimension, pos) else {
        return;
    };
    let Some(Tag::Compound(book)) = tag.get(&"Book".into()) else {
        return;
    };
    let id = match book.get(&"id".into()) {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => return,
    };
    let (pages, writable): (Vec<Tag>, bool) = match id.as_str() {
        "minecraft:writable_book" => (
            match component(book, "minecraft:writable_book_content").and_then(|c| c.get(&"pages".into())) {
                Some(Tag::List(pages)) => pages.clone(),
                _ => Vec::new(),
            },
            true,
        ),
        "minecraft:written_book" => (
            match component(book, "minecraft:written_book_content").and_then(|c| c.get(&"pages".into())) {
                Some(Tag::List(pages)) => pages.clone(),
                _ => Vec::new(),
            },
            false,
        ),
        _ => return,
    };
    let current: Vec<String> = pages.iter().map(page_text).collect();
    let Some(next) = lectern_pages(
        &current,
        text,
        line,
        |l| reserve(dimension, pos, l, link),
        |l| reserved(&*host, dimension, pos, l, link),
    ) else {
        return;
    };
    // A page that still reads the same keeps its component (its click events and style).
    let pages_tag: Vec<Tag> = next
        .iter()
        .enumerate()
        .map(|(i, page)| match pages.get(i) {
            Some(old) if page_text(old) == *page && !writable => old.clone(),
            _ => raw(page),
        })
        .collect();
    let mut book = book.clone();
    let mut components = match book.get(&"components".into()) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    let mut content = if writable {
        let mut c = Compound::new();
        c.insert("title".into(), raw("Printed Book"));
        c.insert("author".into(), string_tag("Data Gatherer"));
        c
    } else {
        component(&book, "minecraft:written_book_content").cloned().unwrap_or_default()
    };
    content.insert("pages".into(), Tag::List(pages_tag));
    if writable {
        components.remove(&"minecraft:writable_book_content".into());
        book.insert("id".into(), string_tag("minecraft:written_book"));
    }
    components.insert("minecraft:written_book_content".into(), Tag::Compound(content));
    book.insert("components".into(), Tag::Compound(components));
    let mut out = tag.clone();
    out.insert("Book".into(), Tag::Compound(book));
    host.set_block_entity(dimension, pos, &to_snbt(&Tag::Compound(out)));
}

// --- SNBT --------------------------------------------------------------------------------------------

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// A tag as SNBT (what `/data merge` takes). Horizon doesn't hand a mod its own writer.
pub fn to_snbt(tag: &Tag) -> String {
    match tag {
        Tag::End => String::new(),
        Tag::Byte(v) => format!("{v}b"),
        Tag::Short(v) => format!("{v}s"),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => format!("{v}L"),
        Tag::Float(bits) => format!("{:?}f", f32::from_bits(*bits)),
        Tag::Double(bits) => format!("{:?}d", f64::from_bits(*bits)),
        Tag::String(text) => quote(&text.to_string_lossy()),
        Tag::ByteArray(a) => format!("[B;{}]", a.iter().map(|v| format!("{}B", *v as i8)).collect::<Vec<_>>().join(",")),
        Tag::IntArray(a) => format!("[I;{}]", a.iter().map(i32::to_string).collect::<Vec<_>>().join(",")),
        Tag::LongArray(a) => format!("[L;{}]", a.iter().map(|v| format!("{v}L")).collect::<Vec<_>>().join(",")),
        Tag::List(list) => format!("[{}]", list.iter().map(to_snbt).collect::<Vec<_>>().join(",")),
        Tag::Compound(c) => {
            let fields: Vec<String> =
                c.iter().map(|(k, v)| format!("{}:{}", quote(&k.to_string_lossy()), to_snbt(v))).collect();
            format!("{{{}}}", fields.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snbt_quotes_and_nests() {
        let tag = Tag::compound([
            ("messages", Tag::List(vec![string_tag("a \"b\""), string_tag("")])),
            ("is_waxed", Tag::Byte(0)),
            ("n", Tag::Int(3)),
            ("at", Tag::IntArray(vec![1, 2, 3])),
        ]);
        let text = to_snbt(&tag);
        assert!(text.contains("\"messages\":[\"a \\\"b\\\"\",\"\"]"), "{text}");
        assert!(text.contains("\"is_waxed\":0b"));
        assert!(text.contains("\"at\":[I;1,2,3]"));
        assert!(text.starts_with('{') && text.ends_with('}'));
    }

    #[test]
    fn a_sign_side_keeps_its_colour_and_other_lines() {
        let side = Tag::compound([
            ("messages", Tag::List(vec![string_tag("a"), string_tag("b"), string_tag("c"), string_tag("d")])),
            ("color", string_tag("red")),
            ("has_glowing_text", Tag::Byte(1)),
        ]);
        let Tag::Compound(out) = sign_side(Some(&side), &[(1, "new".to_owned())]) else {
            panic!("a compound");
        };
        assert_eq!(out.get(&"color".into()), Some(&string_tag("red")));
        let Some(Tag::List(messages)) = out.get(&"messages".into()) else {
            panic!("messages");
        };
        assert_eq!(messages, &vec![string_tag("a"), string_tag("new"), string_tag("c"), string_tag("d")]);
        // A sign with nothing yet gets four lines.
        let Tag::Compound(fresh) = sign_side(None, &[(0, "x".to_owned())]) else {
            panic!("a compound");
        };
        assert!(matches!(fresh.get(&"messages".into()), Some(Tag::List(l)) if l.len() == 4));
    }

    #[test]
    fn pages_read_as_strings_or_raw_compounds() {
        assert_eq!(page_text(&string_tag("one")), "one");
        assert_eq!(page_text(&raw("two")), "two");
    }
}

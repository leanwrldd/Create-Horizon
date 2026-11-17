//! The Clipboard (`ClipboardBlockItem`, `ClipboardBlock`, `ClipboardBlockEntity`, `ClipboardContent`):
//! pages of checklist entries that an item carries and a block on a floor, wall or ceiling keeps.
//! The pages and what can be done to them are `create_kinetics::rules::clipboard`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - **No clipboard screen** (`ClipboardScreen`: 870 lines of editing UI Horizon doesn't give a
//!   mod). A right click with the clipboard in hand, or on a placed one, writes the open page into
//!   the chat; the pages are edited with `/clipboard add|check|uncheck|toggle|remove|edit|page|
//!   newpage|removepage|clear|show`. The command works on the clipboard in the main hand, else on
//!   the placed one right-clicked last.
//! - Entries are text and a checkbox: the item icon of an entry (`ClipboardEntry.icon`) is kept when
//!   present but there is no way to set it; the text is saved as a plain string (a valid text
//!   component). A line of text starting with `[x]` is added ticked.
//! - No copying of block settings between clipboards and blocks (`ClipboardCloneable`,
//!   `ClipboardValueSettingsHandler`): the port has no common settings storage for it. No
//!   address-box completion, no `clipboard_type` model overrides (the item always uses the plain
//!   model), no waterlogging, no `canSurvive` (a placed clipboard stays if its support goes).
//! - The recipe that clears a clipboard (`clipboard_clear`) is Create's: one clipboard in a crafting
//!   grid makes a fresh one.
//! - Taking a placed clipboard: a left click or a sneaking right click gives it back with its pages
//!   (an explosion's drops get them if the block entity can still be read: untried).
//! - Edits of a held clipboard are written to the stack on the next inventory tick (Horizon can't
//!   change a held stack from the command itself).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::clipboard::{self as rules, Content, Edit, Entry};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockStateId, Compound, ComponentTypesEvent, Direction, Flow, Hooks, Host,
    InventoryTickEvent, ItemStack, LeftClickBlockEvent, LevelHost, LootDropsEvent, NativeCommand,
    PerformCommandEvent, PlaceBlockEvent, RegisterCommandsEvent, RightClickBlockEvent,
    StateForPlacementEvent, Tag, Text, UseItemEvent, UseItemOnEvent,
};

use crate::util;

const ID: &str = "create:clipboard";
const CONTENT: &str = "Content";

struct Clipboards;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Clipboards)
}

// ---- saved form (`ClipboardContent.CODEC`) ----------------------------------------------------

fn tag_of(content: &Content) -> Compound {
    let pages = content
        .pages
        .iter()
        .map(|page| {
            Tag::List(
                page.iter()
                    .map(|entry| {
                        let mut tag = Compound::new();
                        tag.insert("checked".into(), Tag::Byte(i8::from(entry.checked)));
                        tag.insert("text".into(), Tag::String(entry.text.as_str().into()));
                        if let Some((icon, amount)) = &entry.icon {
                            let mut stack = Compound::new();
                            stack.insert("id".into(), Tag::String(icon.as_str().into()));
                            stack.insert("count".into(), Tag::Int(1));
                            tag.insert("icon".into(), Tag::Compound(stack));
                            tag.insert("item_amount".into(), Tag::Int(*amount));
                        }
                        Tag::Compound(tag)
                    })
                    .collect(),
            )
        })
        .collect();
    let mut tag = Compound::new();
    // `ClipboardType`: written once there is something on it.
    tag.insert("type".into(), Tag::String(if content.is_empty() { "empty_clipboard" } else { "clipboard" }.into()));
    tag.insert("pages".into(), Tag::List(pages));
    tag.insert("read_only".into(), Tag::Byte(i8::from(content.read_only)));
    tag.insert("previously_opened_page".into(), Tag::Int(content.page as i32));
    tag
}

fn content_of(tag: &Tag) -> Content {
    let mut content = Content::default();
    if let Some(Tag::List(pages)) = tag.get("pages") {
        for page in pages {
            let Tag::List(entries) = page else {
                continue;
            };
            content.pages.push(
                entries
                    .iter()
                    .map(|entry| Entry {
                        checked: matches!(entry.get("checked"), Some(Tag::Byte(b)) if *b != 0),
                        text: match entry.get("text") {
                            Some(Tag::String(text)) => text.to_string_lossy(),
                            _ => String::new(),
                        },
                        icon: match (entry.get("icon").and_then(|icon| icon.get("id")), entry.get("item_amount")) {
                            (Some(Tag::String(id)), Some(Tag::Int(n))) => Some((id.to_string_lossy(), *n)),
                            _ => None,
                        },
                    })
                    .collect(),
            );
        }
    }
    content.read_only = matches!(tag.get("read_only"), Some(Tag::Byte(b)) if *b != 0);
    if let Some(Tag::Int(page)) = tag.get("previously_opened_page") {
        content.page = (*page).max(0) as usize;
    }
    content
}

fn carried(stack: &ItemStack) -> Option<Content> {
    stack.data()?.other(rules::COMPONENT).map(content_of)
}

fn say(host: &mut dyn Host, uuid: u128, lines: &[String]) {
    for line in lines {
        host.tell(uuid, Text::literal(line.clone()));
    }
}

// ---- state kept in memory ----------------------------------------------------------------------

/// The clipboard each player last right-clicked where it stands, for the command.
static TARGETS: Mutex<BTreeMap<u128, ([i32; 3], &'static str)>> = Mutex::new(BTreeMap::new());
/// Edits waiting for the next inventory tick to be written to the held stack.
static PENDING: Mutex<BTreeMap<u128, Compound>> = Mutex::new(BTreeMap::new());
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn normal(face: Direction) -> [i32; 3] {
    match face {
        Direction::Up => [0, 1, 0],
        Direction::Down => [0, -1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::East => [1, 0, 0],
        Direction::West => [-1, 0, 0],
    }
}

impl Hooks for Clipboards {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == rules::COMPONENT) {
            event.types.push(rules::COMPONENT.to_owned());
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    /// `ClipboardBlock.getStateForPlacement`: on the floor, a wall or the ceiling, by the face clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if event.context.item != ID {
            return None;
        }
        let player = create_kinetics::looking(event.context.yaw, 0.0);
        let (face, facing) = rules::placement(normal(event.context.face), player.name());
        let state = level.state_with_property(event.state, "face", face.name())?;
        let state = level.state_with_property(state, "facing", &facing)?;
        Some(Some(state))
    }

    /// `ClipboardBlockItem.updateCustomBlockEntityTag`: what the item carries goes into the block.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != ID {
            return;
        }
        let Some(content) = carried(event.stack) else {
            return;
        };
        let pos = event.pos;
        let tag = tag_of(&content);
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(CONTENT.into(), Tag::Compound(tag.clone()));
        });
        host.set_block_property(pos, "written", "true");
    }

    /// `ClipboardBlockItem.useOn`: a click on a block opens the clipboard instead of placing it, unless
    /// sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.sneaking {
            return Flow::Continue;
        }
        if util::first_click(&*host, player.uuid, event.pos) {
            let content = carried(event.stack).unwrap_or_default();
            say(host, player.uuid, &rules::render(&content));
        }
        Flow::Cancel
    }

    /// `ClipboardBlockItem.use`: in the air too.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if util::click_repeated_anywhere(&*host, player.uuid) {
            return Flow::Cancel;
        }
        util::click_taken(&*host, player.uuid, [0, i32::MIN, 0]);
        let content = carried(event.stack).unwrap_or_default();
        say(host, player.uuid, &rules::render(&content));
        Flow::Cancel
    }

    /// `ClipboardBlock.useWithoutItem`: a right click shows the page; sneaking picks it up.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        if host.held_stack(player.uuid).is_some_and(|held| &*held.item == util::WRENCH) {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if player.sneaking {
            collect(host, player.uuid, pos);
            return Flow::Cancel;
        }
        lock(&TARGETS).insert(player.uuid, (pos, host.dimension()));
        let content = host.mod_block_entity(pos).map(|e| content_at(&e.nbt)).unwrap_or_default();
        say(host, player.uuid, &rules::render(&content));
        Flow::Cancel
    }

    /// `ClipboardBlock.attack`: a left click picks it up.
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        collect(host, player.uuid, pos);
        Flow::Cancel
    }

    /// An explosion's drops: the pages, if the block entity can still be read.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let Some((pos, _)) = event.context.block else {
            return;
        };
        for drop in event.drops.iter_mut() {
            if &*drop.item != ID || drop.data().and_then(|d| d.other(rules::COMPONENT)).is_some() {
                continue;
            }
            if let Some(entity) = host.mod_block_entity(pos)
                && let Some(Tag::Compound(tag)) = util::get(&entity.nbt, CONTENT)
            {
                let _ = host.set_component(drop, rules::COMPONENT, Tag::Compound(tag.clone()));
            }
        }
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.extend(rules::USAGES.iter().map(|usage| NativeCommand::new(usage)));
        event.commands.push(NativeCommand::new("clipboard"));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        if line != "clipboard" && !line.starts_with("clipboard ") {
            return Flow::Continue;
        }
        let uuid = event.player.uuid;
        let edit = match rules::parse(line) {
            Ok(edit) => edit,
            Err(message) => {
                say(host, uuid, &[message]);
                return Flow::Cancel;
            }
        };
        let held = host.held_stack(uuid).filter(|stack| &*stack.item == ID);
        if let Some(stack) = held {
            let mut content = carried(&stack).unwrap_or_default();
            let reply = rules::apply(&mut content, edit);
            if let Some(error) = reply.error {
                say(host, uuid, &[error]);
            } else {
                if reply.changed {
                    lock(&PENDING).insert(uuid, tag_of(&content));
                    PENDING_COUNT.store(lock(&PENDING).len(), Ordering::Relaxed);
                }
                say(host, uuid, &rules::render(&content));
            }
            return Flow::Cancel;
        }
        let target = lock(&TARGETS).get(&uuid).copied();
        let Some((pos, dimension)) = target.filter(|(pos, dimension)| *dimension == host.dimension() && host.block(*pos) == ID) else {
            say(host, uuid, &["Hold a clipboard, or right-click a placed one first".to_owned()]);
            return Flow::Cancel;
        };
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Cancel;
        };
        let mut content = content_at(&entity.nbt);
        let reply = rules::apply(&mut content, edit);
        if let Some(error) = reply.error {
            say(host, uuid, &[error]);
        } else {
            if reply.changed {
                let tag = tag_of(&content);
                host.change_mod_block_entity(pos, &mut |e| {
                    e.nbt.insert(CONTENT.into(), Tag::Compound(tag.clone()));
                });
                host.set_block_property(pos, "written", if content.is_empty() { "false" } else { "true" });
            }
            say(host, uuid, &rules::render(&content));
        }
        let _ = dimension;
        Flow::Cancel
    }

    /// Writes an edit of the held clipboard to the stack.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || PENDING_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(tag) = lock(&PENDING).get(&player.uuid).cloned() else {
            return;
        };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else {
            return;
        };
        if &*stack.item != ID {
            return;
        }
        let _ = host.set_component(&mut stack, rules::COMPONENT, Tag::Compound(tag));
        if let Some(slot) = host.carried_stack(event.slot) {
            *slot = stack;
        }
        let mut pending = lock(&PENDING);
        pending.remove(&player.uuid);
        PENDING_COUNT.store(pending.len(), Ordering::Relaxed);
    }
}

fn content_at(nbt: &Compound) -> Content {
    match util::get(nbt, CONTENT) {
        Some(tag @ Tag::Compound(_)) => content_of(tag),
        _ => Content::default(),
    }
}

/// `ClipboardBlock.breakAndCollect`: the block goes and the player gets the item with its pages.
fn collect(host: &mut dyn Host, uuid: u128, pos: [i32; 3]) {
    let mut stack = ItemStack { item: ID.into(), count: 1, damage: 0, extra: None };
    if let Some(entity) = host.mod_block_entity(pos)
        && let Some(Tag::Compound(tag)) = util::get(&entity.nbt, CONTENT)
    {
        let _ = host.set_component(&mut stack, rules::COMPONENT, Tag::Compound(tag.clone()));
    }
    let dimension = host.dimension();
    let Some(air) = host.default_state("minecraft:air") else {
        return;
    };
    if !host.set_block(dimension, pos, air, util::UPDATE_ALL) {
        return;
    }
    host.play_sound("minecraft:block.wood.break", util::centre(pos), 4, 1.0, 1.0);
    if let Some(rest) = host.give_stack(uuid, stack) {
        util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
    }
}

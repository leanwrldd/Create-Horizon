//! The Schematic and Quill, the Empty Schematic and the Schematic (`SchematicAndQuillItem`,
//! `SchematicAndQuillHandler`, `SchematicItem`, `SchematicHandler`, `SchematicTransformation`, the
//! tools in `client/tools`): select a box and save it as a structure file, and position a written
//! Schematic in the world for a Schematicannon to print. The rules are
//! `create_kinetics::rules::schematic_*`; the hologram is drawn by `schematic_view.rs`; the table
//! and the cannon are `schematic_table.rs` and `schematic_cannon.rs`.
//!
//! Known differences from Create, for the README (the stand-ins for its screens, key presses and
//! scrolling):
//! - **Saving.** Create asks for a name in a screen. Here two right clicks on blocks set the two
//!   corners (a click on the top or bottom of a block that isn't replaceable selects the block beyond
//!   it, as Create does), a third saves the box under the name "My Schematic" (`my_schematic.nbt`,
//!   then `my_schematic_1.nbt`...) and `/schematic save <name>` saves under a name of your own.
//!   Sneaking with the item discards the selection; `/schematic expand <n>` grows the box by n blocks
//!   the way you look (negative shrinks), standing in for Ctrl-scroll on a face. There is no selecting
//!   in the air.
//! - **Positioning.** A right click on a block with a written Schematic puts its footprint's centre
//!   there (`DeployTool`), again to move it; sneaking with it turns it a quarter turn clockwise about its
//!   centre (`RotateTool`). `/schematic rotate [cw|ccw]`, `flip x|z`, `move <x> <y> <z>`, `undeploy`
//!   and `info` do the rest of the tool belt (Mirror, Move XZ, Move Y), and `/schematic print` places it
//!   at once in Creative (`PlaceTool`). The edit goes on the stack at the next inventory tick.
//! - **The hologram** is drawn as translucent coloured boxes in the blocks' map colours, with the
//!   box outlined in Create's blue (`schematic_view.rs`), not as the blocks' models.
//! - A Schematic stores its data in one component, `create:schematic` (`file`, `owner`, `deployed`,
//!   `anchor`, `rotation`, `mirror`, `bounds`: Create's separate `schematic_*` components), and it
//!   is never "uploaded" (see `schematic_files.rs`); `owner` is kept for the tooltip only.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::rules::contraption_facts::REPLACEABLE;
use create_kinetics::rules::schematic_structure::{FALLBACK_NAME, Pos};
use create_kinetics::rules::schematic_table::block_takes_click;
use create_kinetics::rules::schematic_transform::{Mirror, Rotation, Transformation};
use create_kinetics::looking;
use horizon_client::mod_api::{
    ComponentTypesEvent, Flow, Hooks, Host, InventoryTickEvent, ItemStack, NativeCommand, PerformCommandEvent,
    PlayerTickEvent, RegisterCommandsEvent, Tag, Text, TooltipEvent, UseItemEvent, UseItemOnEvent,
    serde_json::json,
};

use super::{schematic_files as files, schematic_world as world};
use crate::util;

pub const SCHEMATIC: &str = "create:schematic";
pub const EMPTY: &str = "create:empty_schematic";
pub const QUILL: &str = "create:schematic_and_quill";
pub const CANNON: &str = "create:schematicannon";
pub const TABLE: &str = "create:schematic_table";
/// The data component a written Schematic carries.
pub const COMPONENT: &str = "create:schematic";
/// The channel the server tells a client what to draw on.
pub const CHANNEL: &str = "create:schematic";

struct Schematics;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Schematics)
}

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- the data a Schematic carries ------------------------------------------------------------------

/// What `SCHEMATIC_FILE`, `_OWNER`, `_DEPLOYED`, `_ANCHOR`, `_ROTATION`, `_MIRROR` and `_BOUNDS`
/// hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    pub file: String,
    pub owner: String,
    pub deployed: bool,
    pub anchor: Pos,
    pub rotation: Rotation,
    pub mirror: Mirror,
    /// The template's size, before turning.
    pub bounds: Pos,
}

fn ints(tag: Option<&Tag>) -> Pos {
    match tag {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => [0; 3],
    }
}

fn text_of(tag: Option<&Tag>) -> String {
    match tag {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

impl Data {
    pub fn to_tag(&self) -> Tag {
        let mut tag = horizon_client::mod_api::Compound::new();
        tag.insert("file".into(), Tag::String(self.file.as_str().into()));
        tag.insert("owner".into(), Tag::String(self.owner.as_str().into()));
        tag.insert("deployed".into(), Tag::Byte(i8::from(self.deployed)));
        tag.insert("anchor".into(), Tag::IntArray(self.anchor.to_vec()));
        tag.insert("rotation".into(), Tag::String(self.rotation.name().into()));
        tag.insert("mirror".into(), Tag::String(self.mirror.name().into()));
        tag.insert("bounds".into(), Tag::IntArray(self.bounds.to_vec()));
        Tag::Compound(tag)
    }

    pub fn from_tag(tag: &Tag) -> Option<Data> {
        let file = text_of(tag.get("file"));
        if file.is_empty() {
            return None;
        }
        Some(Data {
            file,
            owner: text_of(tag.get("owner")),
            deployed: matches!(tag.get("deployed"), Some(Tag::Byte(b)) if *b != 0),
            anchor: ints(tag.get("anchor")),
            rotation: Rotation::from_name(&text_of(tag.get("rotation"))).unwrap_or(Rotation::None),
            mirror: Mirror::from_name(&text_of(tag.get("mirror"))).unwrap_or(Mirror::None),
            bounds: ints(tag.get("bounds")),
        })
    }

    /// The tools' transformation of this Schematic.
    pub fn transformation(&self) -> Transformation {
        Transformation::new(self.anchor, self.rotation, self.mirror, self.bounds)
    }

    /// Takes the transformation's anchor and settings back (`SchematicSyncPacket`).
    pub fn set_transformation(&mut self, t: &Transformation) {
        self.anchor = t.anchor();
        let (rotation, mirror) = t.settings();
        self.rotation = rotation;
        self.mirror = mirror;
    }
}

/// The data of a stack, if it is a written Schematic.
pub fn data_of(stack: &ItemStack) -> Option<Data> {
    if &*stack.item != SCHEMATIC {
        return None;
    }
    Data::from_tag(stack.data()?.other(COMPONENT)?)
}

/// `SchematicItem.create`: a Schematic for a file of the folder, not yet deployed. `None` when the
/// file can't be read.
pub fn new_schematic(host: &dyn Host, file: &str, owner: &str) -> Result<ItemStack, String> {
    let template = files::load(host, file)?;
    let data = Data {
        file: file.to_owned(),
        owner: owner.to_owned(),
        deployed: false,
        anchor: [0; 3],
        rotation: Rotation::None,
        mirror: Mirror::None,
        bounds: template.size,
    };
    let mut stack = ItemStack { item: SCHEMATIC.into(), count: 1, damage: 0, extra: None };
    host.set_component(&mut stack, COMPONENT, data.to_tag()).map_err(|e| format!("{e:?}"))?;
    Ok(stack)
}

/// Edits waiting for the next inventory tick, to be written to the held Schematic (Horizon can't
/// change the held stack from a click or a command).
static PENDING: Mutex<BTreeMap<u128, Data>> = Mutex::new(BTreeMap::new());

fn queue(uuid: u128, data: Data) {
    lock(&PENDING).insert(uuid, data);
}

/// The held Schematic's data, with an edit still waiting counted.
fn current(host: &dyn Host, uuid: u128) -> Option<Data> {
    let held = host.held_stack(uuid)?;
    let from_stack = data_of(&held)?;
    match lock(&PENDING).get(&uuid) {
        Some(pending) if pending.file == from_stack.file => Some(pending.clone()),
        _ => Some(from_stack),
    }
}

// ---- messages ---------------------------------------------------------------------------------------

fn bar(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.action_bar(uuid, Text::literal(line.into()));
}

fn say(host: &mut dyn Host, uuid: u128, line: impl Into<String>) {
    host.tell(uuid, Text::literal(line.into()));
}

// ---- the Schematic and Quill's selection -----------------------------------------------------------

/// What a player has selected with the Schematic and Quill (`SchematicAndQuillHandler`).
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub dimension: String,
    pub first: Pos,
    pub second: Option<Pos>,
}

static SELECTIONS: Mutex<BTreeMap<u128, Selection>> = Mutex::new(BTreeMap::new());

fn selection_of(uuid: u128) -> Option<Selection> {
    lock(&SELECTIONS).get(&uuid).cloned()
}

/// The block a click selects (`SchematicToolBase.updateTargetPos`): the block clicked, or beyond
/// it when the face is the top or the bottom and the block isn't replaceable.
pub fn picked(host: &dyn Host, pos: Pos, face: [i32; 3]) -> Pos {
    let replaceable = REPLACEABLE.binary_search(&host.block(pos).as_str()).is_ok();
    if face[1] != 0 && !replaceable {
        util::add(pos, face)
    } else {
        pos
    }
}

/// `SchematicAndQuillHandler.saveSchematic`: reads the box and writes the file. The file name.
fn save_selection(host: &mut dyn Host, uuid: u128, name: &str) -> Result<String, String> {
    let Some(Selection { dimension, first, second: Some(second) }) = selection_of(uuid) else {
        return Err("Select two corners with the Schematic And Quill first".to_owned());
    };
    let template = world::export(host, &dimension, first, second)?;
    let file = files::save(host, name, false, &template)?;
    lock(&SELECTIONS).remove(&uuid);
    Ok(file)
}

fn on_quill(host: &mut dyn Host, uuid: u128, sneaking: bool, pos: Pos, face: [i32; 3]) {
    if sneaking {
        if lock(&SELECTIONS).remove(&uuid).is_some() {
            bar(host, uuid, "Removed selection.");
        }
        return;
    }
    let dimension = host.dimension().to_owned();
    let current = selection_of(uuid).filter(|s| s.dimension == dimension);
    match current {
        // A box is set: the third click saves it (Create opens the name screen).
        Some(Selection { second: Some(_), .. }) => match save_selection(host, uuid, FALLBACK_NAME) {
            Ok(file) => bar(host, uuid, format!("Saved as {file}")),
            Err(error) => say(host, uuid, format!("Failed to save the schematic: {error}")),
        },
        Some(Selection { first, .. }) => {
            let second = picked(host, pos, face);
            let size = world::size_of(first, second);
            lock(&SELECTIONS).insert(uuid, Selection { dimension, first, second: Some(second) });
            bar(host, uuid, format!("Second position set. Schematic Size: {}x{}x{}", size[0], size[1], size[2]));
        }
        None => {
            let first = picked(host, pos, face);
            lock(&SELECTIONS).insert(uuid, Selection { dimension, first, second: None });
            bar(host, uuid, "First position set.");
        }
    }
}

// ---- the Schematic's tools --------------------------------------------------------------------------

/// `DeployTool.handleRightClick`: the footprint's centre goes on the block clicked.
fn deploy(host: &mut dyn Host, uuid: u128, pos: Pos, face: [i32; 3]) {
    let Some(mut data) = current(&*host, uuid) else {
        bar(host, uuid, "[!] Invalid Item - Use the Schematic Table instead");
        return;
    };
    let target = picked_for_deploy(&*host, pos, face);
    let mut t = data.transformation();
    t.target = Transformation::deploy_target(target, data.bounds);
    data.set_transformation(&t);
    data.deployed = true;
    bar(host, uuid, format!("Schematic positioned at {}, {}, {}", data.anchor[0], data.anchor[1], data.anchor[2]));
    queue(uuid, data);
}

/// Where a deploying click puts the schematic: above a block's top, against its side, in a
/// replaceable block itself (`SchematicToolBase.updateTargetPos`).
fn picked_for_deploy(host: &dyn Host, pos: Pos, face: [i32; 3]) -> Pos {
    let replaceable = REPLACEABLE.binary_search(&host.block(pos).as_str()).is_ok();
    if replaceable { pos } else { util::add(pos, face) }
}

fn on_schematic(host: &mut dyn Host, uuid: u128, sneaking: bool, pos: Pos, face: [i32; 3]) {
    let target = host.block(pos);
    // The cannon and the table take the Schematic themselves (`SchematicHandler.onMouseInput`).
    if target == CANNON || target == TABLE {
        return;
    }
    if sneaking {
        // `RotateTool`: a quarter turn about the centre.
        rotate(host, uuid, true);
        return;
    }
    deploy(host, uuid, pos, face);
}

fn rotate(host: &mut dyn Host, uuid: u128, clockwise: bool) {
    let Some(mut data) = current(&*host, uuid) else {
        return;
    };
    if !data.deployed {
        bar(host, uuid, "Position the Schematic first (right-click a block)");
        return;
    }
    let mut t = data.transformation();
    t.rotate90(clockwise);
    data.set_transformation(&t);
    bar(host, uuid, format!("Rotation: {}", rotation_text(data.rotation)));
    queue(uuid, data);
}

fn rotation_text(rotation: Rotation) -> &'static str {
    match rotation {
        Rotation::None => "None",
        Rotation::Clockwise90 => "Clockwise 90",
        Rotation::Clockwise180 => "Clockwise 180",
        Rotation::Counterclockwise90 => "Clockwise 270",
    }
}

fn mirror_text(mirror: Mirror) -> &'static str {
    match mirror {
        Mirror::None => "None",
        Mirror::LeftRight => "Left-Right",
        Mirror::FrontBack => "Front-Back",
    }
}

// ---- commands ---------------------------------------------------------------------------------------

const USAGES: [&str; 15] = [
    "schematic list",
    "schematic save",
    "schematic save <name:text>",
    "schematic expand <amount:integer>",
    "schematic write <file:text>",
    "schematic give <file:text>",
    "schematic rotate",
    "schematic rotate <direction:word>",
    "schematic flip <axis:word>",
    "schematic move <x:integer> <y:integer> <z:integer>",
    "schematic undeploy",
    "schematic info",
    "schematic print",
    "schematic help",
    "schematic selection",
];

const HELP: [&str; 10] = [
    "Schematic And Quill: right-click two blocks, then a third time to save as My Schematic; /schematic save <name> to name it; sneak to discard; /schematic expand <n> to grow the box the way you look.",
    "Schematic Table: put an Empty Schematic in it, then /schematic write <file or number> (see /schematic list).",
    "/schematic give <file or number>   a written Schematic of a saved file, without the table (Horizon's /give can't carry one)",
    "Schematic: right-click a block to position it, sneak-right-click to turn it a quarter turn.",
    "/schematic rotate [cw|ccw]   turn it 90 degrees about its centre",
    "/schematic flip x|z          mirror it (x swaps east and west, z swaps north and south)",
    "/schematic move <x> <y> <z>  shift it by blocks",
    "/schematic undeploy | info   take it back off the world | its file, place and turn",
    "/schematic print             Creative only: place it at once",
    "/schematicannon status|checklist|play|pause|stop|replace|skip|protect   (see /schematicannon help)",
];

fn parse_flip(axis: &str) -> Option<bool> {
    // true: x (east-west, `flip(Axis.X)`); false: z.
    match axis.to_lowercase().as_str() {
        "x" | "lr" | "left_right" => Some(true),
        "z" | "fb" | "front_back" => Some(false),
        _ => None,
    }
}

fn command(host: &mut dyn Host, uuid: u128, words: &[&str], text: &str, held: Option<ItemStack>) {
    let sub = words.first().copied().unwrap_or("help");
    match sub {
        "help" => {
            for line in HELP {
                say(host, uuid, line);
            }
        }
        "list" => {
            let names = files::list(&*host);
            if names.is_empty() {
                let folder = files::folder(&*host).display().to_string();
                say(host, uuid, format!("No schematics saved in {folder}"));
            }
            for (i, name) in names.iter().enumerate() {
                say(host, uuid, format!("{}. {name}", i + 1));
            }
        }
        "selection" => match selection_of(uuid) {
            Some(Selection { first, second: Some(second), .. }) => {
                let size = world::size_of(first, second);
                say(host, uuid, format!("Selection {first:?} to {second:?}: {}x{}x{}", size[0], size[1], size[2]));
            }
            Some(Selection { first, .. }) => say(host, uuid, format!("First position {first:?}; the second isn't set")),
            None => say(host, uuid, "Nothing is selected"),
        },
        "save" => {
            let name = text.trim().strip_prefix("save").unwrap_or("").trim();
            match save_selection(host, uuid, name) {
                Ok(file) => say(host, uuid, format!("Saved as {file}")),
                Err(error) => say(host, uuid, format!("Failed to save the schematic: {error}")),
            }
        }
        "expand" => expand(host, uuid, words.get(1).and_then(|n| n.parse().ok())),
        "write" => {
            let name = text.trim().strip_prefix("write").unwrap_or("").trim();
            super::schematic_table::write(host, uuid, name);
        }
        "give" => {
            let name = text.trim().strip_prefix("give").unwrap_or("").trim();
            super::schematic_table::give(host, uuid, name);
        }
        "rotate" => {
            let clockwise = !matches!(words.get(1).map(|w| w.to_lowercase()).as_deref(), Some("ccw" | "counterclockwise" | "left"));
            if held_schematic(host, uuid, &held) {
                rotate(host, uuid, clockwise);
            }
        }
        "flip" => {
            let Some(x) = words.get(1).and_then(|a| parse_flip(a)) else {
                say(host, uuid, "Usage: /schematic flip x|z");
                return;
            };
            if !held_schematic(host, uuid, &held) {
                return;
            }
            if let Some(mut data) = current(&*host, uuid) {
                let mut t = data.transformation();
                if x { t.flip_x() } else { t.flip_z() }
                data.set_transformation(&t);
                bar(host, uuid, format!("Mirror: {}", mirror_text(data.mirror)));
                queue(uuid, data);
            }
        }
        "move" => {
            let n: Vec<i32> = words[1..].iter().filter_map(|w| w.parse().ok()).collect();
            let [x, y, z] = n[..] else {
                say(host, uuid, "Usage: /schematic move <x> <y> <z>");
                return;
            };
            if !held_schematic(host, uuid, &held) {
                return;
            }
            if let Some(mut data) = current(&*host, uuid) {
                let mut t = data.transformation();
                t.shift([x, y, z]);
                data.set_transformation(&t);
                bar(host, uuid, format!("Schematic moved to {}, {}, {}", data.anchor[0], data.anchor[1], data.anchor[2]));
                queue(uuid, data);
            }
        }
        "undeploy" => {
            if !held_schematic(host, uuid, &held) {
                return;
            }
            if let Some(mut data) = current(&*host, uuid) {
                data.deployed = false;
                queue(uuid, data);
                bar(host, uuid, "Schematic taken off the world");
            }
        }
        "info" => {
            let Some(held_data) = held.as_ref().and_then(data_of) else {
                say(host, uuid, "Hold a written Schematic");
                return;
            };
            let data = current(&*host, uuid).unwrap_or(held_data);
            say(host, uuid, format!("{} ({}x{}x{})", data.file, data.bounds[0], data.bounds[1], data.bounds[2]));
            if data.deployed {
                say(
                    host,
                    uuid,
                    format!(
                        "Positioned at {}, {}, {}; rotation {}, mirror {}",
                        data.anchor[0], data.anchor[1], data.anchor[2],
                        rotation_text(data.rotation),
                        mirror_text(data.mirror)
                    ),
                );
            } else {
                say(host, uuid, "Not positioned yet (right-click a block)");
            }
        }
        "print" => print(host, uuid),
        other => say(host, uuid, format!("Unknown: schematic {other}. /schematic help lists what there is.")),
    }
}

/// Whether the player holds a written Schematic (else says so).
fn held_schematic(host: &mut dyn Host, uuid: u128, held: &Option<ItemStack>) -> bool {
    if held.as_ref().and_then(data_of).is_none() {
        say(host, uuid, "Hold a written Schematic");
        return false;
    }
    true
}

/// `/schematic expand`: the box grows the way the player looks.
fn expand(host: &mut dyn Host, uuid: u128, amount: Option<i32>) {
    let Some(amount) = amount else {
        say(host, uuid, "Usage: /schematic expand <blocks>");
        return;
    };
    let (Some(Selection { dimension, first, second: Some(second) }), Some(player)) =
        (selection_of(uuid), host.acting_player())
    else {
        say(host, uuid, "Select two corners first");
        return;
    };
    let (mut low, mut high) = world::corners(first, second);
    let look = looking(player.yaw, player.pitch);
    let axis = match look.axis() {
        create_kinetics::Axis::X => 0,
        create_kinetics::Axis::Y => 1,
        create_kinetics::Axis::Z => 2,
    };
    if look.sign() > 0 {
        high[axis] = (high[axis] + amount).max(low[axis]);
    } else {
        low[axis] = (low[axis] - amount).min(high[axis]);
    }
    let size = world::size_of(low, high);
    lock(&SELECTIONS).insert(uuid, Selection { dimension, first: low, second: Some(high) });
    bar(host, uuid, format!("Schematic Size: {}x{}x{}", size[0], size[1], size[2]));
}

/// `SchematicPlacePacket`, the Print tool: Creative only, the whole schematic at once.
fn print(host: &mut dyn Host, uuid: u128) {
    let Some(player) = host.acting_player() else {
        return;
    };
    if player.game_mode != util::CREATIVE {
        say(host, uuid, "/schematic print is for Creative mode only");
        return;
    }
    let Some(data) = current(&*host, uuid).filter(|d| d.deployed) else {
        say(host, uuid, "Hold a positioned Schematic");
        return;
    };
    let template = match files::load(&*host, &data.file) {
        Ok(t) => t,
        Err(error) => {
            say(host, uuid, format!("Schematic failed to Load: {error}"));
            return;
        }
    };
    let layout = create_kinetics::rules::schematic_printer::Layout::new(&template, data.rotation, data.mirror);
    let mut placed = 0;
    let positions: Vec<Pos> = layout.positions().collect();
    for rel in positions {
        let state = layout.state(rel);
        // `CSchematics.creativePrintIncludesAir` is false: air in the file leaves the world as it is.
        if create_kinetics::rules::schematic_printer::is_air(&state) || state.name == "minecraft:structure_void" {
            continue;
        }
        let target = util::add(data.anchor, rel);
        let data_tag = layout
            .nbt(rel)
            .and_then(|nbt| create_kinetics::rules::schematic_cannon::safe_block_entity_data(&state, nbt));
        if world::place_schematic_block(host, target, &state, data_tag.as_ref()) {
            placed += 1;
        }
    }
    say(host, uuid, format!("Placed {placed} blocks"));
}

// ---- the hologram's data for the client ------------------------------------------------------------

/// Each player's last message, so a change is sent once.
static SENT: Mutex<BTreeMap<u128, String>> = Mutex::new(BTreeMap::new());

fn view_message(host: &dyn Host, uuid: u128) -> String {
    let held = host.held_stack(uuid);
    let holo = held.as_ref().and_then(data_of).map(|d| {
        let data = match lock(&PENDING).get(&uuid) {
            Some(pending) if pending.file == d.file => pending.clone(),
            _ => d,
        };
        json!({
            "file": data.file,
            "deployed": data.deployed,
            "anchor": data.anchor,
            "rotation": data.rotation.name(),
            "mirror": data.mirror.name(),
            "bounds": data.bounds,
        })
    });
    let quill = held.as_ref().is_some_and(|s| &*s.item == QUILL);
    let selection = if quill {
        selection_of(uuid).map(|s| json!({ "a": s.first, "b": s.second }))
    } else {
        None
    };
    json!({ "holo": holo, "sel": selection }).to_string()
}

impl Hooks for Schematics {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == COMPONENT) {
            event.types.push(COMPONENT.to_owned());
        }
    }

    /// `SchematicItem.appendHoverText`.
    fn tooltip(&self, _: &dyn Host, event: &mut TooltipEvent<'_>) {
        if &*event.stack.item != SCHEMATIC {
            return;
        }
        match data_of(event.stack) {
            Some(data) => event.lines.push((Text::literal(data.file), 0xFFFF_AA00)),
            None => event
                .lines
                .push((Text::literal("[!] Invalid Item - Use the Schematic Table instead".to_owned()), 0xFFFF_5555)),
        }
    }

    /// The Schematic and Quill's corners and the Schematic's positioning, by right-clicking a block.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let item = &*event.stack.item;
        if item != QUILL && item != SCHEMATIC {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let (pos, face) = (event.pos, event.face);
        let target = host.block(pos);
        // The cannon and the table take the click themselves (they open their screens, and a
        // block's use comes before the item's), the Quill and the Schematic alike.
        if block_takes_click(&target) {
            return Flow::Continue;
        }
        if !util::first_click(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        if item == QUILL {
            on_quill(host, player.uuid, player.sneaking, pos, face);
        } else {
            on_schematic(host, player.uuid, player.sneaking, pos, face);
        }
        Flow::Cancel
    }

    /// A click in the air with either: nothing (Create opens its screens there).
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        let item = &*event.stack.item;
        if item != QUILL && item != SCHEMATIC {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if util::click_repeated_anywhere(&*host, player.uuid) {
            return Flow::Cancel;
        }
        if item == QUILL && player.sneaking {
            if lock(&SELECTIONS).remove(&player.uuid).is_some() {
                bar(host, player.uuid, "Removed selection.");
            }
        } else if item == QUILL {
            bar(host, player.uuid, "Click a block to select it (/schematic help)");
        }
        Flow::Cancel
    }

    fn register_commands(&self, _: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.extend(USAGES.iter().map(|usage| NativeCommand::new(usage)));
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let line = event.command.trim();
        let Some(rest) = line.strip_prefix("schematic") else {
            return Flow::Continue;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return Flow::Continue;
        }
        let uuid = event.player.uuid;
        let rest = rest.trim();
        let words: Vec<&str> = rest.split_whitespace().collect();
        let held = host.held_stack(uuid);
        command(host, uuid, &words, rest, held);
        Flow::Cancel
    }

    /// Writes an edit of the held Schematic to the stack.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || lock(&PENDING).is_empty() {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(data) = lock(&PENDING).get(&player.uuid).cloned() else {
            return;
        };
        let Some(mut stack) = host.carried_stack(event.slot).map(|s| s.clone()) else {
            return;
        };
        if data_of(&stack).is_none_or(|have| have.file != data.file) {
            return;
        }
        if host.set_component(&mut stack, COMPONENT, data.to_tag()).is_ok()
            && let Some(slot) = host.carried_stack(event.slot)
        {
            *slot = stack;
        }
        lock(&PENDING).remove(&player.uuid);
    }

    /// Tells the player's client what to draw when it changes: the held Schematic's hologram and
    /// the Schematic and Quill's box.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let message = view_message(&*host, player.uuid);
        let mut sent = lock(&SENT);
        if sent.get(&player.uuid).is_some_and(|last| *last == message) {
            return;
        }
        sent.insert(player.uuid, message.clone());
        drop(sent);
        let _ = host.net_send_to_player(player.uuid, CHANNEL, message.as_bytes());
    }
}

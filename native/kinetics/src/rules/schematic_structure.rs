//! A structure template, the format of a Schematic file (vanilla's `StructureTemplate.save` and
//! `load`, which Create's `SchematicExport`, `SchematicAndQuillItem` and `SchematicItem` use):
//! `size`, one `palette` of block states, the `blocks` that name a palette entry and a position,
//! the block entity data of some, and the `entities`. No game types: the glue reads the world and
//! the files and hands plain values over.
//!
//! Known differences from Create, for the README:
//! - A file with several palettes (`palettes`, which vanilla's shipwrecks use) reads as its first
//!   palette.
//! - Entities are read and kept but never saved: the Schematic and Quill saves blocks only (no
//!   item frames, armor stands or glue), and the Schematicannon does not print them.

use crate::rules::contraption::State;
use crate::rules::schematic_nbt::Nbt;

pub type Pos = [i32; 3];

/// The `DataVersion` written into a saved file: Horizon's current one (`DATA_VERSION`, which the
/// mod API doesn't export). A file with an older one has its block names upgraded by the game when
/// the glue reads it.
pub const DATA_VERSION: i32 = 5023;

/// `StructureTemplate.StructureBlockInfo`: a block of the structure.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub pos: Pos,
    /// An index into [`Template::palette`].
    pub state: usize,
    /// The block entity's data, with its `id` and without its position.
    pub nbt: Option<Nbt>,
}

/// `StructureTemplate.StructureEntityInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    pub pos: [f64; 3],
    pub block_pos: Pos,
    pub nbt: Nbt,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Template {
    pub size: Pos,
    pub palette: Vec<State>,
    pub blocks: Vec<Block>,
    pub entities: Vec<Entity>,
    pub data_version: i32,
}

fn pos_of(tag: Option<&Nbt>) -> Option<Pos> {
    let list = tag?.as_list()?;
    Some([list.first()?.as_int()?, list.get(1)?.as_int()?, list.get(2)?.as_int()?])
}

fn state_of(tag: &Nbt) -> Option<State> {
    let name = tag.get("Name")?.as_str()?;
    let mut state = State::new(name);
    if let Some(Nbt::Compound(props)) = tag.get("Properties") {
        for (key, value) in props {
            if let Some(value) = value.as_str() {
                state.set(key, value);
            }
        }
    }
    Some(state)
}

impl Template {
    /// `StructureTemplate.load`: reads the root tag. An error says what is wrong with the file.
    pub fn from_nbt(root: &Nbt) -> Result<Template, String> {
        let size = pos_of(root.get("size")).ok_or("the file has no size")?;
        let palette_tag = root
            .get("palette")
            .or_else(|| root.get("palettes").and_then(|p| p.as_list()?.first()))
            .and_then(Nbt::as_list)
            .ok_or("the file has no block palette")?;
        let palette: Vec<State> = palette_tag.iter().filter_map(state_of).collect();
        if palette.len() != palette_tag.len() {
            return Err("a palette entry has no name".to_owned());
        }
        let mut blocks = Vec::new();
        for tag in root.get("blocks").and_then(Nbt::as_list).unwrap_or(&[]) {
            let pos = pos_of(tag.get("pos")).ok_or("a block has no position")?;
            let state = tag.get("state").and_then(Nbt::as_int).ok_or("a block has no state")?;
            let state = usize::try_from(state).ok().filter(|s| *s < palette.len());
            let state = state.ok_or("a block names a state outside the palette")?;
            let nbt = tag.get("nbt").cloned();
            blocks.push(Block { pos, state, nbt });
        }
        let mut entities = Vec::new();
        for tag in root.get("entities").and_then(Nbt::as_list).unwrap_or(&[]) {
            let Some(list) = tag.get("pos").and_then(Nbt::as_list) else {
                continue;
            };
            let coords: Vec<f64> = list.iter().filter_map(Nbt::as_f64).collect();
            let (Some(nbt), [x, y, z]) = (tag.get("nbt"), coords[..].try_into().unwrap_or([0.0; 3])) else {
                continue;
            };
            let block_pos = pos_of(tag.get("blockPos")).unwrap_or([x.floor() as i32, y.floor() as i32, z.floor() as i32]);
            entities.push(Entity { pos: [x, y, z], block_pos, nbt: nbt.clone() });
        }
        Ok(Template {
            size,
            palette,
            blocks,
            entities,
            data_version: root.get("DataVersion").and_then(Nbt::as_int).unwrap_or(0),
        })
    }

    /// `StructureTemplate.save`.
    pub fn to_nbt(&self) -> Nbt {
        let palette = self
            .palette
            .iter()
            .map(|state| {
                let mut tag = Nbt::compound();
                tag.put("Name", Nbt::string(&state.name));
                if !state.props.is_empty() {
                    let mut props = Nbt::compound();
                    for (key, value) in &state.props {
                        props.put(key, Nbt::string(value));
                    }
                    tag.put("Properties", props);
                }
                tag
            })
            .collect();
        let blocks = self
            .blocks
            .iter()
            .map(|block| {
                let mut tag = Nbt::compound();
                tag.put("pos", Nbt::ints(&block.pos));
                tag.put("state", Nbt::Int(block.state as i32));
                if let Some(nbt) = &block.nbt {
                    tag.put("nbt", nbt.clone());
                }
                tag
            })
            .collect();
        let entities = self
            .entities
            .iter()
            .map(|entity| {
                let mut tag = Nbt::compound();
                tag.put("pos", Nbt::doubles(&entity.pos));
                tag.put("blockPos", Nbt::ints(&entity.block_pos));
                tag.put("nbt", entity.nbt.clone());
                tag
            })
            .collect();
        let mut root = Nbt::compound();
        root.put("size", Nbt::ints(&self.size));
        root.put("palette", Nbt::List(palette));
        root.put("blocks", Nbt::List(blocks));
        root.put("entities", Nbt::List(entities));
        root.put("DataVersion", Nbt::Int(if self.data_version == 0 { DATA_VERSION } else { self.data_version }));
        root
    }

    /// The state of a block.
    pub fn state(&self, block: &Block) -> &State {
        &self.palette[block.state]
    }
}

/// A block of the world, as the glue reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldBlock {
    pub state: State,
    /// The block entity's saved data with its `id`, without `x`, `y`, `z`.
    pub nbt: Option<Nbt>,
}

/// `StructureTemplate.fillFromWorld(level, origin, size, true, Blocks.AIR)` followed by
/// `SchematicAndQuillItem.replaceStructureVoidWithAir`: the blocks of a box, positions relative to
/// its lower corner. Air is left out (vanilla's `toIgnore`), a structure void is saved as air, which
/// is how a schematic clears a space; the others are kept in vanilla's order (by y, then x, then z)
/// with the palette in the order its states first appear. `read` gives the block at a position
/// relative to the corner, or `None` where it can't be read (it is left out).
pub fn fill_from_world(size: Pos, read: &mut dyn FnMut(Pos) -> Option<WorldBlock>) -> Template {
    let mut found: Vec<(Pos, WorldBlock)> = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            for z in 0..size[2] {
                let Some(mut block) = read([x, y, z]) else {
                    continue;
                };
                if block.state.name == "minecraft:air" {
                    continue;
                }
                if block.state.name == "minecraft:structure_void" {
                    block.state = State::new("minecraft:air");
                    block.nbt = None;
                }
                found.push(([x, y, z], block));
            }
        }
    }
    let mut template = Template { size, data_version: DATA_VERSION, ..Template::default() };
    for (pos, block) in found {
        let index = match template.palette.iter().position(|s| *s == block.state) {
            Some(i) => i,
            None => {
                template.palette.push(block.state.clone());
                template.palette.len() - 1
            }
        };
        template.blocks.push(Block { pos, state: index, nbt: block.nbt });
    }
    template
}

/// `FilesHelper.slug`: the name in lower case with every run of characters that aren't letters,
/// digits or `_` made one `_`.
pub fn slug(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_alphanumeric() || c == '_' {
            gap = false;
            out.push(c);
        } else if !gap {
            gap = true;
            out.push('_');
        }
    }
    out
}

/// `CreateLang` `schematicAndQuill.fallbackName`.
pub const FALLBACK_NAME: &str = "My Schematic";

/// `SchematicExport.saveSchematic`'s file name: an empty name becomes [`FALLBACK_NAME`], and unless
/// overwriting, `FilesHelper.findFirstValidFilename` gives `slug.nbt`, `slug_1.nbt`, `slug_2.nbt`...
/// the first that `taken` says is free; a name that already ends in `.nbt` is kept when overwriting.
pub fn file_name(name: &str, overwrite: bool, taken: &dyn Fn(&str) -> bool) -> String {
    let name = if name.is_empty() { FALLBACK_NAME } else { name };
    let mut file = name.to_owned();
    if !overwrite {
        let mut index = 0;
        loop {
            file = if index == 0 { format!("{}.nbt", slug(name)) } else { format!("{}_{index}.nbt", slug(name)) };
            index += 1;
            if !taken(&file) {
                break;
            }
        }
    }
    if !file.ends_with(".nbt") {
        file.push_str(".nbt");
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::schematic_nbt;

    fn stairs() -> State {
        State::new("minecraft:oak_stairs").with("facing", "east").with("half", "bottom")
    }

    #[test]
    fn a_box_of_the_world_becomes_a_template_and_back() {
        let mut chest = Nbt::compound();
        chest.put("id", Nbt::string("minecraft:chest"));
        let world = |pos: Pos| -> Option<WorldBlock> {
            Some(match pos {
                [0, 0, 0] => WorldBlock { state: State::new("minecraft:stone"), nbt: None },
                [1, 0, 0] => WorldBlock { state: stairs(), nbt: None },
                [0, 1, 0] => WorldBlock { state: State::new("minecraft:chest"), nbt: Some(chest.clone()) },
                [1, 1, 0] => WorldBlock { state: State::new("minecraft:structure_void"), nbt: None },
                [0, 0, 1] => WorldBlock { state: State::new("minecraft:stone"), nbt: None },
                _ => WorldBlock { state: State::new("minecraft:air"), nbt: None },
            })
        };
        let template = fill_from_world([2, 2, 2], &mut { world });
        // Air is left out, the void is air, by y then x then z.
        let order: Vec<Pos> = template.blocks.iter().map(|b| b.pos).collect();
        assert_eq!(order, [[0, 0, 0], [0, 0, 1], [1, 0, 0], [0, 1, 0], [1, 1, 0]]);
        let names: Vec<&str> = template.palette.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["minecraft:stone", "minecraft:oak_stairs", "minecraft:chest", "minecraft:air"]);
        assert_eq!(template.blocks[0].state, template.blocks[1].state);
        assert_eq!(template.blocks[3].nbt, Some(chest));
        assert_eq!(template.blocks[4].nbt, None);

        let bytes = schematic_nbt::write("", &template.to_nbt());
        let (_, root) = schematic_nbt::read(&bytes).unwrap();
        assert_eq!(Template::from_nbt(&root).unwrap(), template);
    }

    #[test]
    fn a_vanilla_file_reads() {
        let mut state = Nbt::compound();
        state.put("Name", Nbt::string("minecraft:lever"));
        let mut props = Nbt::compound();
        props.put("face", Nbt::string("wall"));
        props.put("facing", Nbt::string("north"));
        state.put("Properties", props);
        let mut block = Nbt::compound();
        block.put("pos", Nbt::ints(&[0, 0, 0]));
        block.put("state", Nbt::Int(0));
        let mut root = Nbt::compound();
        root.put("size", Nbt::ints(&[1, 1, 1]));
        root.put("palettes", Nbt::List(vec![Nbt::List(vec![state])]));
        root.put("blocks", Nbt::List(vec![block]));
        let template = Template::from_nbt(&root).unwrap();
        assert_eq!(template.palette[0].get("face"), Some("wall"));
        assert_eq!(template.blocks.len(), 1);
        assert!(template.entities.is_empty());
    }

    #[test]
    fn a_broken_file_says_what_is_wrong() {
        assert!(Template::from_nbt(&Nbt::compound()).is_err());
        let mut root = Nbt::compound();
        root.put("size", Nbt::ints(&[1, 1, 1]));
        root.put("palette", Nbt::List(Vec::new()));
        let mut block = Nbt::compound();
        block.put("pos", Nbt::ints(&[0, 0, 0]));
        block.put("state", Nbt::Int(3));
        root.put("blocks", Nbt::List(vec![block]));
        assert!(Template::from_nbt(&root).unwrap_err().contains("palette"));
    }

    #[test]
    fn file_names_are_create_s() {
        assert_eq!(slug("My House!"), "my_house_");
        assert_eq!(slug("a  b"), "a_b");
        let none = |_: &str| false;
        assert_eq!(file_name("", false, &none), "my_schematic.nbt");
        assert_eq!(file_name("Castle Keep", false, &none), "castle_keep.nbt");
        let taken = |f: &str| f == "castle_keep.nbt" || f == "castle_keep_1.nbt";
        assert_eq!(file_name("Castle Keep", false, &taken), "castle_keep_2.nbt");
        // `FilesHelper.slug` turns the dot of a typed extension into an underscore.
        assert_eq!(file_name("house.nbt", false, &none), "house_nbt.nbt");
        assert_eq!(file_name("house.nbt", true, &none), "house.nbt");
        assert_eq!(file_name("house", true, &none), "house.nbt");
    }
}

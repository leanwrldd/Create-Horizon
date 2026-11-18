//! The schematics folder and the structure files in it (`CreatePaths.SCHEMATICS_DIR`,
//! `SchematicExport`, `SchematicItem.loadSchematic`): listing, reading (gzip, then the NBT of
//! `create_kinetics::rules::schematic_nbt`) and writing them. The format is
//! `create_kinetics::rules::schematic_structure`; this is the glue to the disk and to Horizon's tags.
//!
//! Known differences from Create, for the README:
//! - **One folder.** Create keeps the player's own files in `schematics/` on the client and the
//!   copies a Schematic Table uploaded in `schematics/uploaded/<player>/` on the server. Here every
//!   file is in `<instance folder>/schematics/`, which the client and the integrated server share, and
//!   nothing is uploaded: a Schematic names a file and whoever reads it reads that folder (on a
//!   dedicated server, the server's own `schematics/` beside its `server.properties`).
//! - No size limits (`CSchematics.maxTotalSchematicSize`...) and no deletion of the oldest.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use create_kinetics::rules::contraption::State;
use create_kinetics::rules::schematic_nbt::{self, Nbt};
use create_kinetics::rules::schematic_structure::{self as structure, DATA_VERSION, Template};
use crate::compat::{Compound, Hooks, Host, Tag};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `CreatePaths.SCHEMATICS_DIR`.
pub fn folder(host: &dyn Host) -> PathBuf {
    host.instance_dir().join("schematics")
}

/// A file name that stays inside the folder: no separators, no `..`, a `.nbt` ending.
pub fn safe_name(file: &str) -> bool {
    !file.is_empty()
        && file.ends_with(".nbt")
        && !file.starts_with('.')
        && !file.contains(['/', '\\', ':', '\0'])
        && !file.contains("..")
}

/// The `.nbt` files of the folder, by name.
pub fn list(host: &dyn Host) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder(host))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".nbt"))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

// --- NBT, in Horizon's form and the rules' ---------------------------------------------------------

pub fn nbt_of(tag: &Tag) -> Option<Nbt> {
    Some(match tag {
        Tag::End => return None,
        Tag::Byte(v) => Nbt::Byte(*v),
        Tag::Short(v) => Nbt::Short(*v),
        Tag::Int(v) => Nbt::Int(*v),
        Tag::Long(v) => Nbt::Long(*v),
        Tag::Float(bits) => Nbt::Float(f32::from_bits(*bits)),
        Tag::Double(bits) => Nbt::Double(f64::from_bits(*bits)),
        Tag::ByteArray(bytes) => Nbt::ByteArray(bytes.iter().map(|b| *b as i8).collect()),
        Tag::String(text) => Nbt::String(text.to_string_lossy()),
        Tag::List(items) => Nbt::List(items.iter().filter_map(nbt_of).collect()),
        Tag::Compound(map) => Nbt::Compound(
            map.iter().filter_map(|(key, value)| Some((key.to_string_lossy(), nbt_of(value)?))).collect(),
        ),
        Tag::IntArray(values) => Nbt::IntArray(values.clone()),
        Tag::LongArray(values) => Nbt::LongArray(values.clone()),
    })
}

pub fn tag_of(nbt: &Nbt) -> Tag {
    match nbt {
        Nbt::Byte(v) => Tag::Byte(*v),
        Nbt::Short(v) => Tag::Short(*v),
        Nbt::Int(v) => Tag::Int(*v),
        Nbt::Long(v) => Tag::Long(*v),
        Nbt::Float(v) => Tag::Float(v.to_bits()),
        Nbt::Double(v) => Tag::Double(v.to_bits()),
        Nbt::ByteArray(bytes) => Tag::ByteArray(bytes.iter().map(|b| *b as u8).collect()),
        Nbt::String(text) => Tag::String(text.as_str().into()),
        Nbt::List(items) => Tag::List(items.iter().map(tag_of).collect()),
        Nbt::Compound(map) => {
            let mut out = Compound::new();
            for (key, value) in map {
                out.insert(key.as_str().into(), tag_of(value));
            }
            Tag::Compound(out)
        }
        Nbt::IntArray(values) => Tag::IntArray(values.clone()),
        Nbt::LongArray(values) => Tag::LongArray(values.clone()),
    }
}

// --- reading ----------------------------------------------------------------------------------------

/// Files read already, with the time they were last written: a hologram and a cannon ask for the
/// same file again and again.
static CACHE: Mutex<Option<HashMap<PathBuf, (SystemTime, u64, Arc<Template>)>>> = Mutex::new(None);

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // `NbtIo.readCompressed` wants gzip; a file that isn't compressed reads as it is.
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .take(512 * 1024 * 1024)
            .read_to_end(&mut out)
            .map_err(|e| format!("not a gzip file: {e}"))?;
        return Ok(out);
    }
    Ok(bytes.to_vec())
}

/// `SchematicItem.loadSchematic`: reads `file` from the folder. An error says what is wrong.
pub fn load(host: &dyn Host, file: &str) -> Result<Arc<Template>, String> {
    if !safe_name(file) {
        return Err(format!("{file} is not a schematic file name"));
    }
    load_path(host, &folder(host).join(file))
}

fn load_path(host: &dyn Host, path: &Path) -> Result<Arc<Template>, String> {
    let meta = std::fs::metadata(path).map_err(|_| "the file is not there".to_owned())?;
    let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
    if let Some(cache) = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()
        && let Some((time, size, template)) = cache.get(path)
        && (*time, *size) == stamp
    {
        return Ok(template.clone());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("can't read the file: {e}"))?;
    let (_, root) = schematic_nbt::read(&gunzip(&bytes)?)?;
    let mut template = Template::from_nbt(&root)?;
    upgrade(host, &mut template);
    let template = Arc::new(template);
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(path.to_owned(), (stamp.0, stamp.1, template.clone()));
    Ok(template)
}

/// A file written by an older game has its block names and properties upgraded as the datafixers
/// do (`StructureTemplate.load` runs `DataFixTypes.STRUCTURE`).
fn upgrade(host: &dyn Host, template: &mut Template) {
    if template.data_version <= 0 || template.data_version >= DATA_VERSION {
        return;
    }
    for state in &mut template.palette {
        let (name, props) = host.upgrade_block_state(&state.name, &state.props, template.data_version);
        *state = State { name, props };
    }
    template.data_version = DATA_VERSION;
}

// --- writing ----------------------------------------------------------------------------------------

/// `SchematicExport.saveSchematic`'s writing: a gzipped structure in the folder under the first
/// free name `structure::file_name` gives. The name it was saved as.
pub fn save(host: &dyn Host, name: &str, overwrite: bool, template: &Template) -> Result<String, String> {
    let dir = folder(host);
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make the schematics folder: {e}"))?;
    let file = structure::file_name(name, overwrite, &|candidate| dir.join(candidate).exists());
    if !safe_name(&file) {
        return Err(format!("{file} is not a usable file name"));
    }
    let raw = schematic_nbt::write("", &template.to_nbt());
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).map_err(|e| e.to_string())?;
    let bytes = gz.finish().map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&file), bytes).map_err(|e| format!("can't write {file}: {e}"))?;
    Ok(file)
}

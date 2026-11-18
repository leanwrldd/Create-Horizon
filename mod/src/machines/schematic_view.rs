//! What the client draws for the schematic items (`SchematicRenderer`, `SchematicHandler.render`,
//! `AABBOutline`, `SchematicAndQuillHandler.tick`): the hologram of a positioned Schematic in hand,
//! the box where an unpositioned one would go, and the Schematic and Quill's selection. The server
//! tells the client what the player holds (`schematic.rs`, channel `create:schematic`); the client
//! reads the file from its own `schematics/` folder.
//!
//! Known differences from Create, for the README:
//! - The hologram is **translucent boxes in each block's map colour** (`world_primitives` draws
//!   lines, boxes and text, not block models, and a mod can't draw a model in the world without a
//!   block entity), so stairs and slabs are shown as their bounding box and nothing is textured. A
//!   block hidden behind its neighbours is left out, the nearest 12,000 within 64 blocks are drawn,
//!   and the outline is Create's blue.
//! - The selection and the box a Schematic would take are plain blue outlines, with no checkered
//!   faces or face highlight.

use std::sync::{Arc, Mutex};

use create_kinetics::rules::contraption_facts::REPLACEABLE;
use create_kinetics::rules::schematic_printer::Layout;
use create_kinetics::rules::schematic_transform::{Mirror, Rotation, Transformation, transform_pos};
use crate::compat::{
    BlockStates, Hooks, Host, NetFromServerEvent, WorldPrimitivesEvent,
    serde_json::{self, Value},
};

use super::{schematic as items, schematic_files as files};

/// Create's `0x6886c5` outline.
const BLUE: u32 = 0xFF68_86C5;
/// Hologram boxes: the block's colour at about half strength.
const ALPHA: u32 = 0x78;
/// The most boxes drawn in a frame, and how far from the camera.
const MAX_BOXES: usize = 12_000;
const RANGE: f64 = 64.0;

struct View;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(View)
}

/// One block of the hologram, in world coordinates.
#[derive(Clone, Copy)]
struct Cell {
    min: [f64; 3],
    max: [f64; 3],
    colour: u32,
}

struct Hologram {
    file: String,
    cells: Vec<Cell>,
    outline: ([f64; 3], [f64; 3]),
}

#[derive(Clone, Debug, PartialEq)]
struct Spec {
    file: String,
    deployed: bool,
    anchor: [i32; 3],
    rotation: Rotation,
    mirror: Mirror,
    bounds: [i32; 3],
}

#[derive(Default)]
struct State {
    spec: Option<Spec>,
    hologram: Option<Arc<Hologram>>,
    selection: Option<([i32; 3], Option<[i32; 3]>)>,
}

static STATE: Mutex<State> = Mutex::new(State { spec: None, hologram: None, selection: None });

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn triple(value: Option<&Value>) -> Option<[i32; 3]> {
    let list = value?.as_array()?;
    Some([list.first()?.as_i64()? as i32, list.get(1)?.as_i64()? as i32, list.get(2)?.as_i64()? as i32])
}

fn spec_of(value: &Value) -> Option<Spec> {
    Some(Spec {
        file: value.get("file")?.as_str()?.to_owned(),
        deployed: value.get("deployed")?.as_bool()?,
        anchor: triple(value.get("anchor"))?,
        rotation: Rotation::from_name(value.get("rotation")?.as_str()?)?,
        mirror: Mirror::from_name(value.get("mirror")?.as_str()?)?,
        bounds: triple(value.get("bounds"))?,
    })
}

/// What a block looks like to the hologram.
#[derive(Clone)]
struct Look {
    colour: u32,
    /// Block-local box of what it fills (`None`: invisible).
    shape: Option<[f64; 6]>,
    full: bool,
}

/// What one state of a block draws as: its colour and the box it fills.
fn look_of(facts: &crate::compat::BlockStateFacts) -> Look {
    if facts.air || facts.invisible {
        return Look { colour: 0, shape: None, full: false };
    }
    let colour = facts.map_color.unwrap_or(0x0090_9090);
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for b in facts.outline.iter() {
        for i in 0..3 {
            lo[i] = lo[i].min(b[i]);
            hi[i] = hi[i].max(b[i + 3]);
        }
    }
    if facts.outline.is_empty() {
        // Plants and the like have no outline worth drawing: a small box in the middle.
        return Look { colour, shape: Some([0.3, 0.0, 0.3, 0.7, 0.6, 0.7]), full: false };
    }
    let full = facts.outline.len() == 1 && lo == [0.0; 3] && hi == [1.0; 3];
    Look { colour, shape: Some([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]), full }
}

/// What each block looks like, by name, from the game's own table of states (the first state of
/// a block stands for it): built once, as there are tens of thousands of states.
static LOOKS: Mutex<Option<std::collections::HashMap<String, Look>>> = Mutex::new(None);

fn looks(host: &dyn Host) -> std::collections::HashMap<String, Look> {
    if let Some(known) = lock(&LOOKS).as_ref() {
        return known.clone();
    }
    let mut table = std::collections::HashMap::new();
    let states: BlockStates = host.block_states();
    for (_, facts) in states.iter() {
        table.entry(facts.name.to_string()).or_insert_with(|| look_of(facts));
    }
    *lock(&LOOKS) = Some(table.clone());
    table
}

fn build(host: &dyn Host, spec: &Spec) -> Option<Hologram> {
    let template = files::load(host, &spec.file).ok()?;
    let layout = Layout::new(&template, spec.rotation, spec.mirror);
    let looks = looks(host);
    // A block the client doesn't know: a grey cube.
    let grey = Look { colour: 0x0090_9090, shape: Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]), full: false };
    let mut full = std::collections::HashSet::new();
    let positions: Vec<[i32; 3]> = layout.positions().collect();
    for rel in &positions {
        if looks.get(&layout.state(*rel).name).is_some_and(|look| look.full) {
            full.insert(*rel);
        }
    }
    let mut cells = Vec::new();
    for rel in positions {
        let name = layout.state(rel).name;
        let look = looks.get(&name).unwrap_or(&grey);
        let Some(shape) = look.shape else {
            continue;
        };
        // Hidden inside the schematic: every neighbour is a full block.
        if look.full {
            let hidden = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]]
                .iter()
                .all(|d| full.contains(&[rel[0] + d[0], rel[1] + d[1], rel[2] + d[2]]));
            if hidden {
                continue;
            }
        }
        let at = [
            f64::from(spec.anchor[0] + rel[0]),
            f64::from(spec.anchor[1] + rel[1]),
            f64::from(spec.anchor[2] + rel[2]),
        ];
        // A hair inside the cell so it doesn't fight the block that may stand there.
        const INSET: f64 = 0.01;
        cells.push(Cell {
            min: [at[0] + shape[0] + INSET, at[1] + shape[1] + INSET, at[2] + shape[2] + INSET],
            max: [at[0] + shape[3] - INSET, at[1] + shape[4] - INSET, at[2] + shape[5] - INSET],
            colour: (ALPHA << 24) | (look.colour & 0x00FF_FFFF),
        });
    }
    let b = layout.bounds;
    let outline = (
        [
            f64::from(spec.anchor[0] + b.min[0]),
            f64::from(spec.anchor[1] + b.min[1]),
            f64::from(spec.anchor[2] + b.min[2]),
        ],
        [
            f64::from(spec.anchor[0] + b.max[0] + 1),
            f64::from(spec.anchor[1] + b.max[1] + 1),
            f64::from(spec.anchor[2] + b.max[2] + 1),
        ],
    );
    Some(Hologram { file: spec.file.clone(), cells, outline })
}

/// The block under the cursor and the face it is hit on, from the hit point.
fn hit(host: &dyn Host) -> Option<([i32; 3], [i32; 3])> {
    let (_, hit) = host.view_hit()?;
    let pos = [hit.block_x, hit.block_y, hit.block_z];
    let local = [hit.x - f64::from(pos[0]), hit.y - f64::from(pos[1]), hit.z - f64::from(pos[2])];
    let mut best = (f64::MAX, [0, 1, 0]);
    for axis in 0..3 {
        for (edge, sign) in [(0.0, -1), (1.0, 1)] {
            let distance = (local[axis] - edge).abs();
            if distance < best.0 {
                let mut face = [0; 3];
                face[axis] = sign;
                best = (distance, face);
            }
        }
    }
    Some((pos, best.1))
}

/// Where the cursor would put an unpositioned Schematic: the box it would fill.
fn preview(host: &dyn Host, spec: &Spec) -> Option<([f64; 3], [f64; 3])> {
    let (pos, face) = hit(host)?;
    let replaceable = REPLACEABLE.binary_search(&host.block(pos).as_str()).is_ok();
    let selected = if replaceable { pos } else { [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]] };
    let mut t = Transformation::new([0; 3], spec.rotation, spec.mirror, spec.bounds);
    t.target = Transformation::deploy_target(selected, spec.bounds);
    let anchor = t.anchor();
    let far = [spec.bounds[0] - 1, spec.bounds[1] - 1, spec.bounds[2] - 1];
    let a = transform_pos([0, 0, 0], spec.mirror, spec.rotation);
    let b = transform_pos(far, spec.mirror, spec.rotation);
    let min = [a[0].min(b[0]), 0, a[2].min(b[2])];
    let max = [a[0].max(b[0]), far[1], a[2].max(b[2])];
    Some((
        [f64::from(anchor[0] + min[0]), f64::from(anchor[1]), f64::from(anchor[2] + min[2])],
        [f64::from(anchor[0] + max[0] + 1), f64::from(anchor[1] + max[1] + 1), f64::from(anchor[2] + max[2] + 1)],
    ))
}

impl Hooks for View {
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != items::CHANNEL {
            return;
        }
        let Ok(message) = serde_json::from_slice::<Value>(event.data) else {
            return;
        };
        let spec = message.get("holo").filter(|v| !v.is_null()).and_then(spec_of);
        let selection = message
            .get("sel")
            .filter(|v| !v.is_null())
            .and_then(|v| Some((triple(v.get("a"))?, triple(v.get("b")))));
        let unchanged = lock(&STATE).spec == spec;
        let hologram = if unchanged {
            lock(&STATE).hologram.clone()
        } else {
            // Only a positioned Schematic has a hologram to build.
            spec.as_ref().filter(|s| s.deployed).and_then(|s| build(&*host, s)).map(Arc::new)
        };
        let mut state = lock(&STATE);
        state.spec = spec;
        state.hologram = hologram;
        state.selection = selection;
    }

    fn world_primitives(&self, host: &dyn Host, event: &mut WorldPrimitivesEvent<'_>) {
        let state = lock(&STATE);
        let camera = event.camera;
        if let Some((a, b)) = state.selection {
            let (low, high) = match b {
                Some(b) => {
                    let (lo, hi) = super::schematic_world::corners(a, b);
                    (lo, hi)
                }
                None => (a, a),
            };
            event.primitives.box_outline(
                [f64::from(low[0]), f64::from(low[1]), f64::from(low[2])],
                [f64::from(high[0] + 1), f64::from(high[1] + 1), f64::from(high[2] + 1)],
                BLUE,
                2.0,
            );
            // The block the next click would take (`SchematicAndQuillHandler.tick`).
            if let Some((pos, face)) = hit(host) {
                let at = items::picked(host, pos, face);
                event.primitives.box_outline(
                    [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])],
                    [f64::from(at[0] + 1), f64::from(at[1] + 1), f64::from(at[2] + 1)],
                    0x66FF_FFFF,
                    1.0,
                );
            }
        }
        let Some(spec) = state.spec.clone() else {
            return;
        };
        if !spec.deployed {
            if let Some((min, max)) = preview(host, &spec) {
                event.primitives.box_outline(min, max, BLUE, 2.0);
            }
            return;
        }
        let Some(hologram) = state.hologram.clone() else {
            return;
        };
        drop(state);
        event.primitives.box_outline(hologram.outline.0, hologram.outline.1, BLUE, 2.0);
        let mut drawn = 0;
        let limit = RANGE * RANGE;
        for cell in &hologram.cells {
            let centre = [
                (cell.min[0] + cell.max[0]) / 2.0 - camera[0],
                (cell.min[1] + cell.max[1]) / 2.0 - camera[1],
                (cell.min[2] + cell.max[2]) / 2.0 - camera[2],
            ];
            if centre[0] * centre[0] + centre[1] * centre[1] + centre[2] * centre[2] > limit {
                continue;
            }
            event.primitives.filled_box(cell.min, cell.max, cell.colour);
            drawn += 1;
            if drawn >= MAX_BOXES {
                break;
            }
        }
        let _ = hologram.file.len();
    }
}

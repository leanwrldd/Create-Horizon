//! The collision shape of each state a structure carries (`BlockState.getCollisionShape`).
//!
//! Only the server can ask the game for a shape of a state that is not in the world
//! (`LevelRef::block_shape`); a client has `block_collision(pos)` for blocks that are. So the server
//! asks once for each distinct state and keeps the answer by the state's text; the controller's
//! update tag carries it in the structure's palette (`Shape`: six numbers a box, in blocks relative
//! to the block), which is how a client collides its player with the same boxes.
//!
//! A state the game can't answer for is a full cube when the old name-based guess says it is solid
//! (`contraption::solid`), nothing otherwise.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use create_kinetics::Pos;
use create_kinetics::rules::collider_obb::Aabb;
use create_kinetics::rules::contraption::State;
use mod_api::{BlockShapeKind, Host, LevelRef, Tag};

static TABLE: LazyLock<Mutex<HashMap<String, Arc<[Aabb]>>>> = LazyLock::new(Mutex::default);
/// The selection outline of each state (`BlockState.getShape`), by the state's text: what a click's
/// ray meets (`ContraptionHandlerClient.rayTraceContraption`). Server only, never sent.
static OUTLINES: LazyLock<Mutex<HashMap<String, Arc<[Aabb]>>>> = LazyLock::new(Mutex::default);
static VERSION: AtomicU64 = AtomicU64::new(1);

fn table() -> MutexGuard<'static, HashMap<String, Arc<[Aabb]>>> {
    TABLE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `name[key=value,...]`, as `/setblock` reads a state.
pub fn text(state: &State) -> String {
    if state.props.is_empty() {
        return state.name.clone();
    }
    let props: Vec<String> = state.props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", state.name, props.join(","))
}

/// Changes whenever a shape is added: a structure built before it is built again.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// The shape the game gave for the state, if it has been asked.
pub fn lookup(state: &State) -> Option<Arc<[Aabb]>> {
    table().get(&text(state)).cloned()
}

/// A full cube, or nothing.
fn guess(state: &State) -> Arc<[Aabb]> {
    if crate::machines::contraption::solid(state) {
        Arc::from(vec![Aabb::new([0.0; 3], [1.0; 3])])
    } else {
        Arc::from(Vec::new())
    }
}

/// The boxes of a state: the game's, or the guess.
pub fn boxes_of(state: &State) -> Arc<[Aabb]> {
    lookup(state).unwrap_or_else(|| guess(state))
}

/// Asks the game for the shape of each state not known yet, at `pos` (shapes that read the level
/// read it there). Whether anything was added.
pub fn fill<'a>(host: &dyn Host, level: LevelRef, pos: Pos, states: impl Iterator<Item = &'a State>) -> bool {
    let mut added = false;
    for state in states {
        let key = text(state);
        if table().contains_key(&key) {
            continue;
        }
        let boxes: Arc<[Aabb]> = match host
            .block_state_id(&key)
            .and_then(|id| level.block_shape(host, id, pos, BlockShapeKind::Collision))
        {
            Some(list) => Arc::from(
                list.iter()
                    .map(|b| Aabb::new([b[0], b[1], b[2]], [b[3], b[4], b[5]]))
                    .filter(|b| (0..3).all(|i| b.max[i] > b.min[i]))
                    .collect::<Vec<_>>(),
            ),
            None => guess(state),
        };
        table().insert(key, boxes);
        added = true;
    }
    if added {
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
    added
}

/// The outline boxes of a state, asked of the game once (`BlockShapeKind::Outline`); a state it can't
/// answer for has its collision boxes, or a full cube when it has none and is not air.
pub fn outline_of(host: &dyn Host, level: LevelRef, pos: Pos, state: &State) -> Arc<[Aabb]> {
    let key = text(state);
    if let Some(known) = OUTLINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return known.clone();
    }
    let asked = host
        .block_state_id(&key)
        .and_then(|id| level.block_shape(host, id, pos, BlockShapeKind::Outline))
        .map(|list| {
            list.iter()
                .map(|b| Aabb::new([b[0], b[1], b[2]], [b[3], b[4], b[5]]))
                .filter(|b| (0..3).all(|i| b.max[i] > b.min[i]))
                .collect::<Vec<_>>()
        });
    let boxes: Arc<[Aabb]> = match asked {
        Some(list) => Arc::from(list),
        None => {
            let collision = boxes_of(state);
            if collision.is_empty() && !state.is_air() {
                Arc::from(vec![Aabb::new([0.0; 3], [1.0; 3])])
            } else {
                collision
            }
        }
    };
    OUTLINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, boxes.clone());
    boxes
}

/// The outline the game gave for the state, if it has been asked.
pub fn outline_lookup(state: &State) -> Option<Arc<[Aabb]>> {
    OUTLINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&text(state)).cloned()
}

/// Asks the game for the outline of each state not known yet (the update tag carries them for the
/// client's click ray). Whether any was new.
pub fn fill_outlines<'a>(host: &dyn Host, level: LevelRef, pos: Pos, states: impl Iterator<Item = &'a State>) -> bool {
    let mut added = false;
    for state in states {
        if outline_lookup(state).is_none() {
            outline_of(host, level, pos, state);
            added = true;
        }
    }
    added
}

/// A shape as the palette carries it: six doubles a box.
pub fn to_tag(boxes: &[Aabb]) -> Tag {
    let mut out = Vec::with_capacity(boxes.len() * 6);
    for b in boxes {
        for c in b.min.into_iter().chain(b.max) {
            out.push(Tag::Double(c.to_bits()));
        }
    }
    Tag::List(out)
}

/// The boxes of a palette entry's `Shape`.
pub fn from_tag(tag: &Tag) -> Option<Vec<Aabb>> {
    let Tag::List(list) = tag else { return None };
    let numbers: Vec<f64> = list
        .iter()
        .map(|t| match t {
            Tag::Double(bits) => Some(f64::from_bits(*bits)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if numbers.len() % 6 != 0 {
        return None;
    }
    Some(
        numbers
            .chunks_exact(6)
            .map(|c| Aabb::new([c[0], c[1], c[2]], [c[3], c[4], c[5]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_reads_as_setblock_does() {
        assert_eq!(text(&State::new("minecraft:stone")), "minecraft:stone");
        let s = State::new("minecraft:oak_stairs").with("facing", "east").with("half", "bottom");
        assert_eq!(text(&s), "minecraft:oak_stairs[facing=east,half=bottom]");
    }

    #[test]
    fn a_shape_goes_through_a_tag_and_back() {
        let boxes = vec![Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 1.0]), Aabb::new([0.25, 0.5, 0.25], [0.75, 1.0, 0.75])];
        assert_eq!(from_tag(&to_tag(&boxes)), Some(boxes));
        assert_eq!(from_tag(&Tag::List(vec![Tag::Double(1.0f64.to_bits())])), None);
        assert_eq!(from_tag(&Tag::Int(3)), None);
        assert_eq!(from_tag(&Tag::List(Vec::new())), Some(Vec::new()));
    }
}

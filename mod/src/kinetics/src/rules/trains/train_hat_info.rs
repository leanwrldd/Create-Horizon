//! Where the Train Hat sits on a mob whose head Create reads off its model: `TrainHatInfo`, `TrainHatInfoReloadListener` and the
//! data of `VanillaHatOffsetGenerator` (Create writes it as `train_hat_info/<entity>.json`; here it is the table [`INFO`]), and
//! `CreateHatArmorLayer.render`'s walk down the model's parts to the head.
//!
//! Create's layer takes the entity model's parts from the renderer. A mod can't, so the glue bakes each type's vanilla layer
//! (`ClientHost::bake_layer`, `minecraft:cow#main`) once at client start, turns it into a [`ModelNode`] tree (a part's pose, the
//! boxes of its faces, its children) and [`resolve`] finds the head in it: the chain of parts from the model's root to the head,
//! the head's first box (`TrainHatInfo.cubeIndex`) and the offset and scale of the info. [`mount_ops`] then gives the draw
//! operations that lay the hat on it, the chain `CreateHatArmorLayer` makes (`translateAndRotate` of each part, the offset and
//! scale from the box, the flip, `translate(0, -2.25 / 16, 0)`, `rotateX(-8.5)`).
//!
//! What differs from Create, so that the table of a 1.21.1 model fits a 26.3 one:
//! - The model of 26.3 has the renderer's own scale and lift in its root's pose (a cat's root is `0.8` scaled and `4.8` down, a
//!   horse's `1.1`), where Create's chain starts at the head part of an `AgeableListModel` and scales in the renderer. The chain
//!   here always starts at the root, so both end up with the scale in force.
//! - `getAdjustedPart` skips a name that isn't a child. Here the path must exist: from the root, else under the head, else below
//!   any part of that name (the bee's `body` is under `bone`, the axolotl's `head` under `body`, a wolf's `real_head` under
//!   `head`); a type whose path is nowhere gets no hat. Equines have their head in `head_parts`, which is what an
//!   `AgeableListModel`'s head part was for them.
//! - A part's boxes are read off its faces' corners (six faces to a box), so a box that grows (`CubeDeformation`) reads larger
//!   than Create's `Cube.minX..maxX`, which doesn't include the growth.
//! - Only the part named `head` (the last of that name) is turned with the entity's head angles (`setupAnim`); a humanoid's
//!   head (piglins) takes Horizon's own pose of it.
//! - `TrainHatInfoReloadListener` also reads `train_hat_info` files of resource packs and other mods; those aren't read here.
//!   `getHatInfoFor`'s special case, a snow golem that wears a pumpkin ([`SNOW_GOLEM_PUMPKIN`]), needs the entity's data, which
//!   a layer doesn't have: [`info_for`] answers the plain snow golem.

use super::train_hat::{HeadPose, MODEL_ORIGIN, Op, rotation_ops};
use super::vec::Vec3;

/// `TrainHatInfo(part, cubeIndex, offset, scale)`: the part (a `/` path under the model's root, empty for the head), which box of
/// it, the hat's offset in texels and its scale on top of the box's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatInfo {
    pub part: &'static str,
    pub cube_index: usize,
    pub offset: Vec3,
    pub scale: f32,
}

/// `TrainHatInfoReloadListener.DEFAULT`.
pub const DEFAULT: HatInfo = HatInfo { part: "", cube_index: 0, offset: [0.0; 3], scale: 1.0 };

/// `getHatInfoFor`'s override for a snow golem with a pumpkin.
pub const SNOW_GOLEM_PUMPKIN: HatInfo = HatInfo { part: "", cube_index: 0, offset: [0.0, -3.0, 0.0], scale: 1.18 };

/// `VanillaHatOffsetGenerator.createOffsets`, in its order.
pub const INFO: [(&str, HatInfo); 58] = [
    ("minecraft:armadillo", HatInfo { part: "body/head/head_cube", cube_index: 0, offset: [0.0, 4.0, 0.0], scale: 0.92 }),
    ("minecraft:axolotl", HatInfo { part: "head", cube_index: 0, offset: [0.0, 1.0, -2.0], scale: 0.75 }),
    ("minecraft:bat", HatInfo { part: "head", cube_index: 0, offset: [0.0, -0.2, 0.0], scale: 0.8 }),
    ("minecraft:bee", HatInfo { part: "body", cube_index: 0, offset: [0.0, 2.0, -2.0], scale: 0.5 }),
    ("minecraft:blaze", HatInfo { part: "", cube_index: 0, offset: [0.0, 4.0, 0.0], scale: 1.0 }),
    ("minecraft:breeze", HatInfo { part: "body/head", cube_index: 0, offset: [0.0, -5.0, -0.1], scale: 0.8 }),
    ("minecraft:camel", HatInfo { part: "body/head", cube_index: 1, offset: [0.0, -8.0, -11.5], scale: 1.0 }),
    ("minecraft:cat", HatInfo { part: "", cube_index: 0, offset: [0.0, 1.0, -0.25], scale: 1.0 }),
    ("minecraft:cave_spider", HatInfo { part: "", cube_index: 0, offset: [0.0, 2.0, -3.5], scale: 1.0 }),
    ("minecraft:chicken", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, -0.25], scale: 1.0 }),
    ("minecraft:cod", HatInfo { part: "", cube_index: 0, offset: [0.0, 10.0, 0.0], scale: 1.0 }),
    ("minecraft:cow", HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, -3.0], scale: 0.87 }),
    ("minecraft:dolphin", HatInfo { part: "body/head", cube_index: 0, offset: [0.0, 3.0, 0.0], scale: 0.75 }),
    ("minecraft:donkey", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.8, 2.0], scale: 1.0 }),
    ("minecraft:elder_guardian", HatInfo { part: "", cube_index: 0, offset: [0.0, 20.0, 0.0], scale: 0.95 }),
    ("minecraft:endermite", HatInfo { part: "segment0", cube_index: 0, offset: [0.0, 2.5, 0.5], scale: 0.75 }),
    ("minecraft:fox", HatInfo { part: "", cube_index: 0, offset: [1.0, 2.5, -2.0], scale: 0.9 }),
    ("minecraft:frog", HatInfo { part: "body/head", cube_index: 0, offset: [0.0, -4.0, -2.5], scale: 0.75 }),
    ("minecraft:ghast", HatInfo { part: "body", cube_index: 0, offset: [0.0, 6.0, 0.0], scale: 0.92 }),
    ("minecraft:glow_squid", HatInfo { part: "body", cube_index: 0, offset: [0.0, 7.0, 0.0], scale: 0.92 }),
    ("minecraft:goat", HatInfo { part: "head", cube_index: 0, offset: [-0.5, -8.5, -9.0], scale: 2.0 }),
    ("minecraft:guardian", HatInfo { part: "", cube_index: 0, offset: [0.0, 20.0, 0.0], scale: 0.9 }),
    ("minecraft:hoglin", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, -4.5], scale: 0.5 }),
    ("minecraft:horse", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.8, 2.0], scale: 1.0 }),
    ("minecraft:iron_golem", HatInfo { part: "", cube_index: 0, offset: [0.0, -2.0, -1.5], scale: 1.0 }),
    ("minecraft:magma_cube", HatInfo { part: "cube7", cube_index: 0, offset: [0.0, 16.0, 0.0], scale: 1.0 }),
    ("minecraft:mooshroom", HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, -3.0], scale: 0.87 }),
    ("minecraft:mule", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.8, 2.0], scale: 1.0 }),
    ("minecraft:ocelot", HatInfo { part: "", cube_index: 0, offset: [0.0, 1.0, -0.25], scale: 1.0 }),
    ("minecraft:panda", HatInfo { part: "", cube_index: 0, offset: [0.0, 4.0, 0.5], scale: 0.75 }),
    ("minecraft:parrot", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, -1.5], scale: 1.0 }),
    ("minecraft:phantom", HatInfo { part: "body/head", cube_index: 0, offset: [0.0, 0.0, -1.0], scale: 1.0 }),
    ("minecraft:pig", HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, -4.0], scale: 1.0 }),
    ("minecraft:piglin", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, 0.0], scale: 0.92 }),
    ("minecraft:piglin_brute", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, 0.0], scale: 0.92 }),
    ("minecraft:polar_bear", HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, 0.0], scale: 1.0 }),
    ("minecraft:pufferfish", HatInfo { part: "body", cube_index: 0, offset: [0.0, -0.5, 0.0], scale: 0.75 }),
    ("minecraft:ravager", HatInfo { part: "neck/head", cube_index: 0, offset: [0.0, 0.0, -5.5], scale: 1.0 }),
    ("minecraft:salmon", HatInfo { part: "", cube_index: 0, offset: [0.0, 1.0, 0.0], scale: 1.0 }),
    ("minecraft:sheep", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.4, -1.0], scale: 0.87 }),
    ("minecraft:silverfish", HatInfo { part: "segment1", cube_index: 0, offset: [0.0, 3.0, 0.0], scale: 1.0 }),
    ("minecraft:skeleton_horse", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.8, 2.0], scale: 1.0 }),
    ("minecraft:slime", HatInfo { part: "cube", cube_index: 0, offset: [0.0, 21.0, 0.0], scale: 1.25 }),
    ("minecraft:sniffer", HatInfo { part: "bone/body/head", cube_index: 0, offset: [0.0, 8.0, -5.0], scale: 1.0 }),
    ("minecraft:snow_golem", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.2, 0.0], scale: 0.82 }),
    ("minecraft:spider", HatInfo { part: "", cube_index: 0, offset: [0.0, 2.0, -3.5], scale: 1.0 }),
    ("minecraft:squid", HatInfo { part: "body", cube_index: 0, offset: [0.0, 7.0, 0.0], scale: 0.92 }),
    ("minecraft:strider", HatInfo { part: "body", cube_index: 0, offset: [0.0, 8.0, 0.0], scale: 0.95 }),
    ("minecraft:tadpole", HatInfo { part: "body", cube_index: 0, offset: [0.0, 1.0, 1.5], scale: 1.0 }),
    ("minecraft:tropical_fish", HatInfo { part: "body", cube_index: 0, offset: [0.0, 1.0, -2.0], scale: 0.5 }),
    ("minecraft:turtle", HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, 0.0], scale: 1.0 }),
    ("minecraft:warden", HatInfo { part: "bone/body/head", cube_index: 0, offset: [0.0, 0.0, 0.5], scale: 0.9 }),
    ("minecraft:witch", HatInfo { part: "", cube_index: 0, offset: [0.0, -1.8, 0.0], scale: 1.0 }),
    ("minecraft:wither", HatInfo { part: "center_head", cube_index: 0, offset: [0.0, 3.0, 0.0], scale: 1.0 }),
    ("minecraft:wolf", HatInfo { part: "real_head", cube_index: 0, offset: [1.0, 2.5, 1.0], scale: 1.0 }),
    ("minecraft:zoglin", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, -4.5], scale: 0.5 }),
    ("minecraft:zombie_horse", HatInfo { part: "", cube_index: 0, offset: [0.0, -0.8, 2.0], scale: 1.0 }),
    ("minecraft:zombified_piglin", HatInfo { part: "", cube_index: 0, offset: [0.0, 0.0, 0.0], scale: 0.92 }),
];

/// `getHatInfoFor` without the pumpkin: the type's info, else the default.
pub fn info_for(entity_type: &str) -> HatInfo {
    INFO.iter().find(|(name, _)| *name == entity_type).map_or(DEFAULT, |(_, info)| *info)
}

/// Whether Create has data for the type.
pub fn has_info(entity_type: &str) -> bool {
    INFO.iter().any(|(name, _)| *name == entity_type)
}

/// The entity types the table lists.
pub fn types() -> impl Iterator<Item = &'static str> {
    INFO.iter().map(|(name, _)| *name)
}

/// The types with a humanoid head (a `HumanoidModel`): Horizon poses it, as it does for zombies.
pub fn is_humanoid(entity_type: &str) -> bool {
    matches!(entity_type, "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:zombified_piglin")
}

/// The model's name of the horse family's head part (`AbstractEquineModel.headParts`).
fn default_part(entity_type: &str) -> &'static str {
    match entity_type {
        "minecraft:horse" | "minecraft:donkey" | "minecraft:mule" | "minecraft:skeleton_horse" | "minecraft:zombie_horse" => "head_parts",
        _ => "head",
    }
}

// --- the model, as far as the hat needs it ---------------------------------------------------------------------------

/// A part of a baked model: its name, its pose (`x, y, z` in texels, `x, y, z` turn in radians, `x, y, z` scale), the boxes of its
/// own faces (`min x, y, z, max x, y, z`, texels) and its children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelNode {
    pub name: String,
    pub pose: [f32; 9],
    pub cubes: Vec<[f32; 6]>,
    pub children: Vec<ModelNode>,
}

/// The boxes a part's faces make: the faces come six to a box (`ModelPart.Cube.polygons`), each as its four corners. A part whose
/// face count isn't a multiple of six (a box with faces left out) is read as one box.
pub fn cubes_of(faces: &[[[f32; 3]; 4]]) -> Vec<[f32; 6]> {
    if faces.is_empty() {
        return Vec::new();
    }
    let group = if faces.len() % 6 == 0 { 6 } else { faces.len() };
    faces
        .chunks(group)
        .map(|chunk| {
            let mut bounds = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
            for corner in chunk.iter().flatten() {
                for axis in 0..3 {
                    bounds[axis] = bounds[axis].min(corner[axis]);
                    bounds[axis + 3] = bounds[axis + 3].max(corner[axis]);
                }
            }
            bounds
        })
        .collect()
}

/// How a part of the chain is posed this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// As baked.
    Rest,
    /// Turned with the entity's head angles (`xRot` the pitch, `yRot` the head's turn from the body).
    Head,
    /// Posed by Horizon (a humanoid model's head).
    Posed,
}

/// One part of the chain from the model's root to the head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub pose: [f32; 9],
    pub turn: Turn,
}

/// What the layer keeps of a type's model: the chain to the head, the head's box and the info's offset and scale.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMount {
    pub steps: Vec<Step>,
    /// `lastChild.cubes.get(clamp(cubeIndex, 0, size - 1))`; none when the last part has no boxes (`isEmpty`).
    pub cube: Option<[f32; 6]>,
    pub offset: Vec3,
    pub scale: f32,
}

/// The nodes after `from` that follow `path`, each a child of the one before.
fn descend<'a>(from: &'a ModelNode, path: &[&str]) -> Option<Vec<&'a ModelNode>> {
    let mut chain = Vec::with_capacity(path.len());
    let mut at = from;
    for name in path {
        at = at.children.iter().find(|c| c.name == *name)?;
        chain.push(at);
    }
    Some(chain)
}

/// The nodes from the root's child to the end of `path`: from the root, else below the first part (nearest the root) of the
/// path's first name that has the rest under it.
fn find_path<'a>(root: &'a ModelNode, path: &[&str]) -> Option<Vec<&'a ModelNode>> {
    if let Some(chain) = descend(root, path) {
        return Some(chain);
    }
    let first = path.first()?;
    let mut queue: std::collections::VecDeque<(&ModelNode, Vec<&ModelNode>)> = root.children.iter().map(|c| (c, vec![c])).collect();
    while let Some((node, chain)) = queue.pop_front() {
        if node.name == *first
            && let Some(rest) = descend(node, &path[1..])
        {
            let mut all = chain;
            all.extend(rest);
            return Some(all);
        }
        for child in &node.children {
            let mut next = chain.clone();
            next.push(child);
            queue.push_back((child, next));
        }
    }
    None
}

/// `CreateHatArmorLayer.render`'s `partsToHead` for a type whose model is `root`: the chain from the root to the head part the
/// info names (the head by default), and the head's box. None when the model has no such part.
pub fn resolve(root: &ModelNode, entity_type: &str, info: &HatInfo) -> Option<ModelMount> {
    let default = default_part(entity_type);
    let path: Vec<&str> = if info.part.is_empty() { vec![default] } else { info.part.split('/').collect() };
    let found = find_path(root, &path)?;
    let mut nodes: Vec<&ModelNode> = vec![root];
    nodes.extend(found);
    let humanoid = is_humanoid(entity_type);
    let head = nodes.iter().rposition(|n| n.name == "head");
    let steps = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| Step {
            pose: n.pose,
            turn: if Some(i) != head {
                Turn::Rest
            } else if humanoid {
                Turn::Posed
            } else {
                Turn::Head
            },
        })
        .collect();
    let last = nodes.last()?;
    let cube = if last.cubes.is_empty() { None } else { Some(last.cubes[info.cube_index.min(last.cubes.len() - 1)]) };
    Some(ModelMount { steps, cube, offset: info.offset, scale: info.scale })
}

/// `ModelPart.translateAndRotate`: translate, then `rotationZYX(zRot, yRot, xRot)` (a turn about x first, then y, then z, in the
/// frame the translation made), then the scale when it isn't 1.
pub fn part_ops(pose: &[f32; 9]) -> Vec<Op> {
    let mut ops = vec![Op::Translate([f64::from(pose[0]) / 16.0, f64::from(pose[1]) / 16.0, f64::from(pose[2]) / 16.0])];
    ops.extend(rotation_ops([f64::from(pose[3]), f64::from(pose[4]), f64::from(pose[5])]));
    if pose[6..9].iter().any(|s| *s != 1.0) {
        ops.push(Op::Scale([f64::from(pose[6]), f64::from(pose[7]), f64::from(pose[8])]));
    }
    ops
}

/// The ops that put the hat model on the head, from the entity's frame (feet at the origin, facing +z). `head` is the entity's head
/// turn in radians (pitch, then yaw), `posed` Horizon's pose of a humanoid head, and `entity_scale` the state's own scale.
pub fn mount_ops(mount: &ModelMount, head: [f64; 2], posed: Option<&HeadPose>, entity_scale: f32) -> Vec<Op> {
    let s = f64::from(entity_scale);
    let mut ops = vec![Op::Scale([s, s, s]), Op::Scale([1.0, -1.0, -1.0]), Op::Translate([0.0, -MODEL_ORIGIN, 0.0])];
    for step in &mount.steps {
        let mut pose = step.pose;
        match (step.turn, posed) {
            (Turn::Posed, Some(p)) => {
                for i in 0..3 {
                    pose[i] = p.position[i] as f32;
                    pose[3 + i] = p.angles[i] as f32;
                }
            }
            // No pose to take: a humanoid head turns as any other.
            (Turn::Head | Turn::Posed, _) => {
                pose[3] = head[0] as f32;
                pose[4] = head[1] as f32;
            }
            (Turn::Rest, _) => {}
        }
        ops.extend(part_ops(&pose));
    }
    if let Some(cube) = mount.cube {
        let (min_y, max_y) = (f64::from(cube[1]), f64::from(cube[4]));
        let off = mount.offset;
        ops.push(Op::Translate([off[0] / 16.0, (min_y - max_y + off[1]) / 16.0, off[2] / 16.0]));
        let size = f64::from((cube[3] - cube[0]).max(cube[5] - cube[2]) / 8.0 * mount.scale);
        ops.push(Op::Scale([size, size, size]));
    }
    ops.push(Op::Scale([1.0, -1.0, -1.0]));
    ops.push(Op::Translate([0.0, -2.25 / 16.0, 0.0]));
    ops.push(Op::Rotate([1.0, 0.0, 0.0], -8.5));
    ops
}

#[cfg(test)]
mod tests {
    use super::super::track_overlay::compose;
    use super::*;

    const REST: [f32; 9] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];

    fn node(name: &str, pose: [f32; 9], cubes: Vec<[f32; 6]>, children: Vec<ModelNode>) -> ModelNode {
        ModelNode { name: name.to_owned(), pose, cubes, children }
    }

    fn at(x: f32, y: f32, z: f32) -> [f32; 9] {
        [x, y, z, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
    }

    /// The vanilla cow: a head 8 by 8 by 6 at `(0, 4, -8)`.
    fn cow() -> ModelNode {
        node(
            "root",
            REST,
            vec![],
            vec![
                node("head", at(0.0, 4.0, -8.0), vec![[-4.0, -4.0, -6.0, 4.0, 4.0, 0.0], [-3.0, 1.0, -7.0, 3.0, 4.0, -6.0]], vec![]),
                node("body", at(0.0, 5.0, 2.0), vec![[-6.0, -10.0, -7.0, 6.0, 8.0, 5.0]], vec![]),
            ],
        )
    }

    #[test]
    fn the_table_is_the_generator_s() {
        assert_eq!(INFO.len(), 58);
        assert!(INFO.iter().all(|(name, _)| name.starts_with("minecraft:")));
        // No type twice.
        let mut names: Vec<&str> = types().collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), INFO.len());
        assert_eq!(info_for("minecraft:cow"), HatInfo { part: "", cube_index: 0, offset: [0.0, 3.0, -3.0], scale: 0.87 });
        assert_eq!(info_for("minecraft:camel"), HatInfo { part: "body/head", cube_index: 1, offset: [0.0, -8.0, -11.5], scale: 1.0 });
        assert_eq!(info_for("minecraft:goat").offset, [-0.5, -8.5, -9.0]);
        assert_eq!(info_for("minecraft:wolf").part, "real_head");
        assert_eq!(info_for("minecraft:slime").scale, 1.25);
        assert_eq!(info_for("minecraft:armadillo").part, "body/head/head_cube");
        // A type Create has nothing for gets the default.
        assert_eq!(info_for("minecraft:creeper"), DEFAULT);
        assert!(!has_info("minecraft:creeper"));
        assert_eq!(SNOW_GOLEM_PUMPKIN.offset, [0.0, -3.0, 0.0]);
    }

    #[test]
    fn six_faces_make_a_box() {
        // Two boxes: a 2 by 2 by 2 at the origin and a 4 by 1 by 1 beside it, each as six faces (all that matters is the corners).
        let face = |lo: [f32; 3], hi: [f32; 3]| [lo, [hi[0], lo[1], lo[2]], [hi[0], hi[1], lo[2]], hi];
        let a = ([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        let b = ([5.0, 0.0, 0.0], [9.0, 1.0, 1.0]);
        let mut faces = Vec::new();
        for (lo, hi) in [a, b] {
            for _ in 0..6 {
                faces.push(face(lo, hi));
            }
        }
        assert_eq!(cubes_of(&faces), vec![[0.0, 0.0, 0.0, 2.0, 2.0, 2.0], [5.0, 0.0, 0.0, 9.0, 1.0, 1.0]]);
        assert!(cubes_of(&[]).is_empty());
        // Five faces: one box.
        assert_eq!(cubes_of(&faces[..5]).len(), 1);
    }

    #[test]
    fn a_cow_wears_the_hat_on_its_head() {
        let mount = resolve(&cow(), "minecraft:cow", &info_for("minecraft:cow")).unwrap();
        assert_eq!(mount.steps.len(), 2);
        assert_eq!(mount.steps[1].turn, Turn::Head);
        assert_eq!(mount.cube, Some([-4.0, -4.0, -6.0, 4.0, 4.0, 0.0]));
        let foot = compose(&mount_ops(&mount, [0.0, 0.0], None, 1.0)).apply([0.0; 3]);
        // 1.501 above the model's origin, less the head's 4 texels down and the offset's (3 - 8) texels, the hat sunk 2.25 texels,
        // and in front of the cow by the head's 8 and the offset's 3 texels.
        let size = 0.87;
        let expect_y = MODEL_ORIGIN - (4.0 / 16.0) - (3.0 - 8.0) / 16.0 - 2.25 / 16.0 * size;
        assert!((foot[1] - expect_y).abs() < 1e-6, "{foot:?} {expect_y}");
        assert!((foot[2] - (8.0 + 3.0) / 16.0).abs() < 1e-6, "{foot:?}");
        assert!(foot[0].abs() < 1e-9);
    }

    #[test]
    fn the_hat_is_as_big_as_the_head_s_wider_side() {
        // A 6 wide head gets 3/4 of the hat of an 8 wide head.
        let mut root = cow();
        root.children[0].cubes = vec![[-3.0, -3.0, -3.0, 3.0, 3.0, 3.0]];
        let small = resolve(&root, "minecraft:cow", &HatInfo { scale: 1.0, ..info_for("minecraft:cow") }).unwrap();
        let big = resolve(&cow(), "minecraft:cow", &HatInfo { scale: 1.0, ..info_for("minecraft:cow") }).unwrap();
        let width = |m: &ModelMount| {
            let a = compose(&mount_ops(m, [0.0, 0.0], None, 1.0));
            let (p, q) = (a.apply([0.0; 3]), a.apply([1.0, 0.0, 0.0]));
            ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt()
        };
        assert!((width(&small) / width(&big) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_part_is_found_below_another_when_the_root_lacks_it() {
        // The wolf's `real_head` is under `head`; the bee's `body` under `bone`; the frog's chain starts at its own `root`.
        let wolf = node(
            "root",
            REST,
            vec![],
            vec![node("head", at(-1.0, 13.5, -7.0), vec![], vec![node("real_head", REST, vec![[-3.0, -3.0, -4.0, 3.0, 3.0, 0.0]], vec![])])],
        );
        let mount = resolve(&wolf, "minecraft:wolf", &info_for("minecraft:wolf")).unwrap();
        assert_eq!(mount.steps.len(), 3);
        // The `head` of the chain is the turned one, not `real_head`.
        assert_eq!(mount.steps.iter().map(|s| s.turn).collect::<Vec<_>>(), vec![Turn::Rest, Turn::Head, Turn::Rest]);
        let bee = node("root", REST, vec![], vec![node("bone", at(0.0, 19.0, 0.0), vec![], vec![node("body", REST, vec![[-3.5, -4.0, -5.0, 3.5, 3.0, 5.0]], vec![])])]);
        assert_eq!(resolve(&bee, "minecraft:bee", &info_for("minecraft:bee")).unwrap().steps.len(), 3);
        // A model with no such part gets no hat.
        assert!(resolve(&bee, "minecraft:cow", &info_for("minecraft:cow")).is_none());
        assert!(resolve(&bee, "minecraft:zombie", &DEFAULT).is_none());
    }

    #[test]
    fn a_path_is_followed_from_the_root_first() {
        let tree = node("root", REST, vec![], vec![node("body", at(0.0, 20.0, 0.0), vec![], vec![node("head", at(0.0, 0.0, -9.0), vec![[-4.0, -4.0, -4.0, 4.0, 4.0, 4.0]], vec![])])]);
        let breeze = resolve(&tree, "minecraft:breeze", &info_for("minecraft:breeze")).unwrap();
        assert_eq!(breeze.steps.len(), 3);
        assert_eq!(breeze.cube, Some([-4.0, -4.0, -4.0, 4.0, 4.0, 4.0]));
    }

    #[test]
    fn horses_have_their_head_in_head_parts() {
        let horse = node(
            "root",
            [0.0, -2.4, 0.0, 0.0, 0.0, 0.0, 1.1, 1.1, 1.1],
            vec![],
            vec![node("head_parts", [0.0, 4.0, -12.0, 0.52, 0.0, 0.0, 1.0, 1.0, 1.0], vec![[-2.0, -11.0, -2.0, 2.0, 1.0, 2.0]], vec![node("head", REST, vec![[-3.0, -6.0, -2.0, 3.0, 0.0, 6.0]], vec![])])],
        );
        let mount = resolve(&horse, "minecraft:horse", &info_for("minecraft:horse")).unwrap();
        assert_eq!(mount.steps.len(), 2);
        assert_eq!(mount.cube, Some([-2.0, -11.0, -2.0, 2.0, 1.0, 2.0]));
        // The root's scale (the renderer's) is in the chain.
        let ops = mount_ops(&mount, [0.0, 0.0], None, 1.0);
        assert!(ops.contains(&Op::Scale([f64::from(1.1_f32); 3])));
    }

    #[test]
    fn a_part_without_boxes_takes_no_offset_or_scale() {
        // `if (!lastChild.isEmpty())`.
        let tree = node("root", REST, vec![], vec![node("head", at(0.0, 4.0, 0.0), vec![], vec![])]);
        let mount = resolve(&tree, "minecraft:pig", &info_for("minecraft:pig")).unwrap();
        assert_eq!(mount.cube, None);
        let ops = mount_ops(&mount, [0.0, 0.0], None, 1.0);
        let big = ops.iter().filter(|o| matches!(o, Op::Scale(v) if v[0] == v[1] && v[1] == v[2] && v[0] != 1.0)).count();
        assert_eq!(big, 0);
    }

    #[test]
    fn a_box_index_past_the_end_takes_the_last() {
        let tree = node("root", REST, vec![], vec![node("head", REST, vec![[0.0; 6], [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]], vec![])]);
        let info = HatInfo { cube_index: 7, ..DEFAULT };
        assert_eq!(resolve(&tree, "minecraft:pig", &info).unwrap().cube, Some([1.0, 1.0, 1.0, 2.0, 2.0, 2.0]));
    }

    #[test]
    fn the_head_turns_the_hat_with_pitch_then_yaw() {
        let mount = resolve(&cow(), "minecraft:cow", &info_for("minecraft:cow")).unwrap();
        let rest = compose(&mount_ops(&mount, [0.0, 0.0], None, 1.0)).apply([0.0, 1.0, 0.0]);
        let turned = compose(&mount_ops(&mount, [0.0, std::f64::consts::FRAC_PI_2], None, 1.0)).apply([0.0, 1.0, 0.0]);
        // A quarter turn of the head moves the hat's front sideways.
        let front = |m: &ModelMount, yaw: f64| compose(&mount_ops(m, [0.0, yaw], None, 1.0)).apply([0.0, 0.0, 1.0]);
        assert!((front(&mount, 0.0)[0] - front(&mount, std::f64::consts::FRAC_PI_2)[0]).abs() > 0.1);
        // The foot of the hat stays above the head either way.
        assert!(rest[1] > 1.0 && turned[1] > 1.0);
    }

    #[test]
    fn the_part_s_turns_are_taken_z_then_y_then_x() {
        // `rotationZYX(z, y, x)` is Rz * Ry * Rx: a point is turned about x first.
        let (x, y) = (0.5_f64, 0.7_f64);
        let got = compose(&rotation_ops([x, y, 0.0])).apply([1.0, 2.0, 3.0]);
        let (sx, cx) = x.sin_cos();
        let (sy, cy) = y.sin_cos();
        let p = [1.0, 2.0 * cx - 3.0 * sx, 2.0 * sx + 3.0 * cx];
        let want = [p[0] * cy + p[2] * sy, p[1], -p[0] * sy + p[2] * cy];
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-5, "{got:?} {want:?}");
        }
    }

    #[test]
    fn a_humanoid_head_takes_the_game_s_pose() {
        let tree = node("root", REST, vec![], vec![node("head", REST, vec![[-4.0, -8.0, -4.0, 4.0, 0.0, 4.0]], vec![])]);
        let mount = resolve(&tree, "minecraft:piglin", &info_for("minecraft:piglin")).unwrap();
        assert_eq!(mount.steps[1].turn, Turn::Posed);
        let posed = HeadPose { position: [0.0, 2.0, 0.0], angles: [0.0; 3] };
        let high = compose(&mount_ops(&mount, [0.0, 0.0], Some(&posed), 1.0)).apply([0.0; 3]);
        let rest = compose(&mount_ops(&mount, [0.0, 0.0], None, 1.0)).apply([0.0; 3]);
        // The head 2 texels lower in the model is 2 texels lower in the world.
        assert!((rest[1] - high[1] - 2.0 / 16.0).abs() < 1e-6);
        // Without a pose it turns with the entity's head like any other.
        let turned = compose(&mount_ops(&mount, [0.0, 1.0], None, 1.0)).apply([0.0, 0.0, 1.0]);
        let straight = compose(&mount_ops(&mount, [0.0, 0.0], None, 1.0)).apply([0.0, 0.0, 1.0]);
        assert!((turned[0] - straight[0]).abs() > 0.1);
    }

    #[test]
    fn the_entity_s_scale_shrinks_the_whole_hat() {
        let mount = resolve(&cow(), "minecraft:cow", &info_for("minecraft:cow")).unwrap();
        let full = compose(&mount_ops(&mount, [0.0; 2], None, 1.0)).apply([0.0; 3]);
        let half = compose(&mount_ops(&mount, [0.0; 2], None, 0.5)).apply([0.0; 3]);
        assert!((half[1] - full[1] * 0.5).abs() < 1e-9 && (half[2] - full[2] * 0.5).abs() < 1e-9);
    }
}

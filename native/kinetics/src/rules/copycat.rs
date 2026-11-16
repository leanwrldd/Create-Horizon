//! Copycats: which blocks they take as a material, how a click sets or cycles it, the shapes and
//! the way the material is drawn. Ported from Create `mc1.21.1/dev` at 4148fd8 (MIT, the Creators
//! of Create): `CopycatBlock` (`useItemOn`, `getAcceptedBlockState`, `setPlacedBy`),
//! `CopycatBlockEntity.cycleMaterial`, `CopycatStepBlock`, `CopycatPanelBlock`,
//! `CopycatSpecialCases`, `AllShapes.STEP_BOTTOM/STEP_TOP/CASING_3PX`. No game types.
//!
//! Known differences from Create:
//! - Create crops the material's own model to the shape (`CopycatStepModel`, `CopycatPanelModel`,
//!   `BakedModelHelper.cropAndMove`); here the material's whole block is drawn scaled into the shape
//!   ([`draw_plan`]), so its texture is squashed instead of cut. A trapdoor material is drawn as
//!   itself, and iron bars as the Copycat Bars model with its own texture, not the material's.
//! - Which blocks are accepted is baked (`copycat_facts`): a full outline and full collision, no
//!   block entity, no stair, not denied. `copycat_allow` (the barrel) is in.

use super::copycat_facts as facts;
use crate::Dir;

/// `AllBlocks.COPYCAT_BASE`: the material of a Copycat with none.
pub const BASE: &str = "create:copycat_base";
/// `AllBlocks.COPYCAT_BARS`: the model a panel draws for a bars material.
pub const BARS_MODEL: &str = "create:copycat_bars";
pub const STEP: &str = "create:copycat_step";
pub const PANEL: &str = "create:copycat_panel";

/// A block state, as a block and the properties set on it (the rest default).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub block: String,
    pub props: Vec<(String, String)>,
}

impl Material {
    pub fn base() -> Material {
        Material { block: BASE.to_owned(), props: Vec::new() }
    }

    /// `!COPYCAT_BASE.has(material)`.
    pub fn is_custom(&self) -> bool {
        self.block != BASE
    }

    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn with(mut self, key: &str, value: &str) -> Material {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value.to_owned(),
            None => self.props.push((key.to_owned(), value.to_owned())),
        }
        self
    }
}

fn listed(list: &[&str], block: &str) -> bool {
    list.binary_search(&block).is_ok()
}

/// `CopycatSpecialCases.isBarsMaterial`.
pub fn is_bars(block: &str) -> bool {
    listed(facts::BARS, block)
}

/// `CopycatSpecialCases.isTrapdoorMaterial`.
pub fn is_trapdoor(block: &str) -> bool {
    listed(facts::TRAPDOORS, block)
}

/// `CopycatBlock.getAcceptedBlockState` for a block item, clicked on `face`: the state it gives the
/// Copycat. A panel also takes bars and trapdoors (`CopycatPanelBlock.isAcceptedRegardless`).
pub fn accepted(block: &str, face: Option<Dir>, panel: bool) -> Option<Material> {
    let regardless = panel && (is_bars(block) || is_trapdoor(block));
    if !regardless && !listed(facts::ACCEPTED, block) {
        return None;
    }
    let mut material = Material { block: block.to_owned(), props: Vec::new() };
    if let Some(face) = face {
        let axis = face.axis();
        if listed(facts::FACING, block) {
            material = material.with("facing", face.name());
        }
        if listed(facts::HORIZONTAL_FACING, block) && axis != crate::Axis::Y {
            material = material.with("facing", face.name());
        }
        if listed(facts::AXIS, block) {
            material = material.with("axis", axis.name());
        }
        if listed(facts::HORIZONTAL_AXIS, block) && axis != crate::Axis::Y {
            material = material.with("axis", axis.name());
        }
    }
    Some(material)
}

/// `CopycatPanelBlock.prepareMaterial`: a trapdoor takes the panel's place. `hit_y` is the click's
/// height in the block (0 to 1) and `player` the horizontal direction the player faces.
pub fn prepare_material(material: Material, panel_facing: Dir, hit_y: f64, player: Dir) -> Material {
    if !is_trapdoor(&material.block) {
        return material;
    }
    let mut material = material;
    if panel_facing == Dir::Down {
        material = material.with("half", "top");
    }
    if panel_facing.axis() == crate::Axis::Y {
        return material.with("facing", player.name()).with("open", "false");
    }
    let near_top = hit_y - 0.5 > 0.0;
    material
        .with("open", "true")
        .with("half", if near_top { "top" } else { "bottom" })
        .with("facing", panel_facing.name())
}

fn next<'a>(order: &[&'a str], current: &str) -> &'a str {
    let at = order.iter().position(|v| *v == current).unwrap_or(0);
    order[(at + 1) % order.len()]
}

/// `CopycatBlockEntity.cycleMaterial`: the material turned one step, if it has a step to turn.
pub fn cycle(material: &Material) -> Option<Material> {
    let block = material.block.as_str();
    if is_trapdoor(block) && material.prop("open") == Some("true") && material.prop("half").is_some() {
        let half = if material.prop("half") == Some("top") { "bottom" } else { "top" };
        return Some(material.clone().with("half", half));
    }
    let value = |key: &str, default: &'static str| material.prop(key).unwrap_or(default).to_owned();
    if listed(facts::FACING, block) {
        // `Direction.values()`' order.
        let order = ["down", "up", "north", "south", "west", "east"];
        return Some(material.clone().with("facing", next(&order, &value("facing", "north"))));
    }
    if listed(facts::HORIZONTAL_FACING, block) {
        // `getClockWise()`.
        let order = ["north", "east", "south", "west"];
        return Some(material.clone().with("facing", next(&order, &value("facing", "north"))));
    }
    if listed(facts::AXIS, block) {
        return Some(material.clone().with("axis", next(&["x", "y", "z"], &value("axis", "y"))));
    }
    if listed(facts::HORIZONTAL_AXIS, block) {
        return Some(material.clone().with("axis", next(&["x", "z"], &value("axis", "x"))));
    }
    if listed(facts::LIT, block) {
        return Some(material.clone().with("lit", next(&["true", "false"], &value("lit", "false"))));
    }
    None
}

/// What a click with a block item does (`CopycatBlock.useItemOn`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Nothing for the Copycat to do: the click goes on.
    Pass,
    /// The same block again: the material turns.
    Cycle(Material),
    /// The item becomes the material.
    Set(Material),
}

/// `current` is the Copycat's material, `applied` what its item gives (after
/// [`prepare_material`]); `None` for an item that gives nothing.
pub fn click(current: &Material, applied: Option<Material>) -> Click {
    let Some(applied) = applied else {
        return Click::Pass;
    };
    if current.block == applied.block {
        return match cycle(current) {
            Some(turned) => Click::Cycle(turned),
            None => Click::Pass,
        };
    }
    if current.is_custom() {
        return Click::Pass;
    }
    Click::Set(applied)
}

/// `Direction.fromYRot`: the horizontal direction a yaw faces.
pub fn horizontal_of_yaw(yaw: f32) -> Dir {
    match ((f64::from(yaw) / 90.0 + 0.5).floor() as i64).rem_euclid(4) {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `CopycatStepBlock.getStateForPlacement`: whether the step is the top half, and its facing.
/// `hit_y` is the click's height in the block.
pub fn step_placement(player: Dir, clicked_face: Dir, hit_y: f64) -> (bool, Dir) {
    let top = clicked_face == Dir::Down || (clicked_face != Dir::Up && hit_y > 0.5);
    (top, player)
}

/// `CopycatPanelBlock.getStateForPlacement`: `getNearestLookingDirection().getOpposite()`.
pub fn panel_placement(looking: Dir) -> Dir {
    looking.opposite()
}

/// `AllShapes.STEP_BOTTOM` / `STEP_TOP`: `shape(0, 0, 8, 16, 8, 16).forHorizontal(SOUTH)`, as
/// `[min x, y, z, max x, y, z]` in blocks: the half towards `facing`, the lower or upper half.
pub fn step_box(facing: Dir, top: bool) -> [f64; 6] {
    let (y0, y1) = if top { (0.5, 1.0) } else { (0.0, 0.5) };
    match facing {
        Dir::South => [0.0, y0, 0.5, 1.0, y1, 1.0],
        Dir::North => [0.0, y0, 0.0, 1.0, y1, 0.5],
        Dir::East => [0.5, y0, 0.0, 1.0, y1, 1.0],
        _ => [0.0, y0, 0.0, 0.5, y1, 1.0],
    }
}

/// `AllShapes.CASING_3PX.get(facing)`: a slab 3/16 thick on the side opposite `facing`.
pub fn panel_box(facing: Dir) -> [f64; 6] {
    let t = 3.0 / 16.0;
    match facing {
        Dir::Up => [0.0, 0.0, 0.0, 1.0, t, 1.0],
        Dir::Down => [0.0, 1.0 - t, 0.0, 1.0, 1.0, 1.0],
        Dir::North => [0.0, 0.0, 1.0 - t, 1.0, 1.0, 1.0],
        Dir::South => [0.0, 0.0, 0.0, 1.0, 1.0, t],
        Dir::East => [0.0, 0.0, 0.0, t, 1.0, 1.0],
        Dir::West => [1.0 - t, 0.0, 0.0, 1.0, 1.0, 1.0],
    }
}

/// One block model to draw: the block and its properties, moved by `translate` then scaled by
/// `scale` about that corner (blocks).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub block: String,
    pub props: Vec<(String, String)>,
    pub translate: [f32; 3],
    pub scale: [f32; 3],
}

/// What a Copycat shows: the material drawn into the Copycat's box (`CopycatStepModel`,
/// `CopycatPanelModel`, `CopycatBarsModel`). `shape` is the Copycat's own box.
pub fn draw_plan(panel_facing: Option<Dir>, shape: [f64; 6], material: &Material) -> Vec<Part> {
    if let Some(facing) = panel_facing {
        if is_trapdoor(&material.block) {
            // `CopycatPanelModel`: the trapdoor's own model, as it is.
            return vec![Part {
                block: material.block.clone(),
                props: material.props.clone(),
                translate: [0.0; 3],
                scale: [1.0; 3],
            }];
        }
        if is_bars(&material.block) {
            return vec![Part {
                block: BARS_MODEL.to_owned(),
                props: vec![("facing".to_owned(), facing.name().to_owned())],
                translate: [0.0; 3],
                scale: [1.0; 3],
            }];
        }
    }
    vec![Part {
        block: material.block.clone(),
        props: material.props.clone(),
        translate: [shape[0] as f32, shape[1] as f32, shape[2] as f32],
        scale: [(shape[3] - shape[0]) as f32, (shape[4] - shape[1]) as f32, (shape[5] - shape[2]) as f32],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> Material {
        accepted("minecraft:stone", Some(Dir::North), false).unwrap()
    }

    #[test]
    fn full_cubes_are_accepted_and_others_are_not() {
        assert!(accepted("minecraft:stone", None, false).is_some());
        assert!(accepted("minecraft:barrel", None, false).is_some());
        assert!(accepted("create:andesite_casing", None, false).is_some());
        assert!(accepted("minecraft:oak_stairs", None, false).is_none());
        assert!(accepted("minecraft:chest", None, false).is_none());
        assert!(accepted("minecraft:oak_slab", None, false).is_none());
        assert!(accepted("minecraft:oak_trapdoor", None, false).is_none());
        assert!(accepted("minecraft:iron_bars", None, false).is_none());
        // A panel takes bars and trapdoors as well.
        assert!(accepted("minecraft:iron_bars", None, true).is_some());
        assert!(accepted("minecraft:oak_trapdoor", None, true).is_some());
        assert!(accepted("create:copycat_step", None, true).is_none());
    }

    #[test]
    fn a_material_turns_to_the_face_clicked() {
        let log = accepted("minecraft:oak_log", Some(Dir::East), false).unwrap();
        assert_eq!(log.prop("axis"), Some("x"));
        let barrel = accepted("minecraft:barrel", Some(Dir::Up), false).unwrap();
        assert_eq!(barrel.prop("facing"), Some("up"));
        // A furnace-like block faces only a horizontal face.
        let dropper = accepted("minecraft:dispenser", Some(Dir::West), false);
        assert!(dropper.is_none() || dropper.unwrap().prop("facing").is_some());
        let plain = accepted("minecraft:stone", Some(Dir::West), false).unwrap();
        assert!(plain.props.is_empty());
    }

    #[test]
    fn a_trapdoor_clicked_on_a_panel_takes_the_panels_place() {
        let door = accepted("minecraft:oak_trapdoor", None, true).unwrap();
        // A wall panel: open, against the panel, half by where the click was.
        let wall = prepare_material(door.clone(), Dir::North, 0.8, Dir::East);
        assert_eq!(wall.prop("open"), Some("true"));
        assert_eq!(wall.prop("half"), Some("top"));
        assert_eq!(wall.prop("facing"), Some("north"));
        assert_eq!(prepare_material(door.clone(), Dir::North, 0.2, Dir::East).prop("half"), Some("bottom"));
        // A floor or ceiling panel: closed, facing the player; a ceiling one on the top half.
        let floor = prepare_material(door.clone(), Dir::Up, 0.9, Dir::East);
        assert_eq!(floor.prop("open"), Some("false"));
        assert_eq!(floor.prop("facing"), Some("east"));
        assert_eq!(prepare_material(door, Dir::Down, 0.0, Dir::East).prop("half"), Some("top"));
        // Anything else is left alone.
        assert_eq!(prepare_material(stone(), Dir::North, 0.8, Dir::East), stone());
    }

    #[test]
    fn clicking_the_same_block_again_cycles_and_others_do_nothing_once_set() {
        let base = Material::base();
        assert_eq!(click(&base, Some(stone())), Click::Set(stone()));
        assert_eq!(click(&base, None), Click::Pass);
        // Set to stone: another block does nothing; stone again has nothing to cycle.
        let other = accepted("minecraft:glass", None, false).unwrap();
        assert_eq!(click(&stone(), Some(other)), Click::Pass);
        assert_eq!(click(&stone(), Some(stone())), Click::Pass);
        // A log turns through x, y, z.
        let log = accepted("minecraft:oak_log", Some(Dir::Up), false).unwrap();
        let Click::Cycle(turned) = click(&log, Some(log.clone())) else { panic!() };
        assert_eq!(turned.prop("axis"), Some("z"));
        let Click::Cycle(turned) = click(&turned, Some(log)) else { panic!() };
        assert_eq!(turned.prop("axis"), Some("x"));
    }

    #[test]
    fn facings_cycle_in_the_games_order() {
        let barrel = accepted("minecraft:barrel", Some(Dir::Up), false).unwrap();
        let mut seen = vec![barrel.prop("facing").unwrap().to_owned()];
        let mut m = barrel;
        for _ in 0..6 {
            m = cycle(&m).unwrap();
            seen.push(m.prop("facing").unwrap().to_owned());
        }
        assert_eq!(seen, ["up", "north", "south", "west", "east", "down", "up"]);
    }

    #[test]
    fn an_open_trapdoor_flips_its_half() {
        let door = accepted("minecraft:oak_trapdoor", None, true).unwrap();
        let open = prepare_material(door, Dir::North, 0.2, Dir::East);
        assert_eq!(cycle(&open).unwrap().prop("half"), Some("top"));
        assert_eq!(cycle(&cycle(&open).unwrap()).unwrap().prop("half"), Some("bottom"));
        // A closed one has nothing to turn.
        assert_eq!(cycle(&open.with("open", "false")), None);
    }

    #[test]
    fn yaws_and_clicks_pick_the_placement() {
        assert_eq!(horizontal_of_yaw(0.0), Dir::South);
        assert_eq!(horizontal_of_yaw(90.0), Dir::West);
        assert_eq!(horizontal_of_yaw(180.0), Dir::North);
        assert_eq!(horizontal_of_yaw(-90.0), Dir::East);
        assert_eq!(step_placement(Dir::South, Dir::Up, 1.0), (false, Dir::South));
        assert_eq!(step_placement(Dir::South, Dir::Down, 0.0), (true, Dir::South));
        assert_eq!(step_placement(Dir::South, Dir::North, 0.7), (true, Dir::South));
        assert_eq!(step_placement(Dir::South, Dir::North, 0.3), (false, Dir::South));
        assert_eq!(panel_placement(Dir::Down), Dir::Up);
    }

    #[test]
    fn the_shapes_are_creates() {
        assert_eq!(step_box(Dir::South, false), [0.0, 0.0, 0.5, 1.0, 0.5, 1.0]);
        assert_eq!(step_box(Dir::North, true), [0.0, 0.5, 0.0, 1.0, 1.0, 0.5]);
        assert_eq!(step_box(Dir::East, false), [0.5, 0.0, 0.0, 1.0, 0.5, 1.0]);
        assert_eq!(step_box(Dir::West, false), [0.0, 0.0, 0.0, 0.5, 0.5, 1.0]);
        assert_eq!(panel_box(Dir::Up), [0.0, 0.0, 0.0, 1.0, 0.1875, 1.0]);
        assert_eq!(panel_box(Dir::Down)[1], 0.8125);
        assert_eq!(panel_box(Dir::North)[2], 0.8125);
        assert_eq!(panel_box(Dir::West)[0], 0.8125);
        assert_eq!(panel_box(Dir::South)[5], 0.1875);
    }

    #[test]
    fn the_material_is_drawn_into_the_shape() {
        let shape = step_box(Dir::South, false);
        let parts = draw_plan(None, shape, &stone());
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].translate, [0.0, 0.0, 0.5]);
        assert_eq!(parts[0].scale, [1.0, 0.5, 0.5]);
        // Bars become the Copycat Bars model, facing the panel's way; a trapdoor draws itself.
        let bars = accepted("minecraft:iron_bars", None, true).unwrap();
        let drawn = draw_plan(Some(Dir::East), panel_box(Dir::East), &bars);
        assert_eq!(drawn[0].block, BARS_MODEL);
        assert_eq!(drawn[0].props, vec![("facing".to_owned(), "east".to_owned())]);
        let door = accepted("minecraft:oak_trapdoor", None, true).unwrap();
        assert_eq!(draw_plan(Some(Dir::Up), panel_box(Dir::Up), &door)[0].scale, [1.0; 3]);
        // The base shows as the base block.
        assert_eq!(draw_plan(None, shape, &Material::base())[0].block, BASE);
    }
}

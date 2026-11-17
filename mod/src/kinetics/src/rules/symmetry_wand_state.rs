//! Block-state turns for the Wand of Symmetry: what `BlockState.mirror(..)` and `.rotate(..)` do to a
//! state's properties (Create's `SymmetryMirror.flipX/flipZ/flipD1/flipD2`).
//!
//! A mod can't ask the game to mirror a state, and can't list a state's properties either, so the glue reads the
//! state as the text `/setblock` takes (`minecraft:oak_stairs[facing=east,half=bottom,...]`) and this module turns
//! the text. The families it knows are vanilla's: a horizontal or six-way `facing`, a pillar's `axis`, a sixteen
//! step `rotation`, fence-like `north`/`east`/`south`/`west` sides, a rail's `shape`, stairs, doors (`hinge`),
//! double chests (`type`), the crafter's `orientation`, and Create's `axis_along_first`
//! (`DirectionalAxisKineticBlock.rotate`: a quarter turn flips it).
//!
//! Known differences from vanilla (no vanilla source is at hand, so this is written from what the turns do to
//! the shapes): a stair's corner shape swaps hands on every mirror (vanilla swaps it only when the stair faces
//! across the mirror; Create's `updateShape` pass after placing fixes the rest), and a block that only shares a
//! property name with these families turns like them.

use std::collections::BTreeMap;

/// `Mirror`: which coordinate a mirror flips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flip {
    /// `Mirror.FRONT_BACK`: x becomes -x (east and west swap).
    FrontBack,
    /// `Mirror.LEFT_RIGHT`: z becomes -z (north and south swap).
    LeftRight,
}

/// `Rotation` as clockwise quarter turns (`CLOCKWISE_90` is 1).
pub type Quarters = u8;

/// `Rotation.COUNTERCLOCKWISE_90`.
pub const COUNTERCLOCKWISE_90: Quarters = 3;
/// `Rotation.CLOCKWISE_180`.
pub const CLOCKWISE_180: Quarters = 2;

const SIDES: [&str; 4] = ["north", "east", "south", "west"];

const RAIL_SHAPES: [&str; 10] = [
    "north_south",
    "east_west",
    "ascending_east",
    "ascending_west",
    "ascending_north",
    "ascending_south",
    "south_east",
    "south_west",
    "north_west",
    "north_east",
];

/// A state as its block id and its properties.
pub type Props = BTreeMap<String, String>;

/// `minecraft:oak_stairs[facing=east,half=bottom]` as `("minecraft:oak_stairs", {facing: east, half: bottom})`.
pub fn parse(text: &str) -> (String, Props) {
    let Some((name, rest)) = text.split_once('[') else {
        return (text.to_owned(), Props::new());
    };
    let props = rest
        .trim_end_matches(']')
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    (name.to_owned(), props)
}

/// The text `/setblock` takes for a block and its properties.
pub fn format(name: &str, props: &Props) -> String {
    if props.is_empty() {
        return name.to_owned();
    }
    let list: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", list.join(","))
}

/// `Rotation.rotate(Direction)`: a horizontal direction turned clockwise, others unchanged.
fn rotate_dir(dir: &str, quarters: Quarters) -> String {
    match SIDES.iter().position(|d| *d == dir) {
        Some(i) => SIDES[(i + usize::from(quarters)) % 4].to_owned(),
        None => dir.to_owned(),
    }
}

/// `Mirror.getRotation(Direction)`: a direction along the mirrored coordinate turns round.
fn mirror_rotation(flip: Flip, dir: &str) -> Quarters {
    match (flip, dir) {
        (Flip::FrontBack, "east" | "west") | (Flip::LeftRight, "north" | "south") => CLOCKWISE_180,
        _ => 0,
    }
}

/// `Mirror.mirror(Direction)`.
fn mirror_dir(flip: Flip, dir: &str) -> String {
    if mirror_rotation(flip, dir) == CLOCKWISE_180 { rotate_dir(dir, CLOCKWISE_180) } else { dir.to_owned() }
}

fn swapped(value: &str, a: &str, b: &str) -> String {
    if value == a {
        b.to_owned()
    } else if value == b {
        a.to_owned()
    } else {
        value.to_owned()
    }
}

fn is_rail(props: &Props) -> bool {
    props.get("shape").is_some_and(|s| RAIL_SHAPES.contains(&s.as_str()))
}

fn is_stairs(props: &Props) -> bool {
    props.contains_key("facing")
        && props.contains_key("half")
        && props
            .get("shape")
            .is_some_and(|s| matches!(s.as_str(), "straight" | "inner_left" | "inner_right" | "outer_left" | "outer_right"))
}

fn has_sides(props: &Props) -> bool {
    SIDES.iter().all(|s| props.contains_key(*s))
}

/// `BlockState.rotate(rotation)`.
pub fn rotate(props: &mut Props, quarters: Quarters) {
    let quarters = quarters % 4;
    if quarters == 0 {
        return;
    }
    if let Some(v) = props.get_mut("rotation")
        && let Ok(n) = v.parse::<i32>()
    {
        // `Rotation.rotate(int, 16)`.
        *v = ((n + 4 * i32::from(quarters)).rem_euclid(16)).to_string();
    }
    if is_rail(props) {
        let mut shape = props["shape"].clone();
        for _ in 0..quarters {
            shape = match shape.as_str() {
                "north_south" => "east_west",
                "east_west" => "north_south",
                "ascending_east" => "ascending_south",
                "ascending_south" => "ascending_west",
                "ascending_west" => "ascending_north",
                "ascending_north" => "ascending_east",
                "south_east" => "south_west",
                "south_west" => "north_west",
                "north_west" => "north_east",
                "north_east" => "south_east",
                other => other,
            }
            .to_owned();
        }
        props.insert("shape".into(), shape);
    }
    if let Some(f) = props.get_mut("facing") {
        *f = rotate_dir(f, quarters);
    }
    if let Some(o) = props.get_mut("orientation")
        && let Some((front, top)) = o.split_once('_')
    {
        *o = format!("{}_{}", rotate_dir(front, quarters), rotate_dir(top, quarters));
    }
    if quarters % 2 == 1 {
        if let Some(a) = props.get_mut("axis") {
            *a = swapped(a, "x", "z");
        }
        if let Some(v) = props.get_mut("axis_along_first") {
            *v = swapped(v, "true", "false");
        }
    }
    if has_sides(props) {
        let old: Vec<String> = SIDES.iter().map(|s| props[*s].clone()).collect();
        for (i, side) in SIDES.iter().enumerate() {
            // The side that turns to `side`.
            let from = (i + 4 - usize::from(quarters)) % 4;
            props.insert((*side).to_owned(), old[from].clone());
        }
    }
}

/// `Mirror.mirror(int rotation, 16)`.
fn mirror_rotation16(rotation: i32, flip: Flip) -> i32 {
    let j = if rotation > 8 { rotation - 16 } else { rotation };
    match flip {
        Flip::FrontBack => (16 - j).rem_euclid(16),
        Flip::LeftRight => (8 - j).rem_euclid(16),
    }
}

/// `BlockState.mirror(mirror)`.
pub fn mirror(props: &mut Props, flip: Flip) {
    if let Some(v) = props.get_mut("rotation")
        && let Ok(n) = v.parse::<i32>()
    {
        *v = mirror_rotation16(n, flip).to_string();
        return;
    }
    if is_rail(props) {
        let shape = props["shape"].clone();
        let out = match flip {
            Flip::LeftRight => match shape.as_str() {
                "ascending_north" | "ascending_south" => swapped(&shape, "ascending_north", "ascending_south"),
                "south_east" | "north_east" => swapped(&shape, "south_east", "north_east"),
                "south_west" | "north_west" => swapped(&shape, "south_west", "north_west"),
                _ => shape,
            },
            Flip::FrontBack => match shape.as_str() {
                "ascending_east" | "ascending_west" => swapped(&shape, "ascending_east", "ascending_west"),
                "south_east" | "south_west" => swapped(&shape, "south_east", "south_west"),
                "north_west" | "north_east" => swapped(&shape, "north_west", "north_east"),
                _ => shape,
            },
        };
        props.insert("shape".into(), out);
        return;
    }
    if is_stairs(props) {
        let facing = props["facing"].clone();
        let turn = mirror_rotation(flip, &facing);
        // The corner changes hands under any mirror; the stair turns round only across it.
        let shape = match props["shape"].as_str() {
            "inner_left" => "inner_right",
            "inner_right" => "inner_left",
            "outer_left" => "outer_right",
            "outer_right" => "outer_left",
            other => other,
        }
        .to_owned();
        props.insert("facing".into(), rotate_dir(&facing, turn));
        props.insert("shape".into(), shape);
        return;
    }
    if let Some(facing) = props.get("facing").cloned() {
        // `state.rotate(mirror.getRotation(facing))`; a six-way facing (`DirectionalBlock.mirror`) turns the same.
        props.insert("facing".into(), rotate_dir(&facing, mirror_rotation(flip, &facing)));
        // `DoorBlock.mirror` cycles the hinge.
        if let Some(h) = props.get_mut("hinge") {
            *h = swapped(h, "left", "right");
        }
        // A double chest's halves swap sides (`ChestBlock.mirror` through `ChestType.getOpposite`).
        if let Some(t) = props.get_mut("type") {
            *t = swapped(t, "left", "right");
        }
    }
    if let Some(o) = props.get_mut("orientation")
        && let Some((front, top)) = o.split_once('_')
    {
        *o = format!("{}_{}", mirror_dir(flip, front), mirror_dir(flip, top));
    }
    if has_sides(props) {
        let (a, b) = match flip {
            Flip::LeftRight => ("north", "south"),
            Flip::FrontBack => ("east", "west"),
        };
        let (va, vb) = (props[a].clone(), props[b].clone());
        props.insert(a.to_owned(), vb);
        props.insert(b.to_owned(), va);
    }
}

/// One of the mirror's turns of a state (`SymmetryMirror.flipX`, `flipZ`, `flipD1`, `flipD2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// `flipX(state)`: `mirror(FRONT_BACK)`.
    X,
    /// `flipZ(state)`: `mirror(LEFT_RIGHT)`.
    Z,
    /// `flipD1(state)`: `rotate(COUNTERCLOCKWISE_90).mirror(FRONT_BACK)`.
    D1,
    /// `flipD2(state)`: `rotate(COUNTERCLOCKWISE_90).mirror(LEFT_RIGHT)`.
    D2,
}

impl Turn {
    pub fn apply(self, props: &mut Props) {
        match self {
            Turn::X => mirror(props, Flip::FrontBack),
            Turn::Z => mirror(props, Flip::LeftRight),
            Turn::D1 => {
                rotate(props, COUNTERCLOCKWISE_90);
                mirror(props, Flip::FrontBack);
            }
            Turn::D2 => {
                rotate(props, COUNTERCLOCKWISE_90);
                mirror(props, Flip::LeftRight);
            }
        }
    }
}

/// A state's text with `turns` applied in order.
pub fn turned(text: &str, turns: &[Turn]) -> String {
    let (name, mut props) = parse(text);
    for turn in turns {
        turn.apply(&mut props);
    }
    format(&name, &props)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_round_trip() {
        let text = "minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]";
        let (name, props) = parse(text);
        assert_eq!(name, "minecraft:oak_stairs");
        assert_eq!(props["facing"], "east");
        assert_eq!(format(&name, &props), text);
        assert_eq!(parse("minecraft:stone"), ("minecraft:stone".to_owned(), Props::new()));
    }

    #[test]
    fn a_facing_turns_round_only_across_the_mirror() {
        let (_, mut p) = parse("minecraft:furnace[facing=east,lit=false]");
        mirror(&mut p, Flip::FrontBack);
        assert_eq!(p["facing"], "west");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!(p["facing"], "west");
        let (_, mut p) = parse("minecraft:dispenser[facing=up,triggered=false]");
        mirror(&mut p, Flip::FrontBack);
        assert_eq!(p["facing"], "up");
    }

    #[test]
    fn quarter_turns_follow_the_clock() {
        let (_, mut p) = parse("minecraft:furnace[facing=north]");
        rotate(&mut p, 1);
        assert_eq!(p["facing"], "east");
        rotate(&mut p, COUNTERCLOCKWISE_90);
        assert_eq!(p["facing"], "north");
    }

    #[test]
    fn a_pillar_swaps_its_axis_on_a_quarter_turn_only() {
        let (_, mut p) = parse("minecraft:oak_log[axis=x]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!(p["axis"], "x");
        rotate(&mut p, 2);
        assert_eq!(p["axis"], "x");
        rotate(&mut p, 1);
        assert_eq!(p["axis"], "z");
        let (_, mut p) = parse("minecraft:oak_log[axis=y]");
        rotate(&mut p, 1);
        assert_eq!(p["axis"], "y");
    }

    #[test]
    fn the_diagonal_turns_swap_axes() {
        // `flipD1` = counterclockwise quarter turn then a front-back mirror: a log along x ends along z.
        assert_eq!(turned("minecraft:oak_log[axis=x]", &[Turn::D1]), "minecraft:oak_log[axis=z]");
        // A facing: north -> (ccw) west -> (front-back) east.
        assert_eq!(turned("minecraft:furnace[facing=north]", &[Turn::D1]), "minecraft:furnace[facing=east]");
        // north -> west -> (left-right leaves west) west.
        assert_eq!(turned("minecraft:furnace[facing=north]", &[Turn::D2]), "minecraft:furnace[facing=west]");
        // Create's `axis_along_first` flips on a quarter turn.
        assert_eq!(
            turned("create:gearbox[axis_along_first=true]", &[Turn::D1]),
            "create:gearbox[axis_along_first=false]"
        );
    }

    #[test]
    fn stairs_turn_round_across_the_mirror_and_swap_corners() {
        let (_, mut p) = parse("minecraft:oak_stairs[facing=north,half=bottom,shape=inner_left]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!((p["facing"].as_str(), p["shape"].as_str()), ("south", "inner_right"));
        let (_, mut p) = parse("minecraft:oak_stairs[facing=east,half=top,shape=outer_right]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!((p["facing"].as_str(), p["shape"].as_str()), ("east", "outer_left"));
        let (_, mut p) = parse("minecraft:oak_stairs[facing=east,half=top,shape=straight]");
        mirror(&mut p, Flip::FrontBack);
        assert_eq!((p["facing"].as_str(), p["shape"].as_str(), p["half"].as_str()), ("west", "straight", "top"));
    }

    #[test]
    fn doors_swap_hinge_and_chests_swap_type() {
        let (_, mut p) = parse("minecraft:oak_door[facing=north,half=lower,hinge=left,open=false]");
        mirror(&mut p, Flip::FrontBack);
        assert_eq!((p["facing"].as_str(), p["hinge"].as_str()), ("north", "right"));
        let (_, mut p) = parse("minecraft:chest[facing=north,type=left]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!((p["facing"].as_str(), p["type"].as_str()), ("south", "right"));
        // A slab's `type` is no chest's.
        let (_, mut p) = parse("minecraft:stone_slab[type=bottom]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!(p["type"], "bottom");
    }

    #[test]
    fn sides_swap_across_the_mirror_and_cycle_on_a_turn() {
        let (_, mut p) = parse("minecraft:oak_fence[east=true,north=false,south=false,waterlogged=false,west=false]");
        mirror(&mut p, Flip::FrontBack);
        assert_eq!((p["east"].as_str(), p["west"].as_str()), ("false", "true"));
        let (_, mut p) = parse("minecraft:oak_fence[east=true,north=false,south=false,west=false]");
        rotate(&mut p, 1);
        assert_eq!((p["south"].as_str(), p["east"].as_str()), ("true", "false"));
        let (_, mut p) = parse("minecraft:oak_fence[east=true,north=false,south=false,west=false]");
        mirror(&mut p, Flip::LeftRight);
        assert_eq!(p["east"], "true");
    }

    #[test]
    fn sixteen_step_rotations_mirror() {
        // South is 0, west 4, north 8, east 12.
        let (_, mut p) = parse("minecraft:oak_sign[rotation=4]");
        mirror(&mut p, Flip::FrontBack); // x -> -x: west to east
        assert_eq!(p["rotation"], "12");
        let (_, mut p) = parse("minecraft:oak_sign[rotation=0]");
        mirror(&mut p, Flip::LeftRight); // z -> -z: south to north
        assert_eq!(p["rotation"], "8");
        let (_, mut p) = parse("minecraft:oak_sign[rotation=4]");
        rotate(&mut p, 1);
        assert_eq!(p["rotation"], "8");
    }

    #[test]
    fn rails_mirror_and_turn() {
        assert_eq!(turned("minecraft:rail[shape=ascending_east]", &[Turn::X]), "minecraft:rail[shape=ascending_west]");
        assert_eq!(turned("minecraft:rail[shape=south_east]", &[Turn::Z]), "minecraft:rail[shape=north_east]");
        assert_eq!(turned("minecraft:rail[shape=north_south]", &[Turn::D1]), "minecraft:rail[shape=east_west]");
    }

    #[test]
    fn the_crafters_orientation_turns() {
        assert_eq!(turned("minecraft:crafter[orientation=east_up]", &[Turn::X]), "minecraft:crafter[orientation=west_up]");
        assert_eq!(
            turned("minecraft:crafter[orientation=north_up]", &[Turn::D1]),
            "minecraft:crafter[orientation=east_up]"
        );
    }

    #[test]
    fn two_turns_apply_in_order() {
        // `flipX(flipZ(block))`: the z flip first.
        assert_eq!(turned("minecraft:furnace[facing=north]", &[Turn::Z, Turn::X]), "minecraft:furnace[facing=south]");
        assert_eq!(turned("minecraft:stone", &[Turn::Z, Turn::X]), "minecraft:stone");
    }
}

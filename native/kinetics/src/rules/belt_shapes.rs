//! The outline and collision shape of every belt block state (Belts III).
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT): `BeltShapes` (`getShape`,
//! `getCollisionShape`, `getBeltShape`, `getCasingShape`, the slope's sixteen slices) and
//! `AllShapes.BELT_COLLISION_MASK`.
//!
//! Create builds each shape for a belt facing south and turns it to the other facings
//! (`VoxelShaper`), a vertical one by standing the flat one on its end. A shape here is a set of
//! one-pixel voxels, so unions, masks and turns are exact, then cut back into at most 16 boxes
//! (Horizon's limit on a block's shape table). Pixels, 16 to a block, with y counted from the block's
//! base and the slope reaching 5 below and above it, as Create's does.
//!
//! `tools/content/belts_iii.py` reads the table this writes to `tools/content/_belt_shapes_gen.json`
//! (rewrite it with `BELT_SHAPES_WRITE=1 cargo test belt_shapes`; a test checks it is current).

use std::collections::BTreeSet;

use crate::belt::{Part, Slope};
use crate::Dir;

/// A set of voxels `(x, y, z)`, each one pixel: x and z 0 to 15, y -5 to 20.
type Voxels = BTreeSet<(i32, i32, i32)>;
/// A box in pixels: `[x0, y0, z0, x1, y1, z1]`.
pub type Box16 = [i32; 6];

/// The most boxes a shape table entry may have (`horizon-mods` `content.rs`).
pub const MAX_BOXES: usize = 16;
/// A cut with this many boxes or fewer is taken without trying the other orders.
const GOOD_ENOUGH: usize = 11;
/// `AllShapes.BELT_COLLISION_MASK`: collision stops at 19 pixels.
const COLLISION_TOP: i32 = 19;
/// The belt's shape is cut at 21 pixels (the top of the slope's last slice) for its outline.
const OUTLINE_TOP: i32 = 21;

fn boxed(b: Box16) -> Voxels {
    let mut v = Voxels::new();
    for x in b[0]..b[3] {
        for y in b[1]..b[4] {
            for z in b[2]..b[5] {
                v.insert((x, y, z));
            }
        }
    }
    v
}

fn or(mut a: Voxels, b: Voxels) -> Voxels {
    a.extend(b);
    a
}

/// `compose(southPart, northPart)`: the south half of one and the north half of the other.
fn compose(south: &Voxels, north: &Voxels) -> Voxels {
    south
        .iter()
        .filter(|v| v.2 >= 8)
        .chain(north.iter().filter(|v| v.2 < 8))
        .copied()
        .collect()
}

/// `makeSlopePart`: sixteen slices, each a pixel along z and one higher than the last.
fn slope_part(ascending: bool) -> Voxels {
    let mut result = Voxels::new();
    for i in 0..16 {
        let y_offset = if ascending { 10 - i } else { i - 5 };
        result.extend(boxed([1, y_offset, 15 - i, 15, y_offset + 11, 16 - i]));
    }
    result
}

fn flat_full() -> Voxels {
    boxed([1, 3, 0, 15, 13, 16])
}

fn flat_end() -> Voxels {
    or(boxed([1, 4, 0, 15, 12, 16]), boxed([1, 3, 1, 15, 13, 15]))
}

fn sideways_full() -> Voxels {
    boxed([3, 1, 0, 13, 15, 16])
}

fn sideways_end() -> Voxels {
    or(boxed([4, 1, 0, 12, 15, 16]), boxed([3, 1, 1, 13, 15, 15]))
}

/// A shape made for a belt facing south, turned about y to face `facing` (`VoxelShaper.forHorizontal`).
fn turned(shape: &Voxels, facing: Dir) -> Voxels {
    shape
        .iter()
        .map(|&(x, y, z)| match facing {
            Dir::West => (15 - z, y, x),
            Dir::North => (15 - x, y, 15 - z),
            Dir::East => (z, y, 15 - x),
            _ => (x, y, z),
        })
        .collect()
}

/// A flat shape made for south stood on its end (`VerticalBeltShaper`): its south half up for a
/// facing in the positive direction, down for the negative (turned half over), across the axis
/// the facing lies on.
fn stood_up(shape: &Voxels, facing: Dir) -> Voxels {
    let up: Voxels = shape.iter().map(|&(x, y, z)| (x, z, 15 - y)).collect();
    let up = if facing.sign() > 0 {
        up
    } else {
        up.iter().map(|&(x, y, z)| (15 - x, 15 - y, z)).collect()
    };
    // The width is along x for a belt facing along z and along z for one facing along x.
    match facing {
        Dir::East | Dir::West => turned(&up, Dir::East),
        _ => up,
    }
}

/// Create's `PARTIAL_CASING`: `box(0, 0, 5, 16, 11, 16)` for south.
fn partial_casing(facing: Dir) -> Voxels {
    turned(&boxed([0, 0, 5, 16, 11, 16]), facing)
}

fn casing_11() -> Voxels {
    boxed([0, 0, 0, 16, 11, 16])
}

fn belt_shape(facing: Dir, slope: Slope, part: Part) -> Voxels {
    let middle = matches!(part, Part::Middle | Part::Pulley);
    let start = part == Part::Start;
    match slope {
        Slope::Vertical => {
            if middle {
                return stood_up(&flat_full(), facing);
            }
            let (south, north) = if start {
                (flat_full(), flat_end())
            } else {
                (flat_end(), flat_full())
            };
            stood_up(&compose(&south, &north), facing)
        }
        Slope::Horizontal | Slope::Sideways => {
            let (full, end) = if slope == Slope::Horizontal {
                (flat_full(), flat_end())
            } else {
                (sideways_full(), sideways_end())
            };
            if middle {
                return turned(&full, facing);
            }
            // The start's closed end is behind it, at the north when facing south.
            turned(&if start { compose(&full, &end) } else { compose(&end, &full) }, facing)
        }
        Slope::Upward | Slope::Downward => {
            let part_shape = slope_part(slope == Slope::Upward);
            if middle {
                return turned(&part_shape, facing);
            }
            let end = flat_end();
            turned(
                &if start { compose(&part_shape, &end) } else { compose(&end, &part_shape) },
                facing,
            )
        }
    }
}

fn casing_shape(facing: Dir, slope: Slope, part: Part) -> Voxels {
    match slope {
        Slope::Vertical | Slope::Sideways => Voxels::new(),
        Slope::Horizontal => casing_11(),
        Slope::Upward | Slope::Downward => {
            let upward = slope == Slope::Upward;
            match part {
                Part::Middle | Part::Pulley => {
                    partial_casing(if upward { facing } else { facing.opposite() })
                }
                Part::Start => {
                    if upward {
                        casing_11()
                    } else {
                        partial_casing(facing.opposite())
                    }
                }
                Part::End => {
                    if !upward {
                        casing_11()
                    } else {
                        partial_casing(facing)
                    }
                }
            }
        }
    }
}

/// `BeltShapes.getShape`: the outline, cut to between the block's base and 21 pixels.
fn outline_voxels(facing: Dir, slope: Slope, part: Part, casing: bool) -> Voxels {
    let mut shape = belt_shape(facing, slope, part);
    if casing {
        shape.extend(casing_shape(facing, slope, part));
    }
    shape.retain(|v| v.1 >= 0 && v.1 < OUTLINE_TOP);
    shape
}

/// `BeltShapes.getCollisionShape`: the shape under the 19 pixel mask.
fn collision_voxels(facing: Dir, slope: Slope, part: Part, casing: bool) -> Voxels {
    let mut shape = outline_voxels(facing, slope, part, casing);
    shape.retain(|v| v.1 < COLLISION_TOP);
    shape
}

/// The grid a voxel set is cut on: x 0 to 15, y -5 to 20 (shifted by 5), z 0 to 15.
const GRID: [usize; 3] = [16, 26, 16];

type Grid = [[[bool; 16]; 26]; 16];

fn grid_of(voxels: &Voxels) -> Grid {
    let mut grid = [[[false; 16]; 26]; 16];
    for &(x, y, z) in voxels {
        grid[x as usize][(y + 5) as usize][z as usize] = true;
    }
    grid
}

/// One try at cutting the grid into boxes: the origin of each box is the first voxel left when
/// they are ordered by the axes of `origin_order` (most significant first), and it is extended along
/// the axes of `extend_order` in turn as far as the voxels stay filled.
fn cut(voxels: &Voxels, origin_order: [usize; 3], extend_order: [usize; 3]) -> Vec<Box16> {
    let mut left = grid_of(voxels);
    let mut cells: Vec<[usize; 3]> = voxels.iter().map(|v| [v.0 as usize, (v.1 + 5) as usize, v.2 as usize]).collect();
    cells.sort_by_key(|c| origin_order.map(|axis| c[axis]));
    let mut boxes = Vec::new();
    for origin in cells {
        if !left[origin[0]][origin[1]][origin[2]] {
            continue;
        }
        let mut size = [1usize; 3];
        for &axis in &extend_order {
            while origin[axis] + size[axis] < GRID[axis] {
                let layer = origin[axis] + size[axis];
                let (u, v) = match axis {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let full = (0..size[u]).all(|a| {
                    (0..size[v]).all(|b| {
                        let mut at = origin;
                        at[axis] = layer;
                        at[u] += a;
                        at[v] += b;
                        left[at[0]][at[1]][at[2]]
                    })
                });
                if !full {
                    break;
                }
                size[axis] += 1;
            }
        }
        for a in 0..size[0] {
            for b in 0..size[1] {
                for c in 0..size[2] {
                    left[origin[0] + a][origin[1] + b][origin[2] + c] = false;
                }
            }
        }
        boxes.push([
            origin[0] as i32,
            origin[1] as i32 - 5,
            origin[2] as i32,
            (origin[0] + size[0]) as i32,
            (origin[1] + size[1]) as i32 - 5,
            (origin[2] + size[2]) as i32,
        ]);
    }
    boxes
}

/// Cuts a voxel set into boxes, with the fewest of the tries over every order of axes.
fn boxes_of(voxels: &Voxels) -> Vec<Box16> {
    let orders: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let mut best: Option<Vec<Box16>> = None;
    for origin in orders {
        for extend in orders {
            let boxes = cut(voxels, origin, extend);
            if best.as_ref().is_none_or(|b| boxes.len() < b.len()) {
                best = Some(boxes);
            }
            // Good enough: a state's collision is tested against every box.
            if best.as_ref().is_some_and(|b| b.len() <= GOOD_ENOUGH) {
                return best.unwrap_or_default();
            }
        }
    }
    best.unwrap_or_default()
}

type Table = std::collections::BTreeMap<String, (Vec<Box16>, Vec<Box16>)>;

/// Every state's outline and collision, worked out once.
fn table() -> &'static Table {
    static TABLE: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Table::new();
        for facing in FACINGS {
            for slope in Slope::ALL {
                for part in Part::ALL {
                    for casing in [false, true] {
                        table.insert(
                            key(facing, slope, part, casing),
                            (
                                boxes_of(&outline_voxels(facing, slope, part, casing)),
                                boxes_of(&collision_voxels(facing, slope, part, casing)),
                            ),
                        );
                    }
                }
            }
        }
        table
    })
}

/// The outline of the belt block, as boxes in pixels.
pub fn outline(facing: Dir, slope: Slope, part: Part, casing: bool) -> Vec<Box16> {
    table()[&key(facing, slope, part, casing)].0.clone()
}

/// The collision of the belt block, as boxes in pixels.
pub fn collision(facing: Dir, slope: Slope, part: Part, casing: bool) -> Vec<Box16> {
    table()[&key(facing, slope, part, casing)].1.clone()
}

/// The key of a state in a shape table whose properties are `facing, slope, part, casing`.
pub fn key(facing: Dir, slope: Slope, part: Part, casing: bool) -> String {
    format!("{},{},{},{casing}", facing.name(), slope.name(), part.name())
}

pub const FACINGS: [Dir; 4] = [Dir::North, Dir::South, Dir::East, Dir::West];

/// The whole table as the JSON `tools/content/belts_iii.py` reads: for each state key, the
/// outline and collision boxes in pixels.
pub fn table_json() -> String {
    let mut entries = Vec::new();
    for facing in FACINGS {
        for slope in Slope::ALL {
            for part in Part::ALL {
                for casing in [false, true] {
                    let show = |boxes: Vec<Box16>| {
                        let inner: Vec<String> = boxes
                            .iter()
                            .map(|b| format!("[{},{},{},{},{},{}]", b[0], b[1], b[2], b[3], b[4], b[5]))
                            .collect();
                        format!("[{}]", inner.join(","))
                    };
                    entries.push(format!(
                        "  \"{}\": {{\"outline\": {}, \"collision\": {}}}",
                        key(facing, slope, part, casing),
                        show(outline(facing, slope, part, casing)),
                        show(collision(facing, slope, part, casing)),
                    ));
                }
            }
        }
    }
    format!("{{\n{}\n}}\n", entries.join(",\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(boxes: &[Box16]) -> i32 {
        boxes.iter().map(|b| b[4]).max().unwrap_or(0)
    }

    #[test]
    fn a_flat_middle_block_is_the_full_box_along_the_belt() {
        // `FLAT_FULL_PART`: box(1, 3, 0, 15, 13, 16) for a belt along z, turned for x.
        assert_eq!(outline(Dir::South, Slope::Horizontal, Part::Middle, false), vec![[1, 3, 0, 15, 13, 16]]);
        assert_eq!(outline(Dir::North, Slope::Horizontal, Part::Pulley, false), vec![[1, 3, 0, 15, 13, 16]]);
        assert_eq!(outline(Dir::East, Slope::Horizontal, Part::Middle, false), vec![[0, 3, 1, 16, 13, 15]]);
        assert_eq!(collision(Dir::West, Slope::Horizontal, Part::Middle, false), vec![[0, 3, 1, 16, 13, 15]]);
    }

    #[test]
    fn a_flat_end_is_rounded_at_its_closed_end_only() {
        // Facing south, the start's closed end is the north half: its rounded edge is at z 0.
        let start = outline(Dir::South, Slope::Horizontal, Part::Start, false);
        let covers = |boxes: &[Box16], x: i32, y: i32, z: i32| {
            boxes.iter().any(|b| b[0] <= x && x < b[3] && b[1] <= y && y < b[4] && b[2] <= z && z < b[5])
        };
        // The corner pixel of the rounded end is missing, the south half is full.
        assert!(!covers(&start, 1, 3, 0));
        assert!(covers(&start, 1, 4, 0));
        assert!(covers(&start, 1, 3, 12));
        // The end block is the other way round, and a belt facing north swaps them.
        let end = outline(Dir::South, Slope::Horizontal, Part::End, false);
        assert!(covers(&end, 1, 3, 3) && !covers(&end, 1, 3, 15));
        let north = outline(Dir::North, Slope::Horizontal, Part::Start, false);
        assert!(!covers(&north, 1, 3, 15) && covers(&north, 1, 3, 3));
        // Facing east, the start is at the west.
        let east = outline(Dir::East, Slope::Horizontal, Part::Start, false);
        assert!(!covers(&east, 0, 3, 1) && covers(&east, 12, 3, 1));
    }

    #[test]
    fn a_sloped_block_climbs_a_pixel_for_each_pixel_along() {
        // Facing south and descending: it is lowest at the south edge (z 15) and one pixel higher
        // for each pixel north: the slice at z 15 - i reaches i + 6.
        let down = collision(Dir::South, Slope::Downward, Part::Middle, false);
        let height_at = |boxes: &[Box16], z: i32| {
            boxes.iter().filter(|b| b[2] <= z && z < b[5]).map(|b| b[4]).max().unwrap_or(0)
        };
        for i in 0..13 {
            assert_eq!(height_at(&down, 15 - i), i + 6, "slice {i}");
        }
        // The mask stops collision at 19 pixels, the outline goes on to 21.
        assert_eq!(top(&down), 19);
        assert_eq!(top(&outline(Dir::South, Slope::Downward, Part::Middle, false)), 21);
        // Ascending is the same the other way along z.
        let up = collision(Dir::South, Slope::Upward, Part::Middle, false);
        for z in 0..14 {
            assert_eq!(height_at(&up, z), z + 6, "z {z}");
        }
        // Facing north mirrors it.
        let north = collision(Dir::North, Slope::Downward, Part::Middle, false);
        assert_eq!(height_at(&north, 0), 6);
        assert_eq!(height_at(&north, 12), 18);
    }

    #[test]
    fn a_sloped_end_is_flat_on_the_end_half() {
        // Descending south, the end block's south half is the flat end (the belt ends there).
        let end = collision(Dir::South, Slope::Downward, Part::End, false);
        let height_at = |z: i32| end.iter().filter(|b| b[2] <= z && z < b[5]).map(|b| b[4]).max().unwrap_or(0);
        // The flat end is 12 pixels thick at its very edge and 13 inside it; the sloped half climbs.
        assert_eq!(height_at(12), 13);
        assert_eq!(height_at(15), 12);
        assert_eq!(height_at(3), 18);
    }

    #[test]
    fn a_casing_fills_the_block_under_the_belt() {
        // A flat cased belt block is solid to 11 pixels under the belt.
        let boxes = outline(Dir::South, Slope::Horizontal, Part::Middle, true);
        assert!(boxes.iter().any(|b| b[0] == 0 && b[3] == 16 && b[1] == 0 && b[4] >= 11));
        assert_eq!(top(&boxes), 13);
        // Vertical and sideways belts have no casing shape of their own.
        assert_eq!(
            outline(Dir::South, Slope::Vertical, Part::Middle, true),
            outline(Dir::South, Slope::Vertical, Part::Middle, false)
        );
        assert_eq!(
            outline(Dir::East, Slope::Sideways, Part::End, true),
            outline(Dir::East, Slope::Sideways, Part::End, false)
        );
    }

    #[test]
    fn a_vertical_belt_stands_on_its_end_with_its_start_at_the_bottom() {
        let covers = |boxes: &[Box16], x: i32, y: i32, z: i32| {
            boxes.iter().any(|b| b[0] <= x && x < b[3] && b[1] <= y && y < b[4] && b[2] <= z && z < b[5])
        };
        // Facing south (a positive way): the belt is 10 pixels thick in z, 14 wide in x.
        let middle = outline(Dir::South, Slope::Vertical, Part::Middle, false);
        assert!(covers(&middle, 8, 8, 8) && covers(&middle, 1, 0, 3) && covers(&middle, 14, 15, 12));
        assert!(!covers(&middle, 0, 8, 8) && !covers(&middle, 8, 8, 2) && !covers(&middle, 8, 8, 13));
        // Across x for a belt facing east.
        let east = outline(Dir::East, Slope::Vertical, Part::Middle, false);
        assert!(covers(&east, 8, 8, 1) && !covers(&east, 2, 8, 8) && covers(&east, 3, 8, 8));
        // The start's rounded end is at the bottom for a positive facing, at the top for a negative.
        let start = outline(Dir::South, Slope::Vertical, Part::Start, false);
        assert!(!covers(&start, 1, 0, 3) && covers(&start, 1, 15, 3));
        let start = outline(Dir::North, Slope::Vertical, Part::Start, false);
        assert!(!covers(&start, 1, 15, 3) && covers(&start, 1, 0, 3));
    }

    #[test]
    fn every_state_fits_in_a_shape_table() {
        for facing in FACINGS {
            for slope in Slope::ALL {
                for part in Part::ALL {
                    for casing in [false, true] {
                        for boxes in [outline(facing, slope, part, casing), collision(facing, slope, part, casing)] {
                            assert!(!boxes.is_empty(), "{facing:?} {slope:?} {part:?}");
                            assert!(
                                boxes.len() <= MAX_BOXES,
                                "{facing:?} {slope:?} {part:?} {casing}: {} boxes",
                                boxes.len()
                            );
                            assert!(boxes.iter().all(|b| b[0] >= 0 && b[3] <= 16 && b[2] >= 0 && b[5] <= 16 && b[1] >= 0));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_checked_in_table_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/content/_belt_shapes_gen.json");
        let current = table_json();
        // `tools/check_module.sh` tests a copy of `native/` with no `tools/` beside it.
        if !path.parent().is_some_and(|dir| dir.exists()) {
            return;
        }
        if std::env::var_os("BELT_SHAPES_WRITE").is_some() {
            std::fs::write(&path, &current).unwrap();
        }
        let saved = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            saved == current,
            "tools/content/_belt_shapes_gen.json is stale: BELT_SHAPES_WRITE=1 cargo test belt_shapes"
        );
    }
}

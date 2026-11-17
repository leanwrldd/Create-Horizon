//! The Handheld Worldshaper's rules (`WorldshaperItem`, `TerrainBrushes`, `CuboidBrush`,
//! `SphereBrush`, `CylinderBrush`, `DynamicBrush`, `TerrainTools`, `PlacementOptions`): which
//! blocks a brush covers and what each tool does to them. No game types: the game glue is
//! `machines/worldshaper.rs`.
//!
//! Known differences from Create, for the README:
//! - No screen. The brush, tool, size and placement are cycled with sneaking right clicks (the
//!   glue lists them) and the block to use is picked by left-clicking it.
//! - The size is one number for every dimension of the brush (a cuboid is n x n x n, a cylinder has
//!   radius and height n); Create's screen sets width, height and length apart. The Surface and
//!   Cluster brushes keep Create's defaults for diagonals and fuzziness (both on).
//! - The Flatten tool (`FlattenTool`) is left out: it copies whole block states up and down a
//!   smoothed height map, and Horizon gives a mod no way to read a block's full state back.
//! - No placement patterns (`PlacementPatterns`: checkered, random, ...), and the block to use is
//!   its default state (no facing, no block entity data).

use crate::Dir;

pub type Pos = [i32; 3];

/// `TerrainBrushes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    Cuboid,
    Sphere,
    Cylinder,
    Surface,
    Cluster,
}

impl Brush {
    pub const ALL: [Brush; 5] = [Self::Cuboid, Self::Sphere, Self::Cylinder, Self::Surface, Self::Cluster];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cuboid => "cuboid",
            Self::Sphere => "sphere",
            Self::Cylinder => "cylinder",
            Self::Surface => "surface",
            Self::Cluster => "cluster",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }

    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|b| *b == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// `Brush.getMax` and `getMin` of the size.
    pub fn size_range(self) -> (i32, i32) {
        match self {
            Self::Cuboid => (1, 32),
            Self::Sphere => (0, 10),
            Self::Cylinder => (1, 8),
            Self::Surface | Self::Cluster => (1, 10),
        }
    }

    /// `Brush.getSupportedTools`.
    pub fn tools(self) -> &'static [Tool] {
        match self {
            Self::Surface => &[Tool::Overlay, Tool::Replace, Tool::Clear],
            Self::Cluster => &[Tool::Replace, Tool::Clear],
            _ => &[Tool::Fill, Tool::Place, Tool::Replace, Tool::Clear, Tool::Overlay],
        }
    }

    /// `Brush.hasPlacementOptions`.
    pub fn has_placement_options(self) -> bool {
        !matches!(self, Self::Surface | Self::Cluster)
    }

    /// `DynamicBrush.redirectTool`: the Surface brush's overlay is a place.
    pub fn redirect(self, tool: Tool) -> Tool {
        if self == Self::Surface && tool == Tool::Overlay { Tool::Place } else { tool }
    }

    /// The brush's three parameters for a size (`Brush.set`): the cuboid's width, height and
    /// length; the sphere's radius; the cylinder's radius and height; the dynamic brushes' range
    /// then 0, 0 (diagonals and fuzzy on).
    pub fn params(self, size: i32) -> [i32; 3] {
        let (min, max) = self.size_range();
        let n = size.clamp(min, max);
        match self {
            Self::Cuboid => [n, n, n],
            Self::Sphere => [n, 0, 0],
            Self::Cylinder => [n, n, 0],
            Self::Surface | Self::Cluster => [n, 0, 0],
        }
    }
}

/// `TerrainTools` (without Flatten).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Fill,
    Place,
    Replace,
    Clear,
    Overlay,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Place => "place",
            Self::Replace => "replace",
            Self::Clear => "clear",
            Self::Overlay => "overlay",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Fill, Self::Place, Self::Replace, Self::Clear, Self::Overlay]
            .into_iter()
            .find(|t| t.name() == name)
    }

    /// `TerrainTools.requiresSelectedBlock`.
    pub fn requires_block(self) -> bool {
        self != Self::Clear
    }
}

/// The next tool the brush supports after `tool`.
pub fn next_tool(brush: Brush, tool: Tool) -> Tool {
    let tools = brush.tools();
    match tools.iter().position(|t| *t == tool) {
        Some(at) => tools[(at + 1) % tools.len()],
        None => tools[0],
    }
}

/// `PlacementOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Merged,
    Attached,
    Inserted,
}

impl Placement {
    pub fn name(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Attached => "attached",
            Self::Inserted => "inserted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Merged, Self::Attached, Self::Inserted].into_iter().find(|p| p.name() == name)
    }

    pub fn next(self) -> Self {
        match self {
            Self::Merged => Self::Attached,
            Self::Attached => Self::Inserted,
            Self::Inserted => Self::Merged,
        }
    }
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn relative(face: Dir, n: i32) -> Pos {
    let [x, y, z] = face.normal();
    [x * n, y * n, z * n]
}

/// The blocks a shaped brush covers, relative to its centre (`ShapedBrush.getIncludedPositions`).
pub fn shape(brush: Brush, params: [i32; 3]) -> Vec<Pos> {
    let [p0, p1, p2] = params;
    match brush {
        Brush::Cuboid => {
            // `betweenClosed(((p - 1) / -2), (p / 2))`: Java's division truncates, as Rust's does.
            let lo = [(p0 - 1) / -2, (p1 - 1) / -2, (p2 - 1) / -2];
            let hi = [p0 / 2, p1 / 2, p2 / 2];
            let mut out = Vec::new();
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        out.push([x, y, z]);
                    }
                }
            }
            out
        }
        Brush::Sphere => {
            let r = p0.clamp(0, 10);
            let mut out = Vec::new();
            for x in -r - 1..=r + 1 {
                for y in -r - 1..=r + 1 {
                    for z in -r - 1..=r + 1 {
                        let d = f64::from(x * x + y * y + z * z).sqrt();
                        if d < f64::from(r) + 0.5 {
                            out.push([x, y, z]);
                        }
                    }
                }
            }
            out
        }
        Brush::Cylinder => {
            let (r, h) = (p0.clamp(0, 8), p1.clamp(0, 8));
            let mut flat = Vec::new();
            for x in -r - 1..=r + 1 {
                for z in -r - 1..=r + 1 {
                    if f64::from(x * x + z * z).sqrt() < f64::from(r) + 0.42 {
                        flat.push([x, 0, z]);
                    }
                }
            }
            let mut out = Vec::new();
            for layer in 0..h {
                let y = layer - h / 2;
                out.extend(flat.iter().map(|p| [p[0], p[1] + y, p[2]]));
            }
            out
        }
        Brush::Surface | Brush::Cluster => Vec::new(),
    }
}

/// `Brush.getOffset`: where the brush's centre sits from the block hit, for the face hit.
pub fn offset(brush: Brush, params: [i32; 3], face: Dir, placement: Placement) -> Pos {
    if placement == Placement::Merged {
        return [0; 3];
    }
    let attached = placement == Placement::Attached;
    let sign = if attached { 1 } else { -1 };
    let negative = face.sign() < 0;
    match brush {
        Brush::Cuboid => {
            let offset = if attached { if negative { 2 } else { 1 } } else { 0 };
            let part = |p: i32| (p + if p == 0 { 0 } else { offset }) / 2;
            let along = match face.axis() {
                crate::Axis::X => part(params[0]),
                crate::Axis::Y => part(params[1]),
                crate::Axis::Z => part(params[2]),
            };
            relative(face, along * sign)
        }
        Brush::Sphere => {
            let r = params[0] + 1 + if attached { 0 } else { -1 };
            relative(face, r * sign)
        }
        Brush::Cylinder => {
            let offset = if attached { 0 } else { -1 };
            let y_offset = if attached {
                if negative { 1 } else { 2 }
            } else if negative {
                0
            } else {
                -1
            };
            let r = params[0] + 1 + offset;
            let y = (params[1] + if params[1] == 0 { 0 } else { y_offset }) / 2;
            relative(face, (if face.axis() == crate::Axis::Y { y } else { r }) * sign)
        }
        Brush::Surface | Brush::Cluster => [0; 3],
    }
}

/// What a brush and tool ask of the world.
pub trait Terrain {
    /// `BlockState.canBeReplaced`.
    fn replaceable(&self, pos: Pos) -> bool;
    /// `getDestroySpeed == -1`: bedrock and the like.
    fn unbreakable(&self, pos: Pos) -> bool;
    /// Whether two positions hold the same block.
    fn same_block(&self, a: Pos, b: Pos) -> bool;
    /// `BlockHelper.hasBlockSolidSide`: the block at `pos` has a solid face on `side`.
    fn solid_side(&self, pos: Pos, side: Dir) -> bool;
}

/// `DynamicBrush.addToGlobalPositions`: a flood fill over like blocks from the one hit, within
/// `range`. `surface` is the Surface brush (the Cluster brush's is not); `params` are its
/// parameters (`[range, diagonals, fuzzy]`, 0 for on).
pub fn dynamic_positions(
    terrain: &dyn Terrain,
    surface: bool,
    params: [i32; 3],
    target: Pos,
    face: Dir,
    tool: Tool,
) -> Vec<Pos> {
    let search_diagonals = params[1] == 0;
    let fuzzy = params[2] == 0;
    let replace = tool != Tool::Overlay;
    let range = f64::from(params[0]);
    let mut offsets = Vec::new();
    for x in -1..=1i32 {
        for y in -1..=1i32 {
            for z in -1..=1i32 {
                let steps = x.abs() + y.abs() + z.abs();
                let along = match face.axis() {
                    crate::Axis::X => x,
                    crate::Axis::Y => y,
                    crate::Axis::Z => z,
                };
                if (steps < 2 || search_diagonals) && (along == 0 || !surface) {
                    offsets.push([x, y, z]);
                }
            }
        }
    }
    let start = if replace { target } else { add(target, relative(face, 1)) };
    let mut visited: Vec<Pos> = Vec::new();
    let mut frontier = std::collections::VecDeque::from([start]);
    let mut affected = Vec::new();
    while let Some(current) = frontier.pop_front() {
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);
        let d = [current[0] - start[0], current[1] - start[1], current[2] - start[2]];
        if f64::from(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) >= range * range {
            continue;
        }
        if replace {
            let above = add(current, relative(face, 1));
            if terrain.unbreakable(current)
                || (!terrain.same_block(current, target) && !fuzzy)
                || terrain.replaceable(current)
                || (terrain.solid_side(above, face.opposite()) && surface)
            {
                continue;
            }
        } else {
            let below = add(current, relative(face.opposite(), 1));
            if terrain.replaceable(below)
                || (!terrain.same_block(below, target) && !fuzzy)
                || !terrain.replaceable(current)
            {
                continue;
            }
        }
        affected.push(current);
        for o in &offsets {
            frontier.push_back(add(current, *o));
        }
    }
    affected
}

/// What a tool does to one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Put the block to use here.
    Set(Pos),
    /// Put air here.
    Air(Pos),
}

/// `TerrainTools.run`: the edits for the positions a brush found.
pub fn run(terrain: &dyn Terrain, tool: Tool, positions: &[Pos], painted_is_same: &dyn Fn(Pos) -> bool) -> Vec<Edit> {
    let mut edits = Vec::new();
    for &p in positions {
        match tool {
            Tool::Clear => edits.push(Edit::Air(p)),
            Tool::Fill => {
                if terrain.replaceable(p) {
                    edits.push(Edit::Set(p));
                }
            }
            Tool::Place => edits.push(Edit::Set(p)),
            Tool::Replace => {
                if !terrain.replaceable(p) {
                    edits.push(Edit::Set(p));
                }
            }
            Tool::Overlay => {
                if terrain.replaceable(p) || painted_is_same(p) {
                    continue;
                }
                let above = add(p, [0, 1, 0]);
                if terrain.replaceable(above) {
                    edits.push(Edit::Set(above));
                }
            }
        }
    }
    edits
}

/// Blocks `getDestroySpeed` answers -1 for.
pub const UNBREAKABLE: &[&str] = &[
    "minecraft:bedrock",
    "minecraft:barrier",
    "minecraft:command_block",
    "minecraft:chain_command_block",
    "minecraft:repeating_command_block",
    "minecraft:structure_block",
    "minecraft:jigsaw",
    "minecraft:end_portal",
    "minecraft:end_portal_frame",
    "minecraft:end_gateway",
    "minecraft:nether_portal",
];

/// The first solid block along a ray (`Level.clip` with `Block.OUTLINE`, no fluids) and the face
/// it was entered through, for `solid(pos)`. `dir` need not be a unit vector.
pub fn raycast(origin: [f64; 3], dir: [f64; 3], range: f64, solid: &dyn Fn(Pos) -> bool) -> Option<(Pos, Dir)> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len == 0.0 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let mut cell = [origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32];
    let step: [i32; 3] = std::array::from_fn(|i| if d[i] > 0.0 { 1 } else { -1 });
    let mut t_max: [f64; 3] = std::array::from_fn(|i| {
        if d[i] == 0.0 {
            f64::INFINITY
        } else {
            let edge = if d[i] > 0.0 { f64::from(cell[i]) + 1.0 } else { f64::from(cell[i]) };
            (edge - origin[i]) / d[i]
        }
    });
    let t_delta: [f64; 3] = std::array::from_fn(|i| if d[i] == 0.0 { f64::INFINITY } else { 1.0 / d[i].abs() });
    let mut face = None;
    loop {
        if face.is_some() && solid(cell) {
            return face.map(|f| (cell, f));
        }
        let axis = (0..3).min_by(|a, b| t_max[*a].total_cmp(&t_max[*b]))?;
        if t_max[axis] > range {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        face = Some(match (axis, step[axis] > 0) {
            (0, true) => Dir::West,
            (0, false) => Dir::East,
            (1, true) => Dir::Down,
            (1, false) => Dir::Up,
            (_, true) => Dir::North,
            (_, false) => Dir::South,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuboids_cover_their_size() {
        assert_eq!(shape(Brush::Cuboid, [1, 1, 1]), vec![[0, 0, 0]]);
        assert_eq!(shape(Brush::Cuboid, [3, 3, 3]).len(), 27);
        assert_eq!(shape(Brush::Cuboid, [4, 2, 1]).len(), 8);
        // An even width reaches one block further to the positive side.
        let two = shape(Brush::Cuboid, [2, 1, 1]);
        assert_eq!(two, vec![[0, 0, 0], [1, 0, 0]]);
    }

    #[test]
    fn spheres_and_cylinders_are_round() {
        assert_eq!(shape(Brush::Sphere, [0, 0, 0]), vec![[0, 0, 0]]);
        // Radius 1: the centre, six faces and twelve edges (< 1.5 away); the corners are 1.73 away.
        assert_eq!(shape(Brush::Sphere, [1, 0, 0]).len(), 19);
        assert!(shape(Brush::Sphere, [3, 0, 0]).contains(&[3, 0, 0]));
        assert!(!shape(Brush::Sphere, [3, 0, 0]).contains(&[3, 3, 0]));
        let flat = shape(Brush::Cylinder, [1, 1, 0]);
        // The diagonals are 1.414 away, inside 1 + .42.
        assert_eq!(flat.len(), 9);
        assert!(flat.iter().all(|p| p[1] == 0));
        // Three layers are centred on the middle one.
        let tall = shape(Brush::Cylinder, [1, 3, 0]);
        assert_eq!(tall.len(), 27);
        assert!(tall.iter().any(|p| p[1] == -1) && tall.iter().any(|p| p[1] == 1));
    }

    #[test]
    fn the_placement_options_move_the_brush_off_the_face() {
        let params = Brush::Sphere.params(2);
        assert_eq!(offset(Brush::Sphere, params, Dir::Up, Placement::Merged), [0, 0, 0]);
        // Attached: the sphere sits on the face, radius + 1 blocks out.
        assert_eq!(offset(Brush::Sphere, params, Dir::Up, Placement::Attached), [0, 3, 0]);
        // Inserted: it is pushed into the block instead.
        assert_eq!(offset(Brush::Sphere, params, Dir::Up, Placement::Inserted), [0, -2, 0]);
        let cube = Brush::Cuboid.params(3);
        assert_eq!(offset(Brush::Cuboid, cube, Dir::East, Placement::Attached), [2, 0, 0]);
        assert_eq!(offset(Brush::Cuboid, cube, Dir::West, Placement::Attached), [-2, 0, 0]);
    }

    #[test]
    fn brushes_and_tools_cycle_within_what_they_support() {
        assert_eq!(Brush::Cluster.next(), Brush::Cuboid);
        assert_eq!(next_tool(Brush::Cluster, Tool::Replace), Tool::Clear);
        assert_eq!(next_tool(Brush::Cluster, Tool::Clear), Tool::Replace);
        assert_eq!(next_tool(Brush::Surface, Tool::Fill), Tool::Overlay);
        assert_eq!(Brush::Surface.redirect(Tool::Overlay), Tool::Place);
        assert!(!Tool::Clear.requires_block());
        assert_eq!(Brush::from_name("sphere"), Some(Brush::Sphere));
        assert_eq!(Tool::from_name("clear"), Some(Tool::Clear));
        assert_eq!(Placement::Inserted.next(), Placement::Merged);
    }

    /// A world of a stone floor (y <= 0) and air.
    struct Floor;

    impl Terrain for Floor {
        fn replaceable(&self, p: Pos) -> bool {
            p[1] > 0
        }
        fn unbreakable(&self, p: Pos) -> bool {
            p[1] < -3
        }
        fn same_block(&self, _: Pos, _: Pos) -> bool {
            true
        }
        fn solid_side(&self, p: Pos, _: Dir) -> bool {
            p[1] <= 0
        }
    }

    #[test]
    fn the_tools_edit_as_create_does() {
        let spots = [[0, 0, 0], [0, 1, 0]];
        let same = |_: Pos| false;
        // Fill only touches replaceable blocks, Replace only the others, Place everything.
        assert_eq!(run(&Floor, Tool::Fill, &spots, &same), vec![Edit::Set([0, 1, 0])]);
        assert_eq!(run(&Floor, Tool::Replace, &spots, &same), vec![Edit::Set([0, 0, 0])]);
        assert_eq!(run(&Floor, Tool::Place, &spots, &same).len(), 2);
        assert_eq!(run(&Floor, Tool::Clear, &spots, &same), vec![Edit::Air([0, 0, 0]), Edit::Air([0, 1, 0])]);
        // Overlay lays the block on top of the solid ones, unless they are the block already.
        assert_eq!(run(&Floor, Tool::Overlay, &spots, &same), vec![Edit::Set([0, 1, 0])]);
        assert!(run(&Floor, Tool::Overlay, &spots, &|_| true).is_empty());
    }

    #[test]
    fn the_surface_brush_floods_the_floor_within_its_range() {
        let found = dynamic_positions(&Floor, true, [2, 0, 0], [0, 0, 0], Dir::Up, Tool::Replace);
        // Range 2 around the block hit: those closer than 2 on the floor plane.
        assert!(found.contains(&[0, 0, 0]));
        assert!(found.contains(&[1, 0, 0]));
        assert!(found.contains(&[1, 0, 1]));
        assert!(!found.contains(&[2, 0, 0]));
        assert!(found.iter().all(|p| p[1] == 0));
    }

    #[test]
    fn a_ray_stops_at_the_first_solid_block_and_names_the_face() {
        let solid = |p: Pos| p[1] <= 0;
        // Looking straight down from above the floor.
        let hit = raycast([0.5, 5.5, 0.5], [0.0, -1.0, 0.0], 20.0, &solid);
        assert_eq!(hit, Some(([0, 0, 0], Dir::Up)));
        // Looking along the floor at a wall.
        let wall = |p: Pos| p[0] >= 5;
        assert_eq!(raycast([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 20.0, &wall), Some(([5, 0, 0], Dir::West)));
        assert_eq!(raycast([0.5, 0.5, 0.5], [-1.0, 0.0, 0.0], 20.0, &wall), None);
        assert_eq!(raycast([0.5, 5.5, 0.5], [0.0, -1.0, 0.0], 3.0, &solid), None);
    }
}

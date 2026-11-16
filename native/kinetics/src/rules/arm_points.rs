//! What a Mechanical Arm can work with and how (`ArmInteractionPoint`, `AllArmInteractionPointTypes`,
//! `ArmInteractionPointHandler`), from Create `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of
//! Create). No game types: the glue says what a block is and holds, and these rules answer.
//!
//! Create 6.0.10 has no point type for chests, barrels or vaults ("not every type of Inventory
//! can be interacted with directly": the Ponder text); a funnel or a depot bridges that. Types
//! for blocks this port doesn't have (the Mechanical Crafter, the Packager, the Track Station)
//! are left out.

use super::arm::{Angles, RANGE, target_angles};
use crate::{Dir, Pos};

/// `ArmInteractionPoint.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Deposit,
    Take,
}

impl Mode {
    /// `NBTHelper.writeEnum`.
    pub const fn name(self) -> &'static str {
        match self {
            Mode::Deposit => "DEPOSIT",
            Mode::Take => "TAKE",
        }
    }

    /// `NBTHelper.readEnum`: the first constant for an unknown name.
    pub fn from_name(name: &str) -> Self {
        if name == "TAKE" { Mode::Take } else { Mode::Deposit }
    }

    /// `cycleMode`.
    pub const fn other(self) -> Self {
        match self {
            Mode::Deposit => Mode::Take,
            Mode::Take => Mode::Deposit,
        }
    }

    /// `mechanical_arm.deposit_to` and `extract_from` in `en_us`, for the block's name.
    pub fn line(self, block: &str) -> String {
        match self {
            Mode::Deposit => format!("Deposit items to {block}"),
            Mode::Take => format!("Take items from {block}"),
        }
    }

    /// `Mode.getColor`, for the outline (here, particles).
    pub const fn color(self) -> u32 {
        match self {
            Mode::Deposit => 0xDDC166,
            Mode::Take => 0x7FCDE0,
        }
    }
}

/// An `ArmInteractionPointType`, in the order Create registers them (`getPrimaryType` takes the
/// first that fits; no two fit one block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointType {
    Basin,
    Belt,
    BlazeBurner,
    Chute,
    CrushingWheels,
    Deployer,
    Depot,
    Funnel,
    Millstone,
    Saw,
    Campfire,
    Composter,
    Jukebox,
    RespawnAnchor,
}

impl PointType {
    pub const ALL: [PointType; 14] = [
        PointType::Basin,
        PointType::Belt,
        PointType::BlazeBurner,
        PointType::Chute,
        PointType::CrushingWheels,
        PointType::Deployer,
        PointType::Depot,
        PointType::Funnel,
        PointType::Millstone,
        PointType::Saw,
        PointType::Campfire,
        PointType::Composter,
        PointType::Jukebox,
        PointType::RespawnAnchor,
    ];

    /// The id Create registers the type under, saved as `Type`.
    pub const fn id(self) -> &'static str {
        match self {
            PointType::Basin => "create:basin",
            PointType::Belt => "create:belt",
            PointType::BlazeBurner => "create:blaze_burner",
            PointType::Chute => "create:chute",
            PointType::CrushingWheels => "create:crushing_wheels",
            PointType::Deployer => "create:deployer",
            PointType::Depot => "create:depot",
            PointType::Funnel => "create:funnel",
            PointType::Millstone => "create:millstone",
            PointType::Saw => "create:saw",
            PointType::Campfire => "create:campfire",
            PointType::Composter => "create:composter",
            PointType::Jukebox => "create:jukebox",
            PointType::RespawnAnchor => "create:respawn_anchor",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// `DepositOnlyArmInteractionPoint`: nothing is taken from these, and clicking one twice
    /// does not make it an input.
    pub const fn deposit_only(self) -> bool {
        matches!(
            self,
            PointType::BlazeBurner
                | PointType::CrushingWheels
                | PointType::Funnel
                | PointType::Campfire
                | PointType::RespawnAnchor
        )
    }

    /// `ArmInteractionPoint.getSlotCount` for the types that don't read the container's own:
    /// `None` where the glue asks the block.
    pub const fn fixed_slots(self) -> Option<usize> {
        match self {
            PointType::BlazeBurner
            | PointType::CrushingWheels
            | PointType::Funnel
            | PointType::Campfire
            | PointType::RespawnAnchor => Some(0),
            PointType::Composter => Some(2),
            PointType::Jukebox => Some(1),
            _ => None,
        }
    }
}

/// What the glue reads of the block at a position for [`classify`].
pub struct Block<'a> {
    /// The block's id, `create:basin`.
    pub name: &'a str,
    /// A state property of the block (`facing`, `slope`, `extracting`, `shape`).
    pub property: &'a dyn Fn(&str) -> Option<String>,
    /// The id of the block above (a belt tunnel above a belt takes it out).
    pub above: &'a str,
    /// Whether the block is in the `minecraft:campfires` tag.
    pub campfire: bool,
    /// Whether the block entity's speed isn't zero (the saw's).
    pub moving: bool,
}

/// `ArmInteractionPointType.getPrimaryType`: the type of point the block makes, if any.
pub fn classify(block: &Block<'_>) -> Option<PointType> {
    PointType::ALL.into_iter().find(|t| can_create(*t, block))
}

/// `canCreatePoint` of each type.
pub fn can_create(ty: PointType, block: &Block<'_>) -> bool {
    let prop = |key: &str| (block.property)(key);
    match ty {
        PointType::Basin => block.name == "create:basin",
        // `AllBlocks.BELT.has(state) && !(above instanceof BeltTunnelBlock) &&
        // BeltBlock.canTransportObjects(state)`: not on a vertical or a sideways belt.
        PointType::Belt => {
            block.name == "create:belt"
                && !block.above.ends_with("_tunnel")
                && !matches!(prop("slope").as_deref(), Some("vertical" | "sideways"))
        }
        PointType::BlazeBurner => block.name == "create:blaze_burner",
        PointType::Chute => matches!(block.name, "create:chute" | "create:smart_chute"),
        PointType::CrushingWheels => block.name == "create:crushing_wheel_controller",
        PointType::Deployer => block.name == "create:deployer",
        PointType::Depot => matches!(block.name, "create:depot" | "create:weighted_ejector"),
        // An `AbstractFunnelBlock`, not an extracting funnel and not a belt funnel that pushes.
        PointType::Funnel => {
            matches!(
                block.name,
                "create:andesite_funnel"
                    | "create:brass_funnel"
                    | "create:andesite_belt_funnel"
                    | "create:brass_belt_funnel"
            ) && prop("extracting").as_deref() != Some("true")
                && prop("shape").as_deref() != Some("pushing")
        }
        PointType::Millstone => block.name == "create:millstone",
        PointType::Saw => {
            block.name == "create:mechanical_saw"
                && prop("facing").as_deref() == Some("up")
                && block.moving
        }
        PointType::Campfire => block.campfire,
        PointType::Composter => block.name == "minecraft:composter",
        PointType::Jukebox => block.name == "minecraft:jukebox",
        PointType::RespawnAnchor => block.name == "minecraft:respawn_anchor",
    }
}

/// The block's name as the lines shown to a player have it: Create's `en_us` names for its own
/// blocks, the id's words for the rest.
pub fn block_title(name: &str) -> String {
    match name {
        "create:belt" => return "Belt".to_owned(),
        "create:crushing_wheel_controller" => return "Crushing Wheel Controller".to_owned(),
        _ => {}
    }
    let path = name.rsplit(':').next().unwrap_or(name);
    path.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn plus(pos: Pos, offset: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| f64::from(pos[i]) + offset[i])
}

/// `getInteractionPositionVector` and `getInteractionDirection` (the way the claw points) of the
/// point of `ty` at `pos`. `facing` is the block's `facing` for the Deployer and the funnels.
pub fn interaction(ty: PointType, pos: Pos, facing: Option<Dir>) -> ([f64; 3], Dir) {
    let centre = plus(pos, [0.5; 3]);
    let along = |dir: Dir, scale: f64| {
        let n = dir.normal();
        [0, 1, 2].map(|i| centre[i] + f64::from(n[i]) * scale)
    };
    match ty {
        // `TopFaceArmInteractionPoint` (chutes, jukeboxes), the crushing wheels, the respawn anchor.
        PointType::Chute | PointType::Jukebox | PointType::CrushingWheels | PointType::RespawnAnchor => {
            (plus(pos, [0.5, 1.0, 0.5]), Dir::Down)
        }
        // `DepotPoint` (depots, ejectors, saws) and `BeltPoint`.
        PointType::Depot | PointType::Belt | PointType::Saw => (plus(pos, [0.5, 14.0 / 16.0, 0.5]), Dir::Down),
        PointType::Composter => (plus(pos, [0.5, 13.0 / 16.0, 0.5]), Dir::Down),
        // `DeployerPoint`: 0.65 out of the back of the block's facing.
        PointType::Deployer => {
            let dir = facing.unwrap_or(Dir::Up).opposite();
            (along(dir, 0.65), dir)
        }
        // `FunnelPoint`: 0.15 into the funnel, pointing into it.
        PointType::Funnel => match facing {
            Some(f) => (along(f, -0.15), f.opposite()),
            None => (centre, Dir::Up),
        },
        PointType::Basin | PointType::BlazeBurner | PointType::Millstone | PointType::Campfire => (centre, Dir::Down),
    }
}

/// `getTargetAngles` of a point.
pub fn point_angles(ty: PointType, point: Pos, facing: Option<Dir>, arm: Pos, ceiling: bool) -> Angles {
    let (at, claw) = interaction(ty, point, facing);
    target_angles(arm, at, claw, ceiling)
}

// --- the selection a player makes with the item (`ArmInteractionPointHandler`) -----------------------

/// A selected block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub ty: PointType,
    pub pos: Pos,
    pub mode: Mode,
}

/// `Vec3i.closerThan(pos, RANGE)`: nearer than the range, exclusive.
pub fn in_range(arm: Pos, point: Pos) -> bool {
    let d = [0, 1, 2].map(|i| i64::from(arm[i]) - i64::from(point[i]));
    d.iter().map(|v| v * v).sum::<i64>() < i64::from(RANGE) * i64::from(RANGE)
}

/// What `flushSettings` found when the arm was placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flush {
    /// The points kept, in the order they were selected.
    pub points: Vec<Point>,
    /// How many were out of range and dropped.
    pub removed: usize,
    pub inputs: usize,
    pub outputs: usize,
}

impl Flush {
    /// The line `flushSettings` shows (`points_outside_range` in red, else `summary`), if any.
    pub fn message(&self) -> Option<(String, bool)> {
        if self.removed > 0 {
            return Some((
                format!("{} selected interaction point(s) removed due to range limitations.", self.removed),
                true,
            ));
        }
        (self.inputs + self.outputs > 0).then(|| {
            (format!("Mechanical Arm has {} input(s) and {} output(s).", self.inputs, self.outputs), false)
        })
    }
}

/// `ArmInteractionPointHandler.currentSelection`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub points: Vec<Point>,
}

impl Selection {
    pub fn find(&self, pos: Pos) -> Option<usize> {
        self.points.iter().position(|p| p.pos == pos)
    }

    /// `rightClickingBlocksSelectsThem`: a block already selected has its mode cycled; a new
    /// one of type `ty` is added and cycled; a block that makes no point (`None`) does nothing.
    /// The mode the point has now.
    pub fn right_click(&mut self, pos: Pos, ty: Option<PointType>) -> Option<(PointType, Mode)> {
        let index = match self.find(pos) {
            Some(i) => i,
            None => {
                let ty = ty?;
                self.points.push(Point { ty, pos, mode: Mode::Deposit });
                self.points.len() - 1
            }
        };
        let point = &mut self.points[index];
        // `DepositOnlyArmInteractionPoint.cycleMode` does nothing.
        if !point.ty.deposit_only() {
            point.mode = point.mode.other();
        }
        Some((point.ty, point.mode))
    }

    /// `leftClickingBlocksDeselectsThem`: whether the block was selected.
    pub fn left_click(&mut self, pos: Pos) -> bool {
        match self.find(pos) {
            Some(i) => {
                self.points.remove(i);
                true
            }
            None => false,
        }
    }

    /// `flushSettings`: what stays when an arm is placed at `arm`.
    pub fn flush(&self, arm: Pos) -> Flush {
        let points: Vec<Point> = self.points.iter().copied().filter(|p| in_range(arm, p.pos)).collect();
        let removed = self.points.len() - points.len();
        let outputs = points.iter().filter(|p| p.mode == Mode::Deposit).count();
        Flush { inputs: points.len() - outputs, outputs, removed, points }
    }
}

/// `initInteractionPoints`: the points as the arm lists them: takes become inputs, deposits
/// outputs, each in the order saved. Entries are indexes into `points`.
pub fn split(points: &[Point]) -> (Vec<usize>, Vec<usize>) {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (i, p) in points.iter().enumerate() {
        match p.mode {
            Mode::Take => inputs.push(i),
            Mode::Deposit => outputs.push(i),
        }
    }
    (inputs, outputs)
}

// --- what each point takes and gives ----------------------------------------------------------------

/// `ItemHandlerHelper.insertItem` in a simulation over a handler's slots (each `Some((key, count))`):
/// the stack goes into each slot in turn that `accepts` it, topping up an equal stack or filling an
/// empty one, up to `limit(slot)`. What is left over of `count`.
pub fn insert_into_slots<K: PartialEq>(
    slots: &[Option<(K, u32)>],
    key: &K,
    count: u32,
    limit: &dyn Fn(usize) -> u32,
    accepts: &dyn Fn(usize) -> bool,
) -> u32 {
    let mut left = count;
    for (i, slot) in slots.iter().enumerate() {
        if left == 0 {
            break;
        }
        if !accepts(i) {
            continue;
        }
        let room = match slot {
            Some((have, n)) if have == key => limit(i).saturating_sub(*n),
            Some(_) => 0,
            None => limit(i),
        };
        left -= room.min(left);
    }
    left
}

/// `BlazeBurnerPoint.insert`: what `BlazeBurnerBlock.tryInsert` did to the copy of the stack
/// (`count` items), and so what comes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fed {
    /// The burner took nothing, or was given a Creative Blaze Cake (which it keeps using without
    /// taking it): the arm still holds the stack.
    Unchanged,
    /// One item was taken (the stack shrank), leaving `left`; a container (an empty bucket) came
    /// back if the item had one.
    Took { left: u32, container: bool },
}

/// The outcome of `tryInsert` for a stack of `count`: `accepted` says the fuel was taken.
pub fn blaze_burner_fed(count: u32, accepted: bool, creative_cake: bool, has_container: bool) -> Fed {
    if creative_cake || !accepted {
        Fed::Unchanged
    } else {
        Fed::Took { left: count.saturating_sub(1), container: has_container }
    }
}

/// `FunnelPoint.insert`'s first checks: the funnel isn't powered and its filter lets the stack by.
pub fn funnel_takes(powered: bool, filter_passes: bool) -> bool {
    !powered && filter_passes
}

/// `RespawnAnchorPoint.insert`: the charge after a stack of `item` is put in, if it is taken
/// (glowstone, below 4 charges; the charge defaults to 4 where the block has none).
pub fn respawn_anchor_charge(item: &str, charge: Option<u8>) -> Option<u8> {
    let charge = charge.unwrap_or(4);
    (item == "minecraft:glowstone" && charge != 4).then_some(charge + 1)
}

/// `JukeboxPoint.insert`: a playable record goes into an empty jukebox (`HAS_RECORD` unset).
pub fn jukebox_takes(playable: bool, has_record: bool) -> bool {
    playable && !has_record
}

/// `ComposterPoint`'s composter: `InputContainer.canPlaceItemThroughFace` from above: below
/// level 7 and a compostable item.
pub fn composter_takes(level: u8, compostable: bool) -> bool {
    level < 7 && compostable
}

/// The composter's output (`OutputContainer`): at level 8 one bone meal, which empties it.
pub fn composter_output(level: u8) -> Option<&'static str> {
    (level == 8).then_some("minecraft:bone_meal")
}

/// `CampfirePoint.insert`: the stack is taken if a campfire recipe cooks it (`cooking` ticks) and
/// a slot is free; one item goes onto the fire.
pub fn campfire_takes(cooking: Option<u32>, free_slots: usize) -> Option<u32> {
    cooking.filter(|_| free_slots > 0)
}

/// How many slots the vanilla container block `name` has (a single chest; a double chest is two
/// of these), for working out what a funnel's target has room for.
pub fn vanilla_container_size(name: &str) -> Option<usize> {
    let path = name.strip_prefix("minecraft:")?;
    Some(match path {
        "chest" | "trapped_chest" | "barrel" => 27,
        "hopper" => 5,
        "dropper" | "dispenser" => 9,
        p if p.ends_with("shulker_box") => 27,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(
        name: &'a str,
        props: &'a dyn Fn(&str) -> Option<String>,
        above: &'a str,
    ) -> Block<'a> {
        Block { name, property: props, above, campfire: false, moving: true }
    }

    #[test]
    fn blocks_make_the_points_create_lists() {
        let none = |_: &str| None;
        for (name, ty) in [
            ("create:basin", PointType::Basin),
            ("create:belt", PointType::Belt),
            ("create:blaze_burner", PointType::BlazeBurner),
            ("create:smart_chute", PointType::Chute),
            ("create:weighted_ejector", PointType::Depot),
            ("create:depot", PointType::Depot),
            ("create:millstone", PointType::Millstone),
            ("minecraft:composter", PointType::Composter),
            ("minecraft:jukebox", PointType::Jukebox),
            ("minecraft:respawn_anchor", PointType::RespawnAnchor),
        ] {
            assert_eq!(classify(&facts(name, &none, "minecraft:air")), Some(ty), "{name}");
        }
        assert_eq!(classify(&facts("minecraft:chest", &none, "minecraft:air")), None);
        assert_eq!(classify(&facts("create:item_vault", &none, "minecraft:air")), None);
    }

    #[test]
    fn a_belt_under_a_tunnel_or_standing_up_is_no_point() {
        let flat = |_: &str| None;
        let vertical = |k: &str| (k == "slope").then(|| "vertical".to_owned());
        assert_eq!(classify(&facts("create:belt", &flat, "create:brass_tunnel")), None);
        assert_eq!(classify(&facts("create:belt", &vertical, "minecraft:air")), None);
    }

    #[test]
    fn a_saw_is_a_point_only_upright_and_turning() {
        let up = |k: &str| (k == "facing").then(|| "up".to_owned());
        let side = |k: &str| (k == "facing").then(|| "north".to_owned());
        assert_eq!(classify(&facts("create:mechanical_saw", &up, "minecraft:air")), Some(PointType::Saw));
        assert_eq!(classify(&facts("create:mechanical_saw", &side, "minecraft:air")), None);
        let mut still = facts("create:mechanical_saw", &up, "minecraft:air");
        still.moving = false;
        assert_eq!(classify(&still), None);
    }

    #[test]
    fn extracting_and_pushing_funnels_are_no_points() {
        let extracting = |k: &str| (k == "extracting").then(|| "true".to_owned());
        let pushing = |k: &str| (k == "shape").then(|| "pushing".to_owned());
        let none = |_: &str| None;
        assert_eq!(classify(&facts("create:brass_funnel", &none, "x")), Some(PointType::Funnel));
        assert_eq!(classify(&facts("create:brass_funnel", &extracting, "x")), None);
        assert_eq!(classify(&facts("create:andesite_belt_funnel", &pushing, "x")), None);
    }

    #[test]
    fn the_claw_points_into_what_it_works_on() {
        let (at, claw) = interaction(PointType::Depot, [2, 0, 0], None);
        assert_eq!((at, claw), ([2.5, 14.0 / 16.0, 0.5], Dir::Down));
        // A deployer facing north is reached from the south, the claw looking north.
        let (at, claw) = interaction(PointType::Deployer, [0, 0, 0], Some(Dir::North));
        assert_eq!(claw, Dir::South);
        assert!((at[2] - 1.15).abs() < 1e-9, "{at:?}");
        // A funnel facing east: in 0.15 from its middle, the claw looking west.
        let (at, claw) = interaction(PointType::Funnel, [0, 0, 0], Some(Dir::East));
        assert_eq!(claw, Dir::West);
        assert!((at[0] - 0.35).abs() < 1e-9, "{at:?}");
    }

    #[test]
    fn clicking_selects_then_toggles_and_deposit_only_stays() {
        let mut s = Selection::default();
        assert_eq!(s.right_click([1, 0, 0], None), None);
        assert_eq!(s.right_click([1, 0, 0], Some(PointType::Depot)), Some((PointType::Depot, Mode::Take)));
        assert_eq!(s.right_click([1, 0, 0], None), Some((PointType::Depot, Mode::Deposit)));
        assert_eq!(s.right_click([2, 0, 0], Some(PointType::Funnel)), Some((PointType::Funnel, Mode::Deposit)));
        assert_eq!(s.right_click([2, 0, 0], None), Some((PointType::Funnel, Mode::Deposit)));
        assert!(s.left_click([1, 0, 0]));
        assert!(!s.left_click([1, 0, 0]));
        assert_eq!(s.points.len(), 1);
    }

    #[test]
    fn placing_keeps_points_within_the_range() {
        let mut s = Selection::default();
        s.right_click([1, 0, 0], Some(PointType::Depot));
        s.right_click([4, 0, 0], Some(PointType::Depot));
        s.right_click([5, 0, 0], Some(PointType::Depot));
        s.right_click([0, 0, 0], Some(PointType::Basin));
        let flush = s.flush([0, 0, 0]);
        // 5 blocks away is not closer than 5.
        assert_eq!(flush.removed, 1);
        assert_eq!((flush.inputs, flush.outputs), (3, 0));
        assert!(flush.message().unwrap().1);
        let ok = Selection { points: s.points[..2].to_vec() }.flush([0, 0, 0]);
        assert_eq!(ok.message().unwrap().0, "Mechanical Arm has 2 input(s) and 0 output(s).");
        assert_eq!(Selection::default().flush([0, 0, 0]).message(), None);
    }

    #[test]
    fn the_points_split_into_inputs_and_outputs_in_order() {
        let p = |mode| Point { ty: PointType::Depot, pos: [0; 3], mode };
        let (inputs, outputs) = split(&[p(Mode::Take), p(Mode::Deposit), p(Mode::Take)]);
        assert_eq!((inputs, outputs), (vec![0, 2], vec![1]));
    }

    #[test]
    fn a_handler_fills_equal_stacks_then_empty_slots() {
        let slots = [Some(("a", 60)), None, Some(("b", 1))];
        let limit = |_: usize| 64;
        let all = |_: usize| true;
        assert_eq!(insert_into_slots(&slots, &"a", 10, &limit, &all), 0);
        assert_eq!(insert_into_slots(&slots, &"a", 200, &limit, &all), 200 - 4 - 64);
        let only_first = |i: usize| i == 0;
        assert_eq!(insert_into_slots(&slots, &"a", 10, &limit, &only_first), 6);
    }

    #[test]
    fn the_burner_keeps_a_creative_cake_and_shrinks_other_fuel() {
        assert_eq!(blaze_burner_fed(3, true, false, false), Fed::Took { left: 2, container: false });
        assert_eq!(blaze_burner_fed(1, true, false, true), Fed::Took { left: 0, container: true });
        assert_eq!(blaze_burner_fed(3, true, true, false), Fed::Unchanged);
        assert_eq!(blaze_burner_fed(3, false, false, false), Fed::Unchanged);
    }

    #[test]
    fn small_blocks_follow_their_rules() {
        assert_eq!(respawn_anchor_charge("minecraft:glowstone", Some(3)), Some(4));
        assert_eq!(respawn_anchor_charge("minecraft:glowstone", Some(4)), None);
        assert_eq!(respawn_anchor_charge("minecraft:stone", Some(0)), None);
        assert!(jukebox_takes(true, false) && !jukebox_takes(true, true) && !jukebox_takes(false, false));
        assert!(composter_takes(6, true) && !composter_takes(7, true) && !composter_takes(0, false));
        assert_eq!(composter_output(8), Some("minecraft:bone_meal"));
        assert_eq!(composter_output(7), None);
        assert_eq!(campfire_takes(Some(600), 1), Some(600));
        assert_eq!(campfire_takes(Some(600), 0), None);
        assert_eq!(campfire_takes(None, 4), None);
        assert!(funnel_takes(false, true) && !funnel_takes(true, true) && !funnel_takes(false, false));
    }

    #[test]
    fn names_read_as_create_shows_them() {
        assert_eq!(block_title("create:blaze_burner"), "Blaze Burner");
        assert_eq!(block_title("minecraft:soul_campfire"), "Soul Campfire");
        assert_eq!(block_title("create:belt"), "Belt");
        assert_eq!(vanilla_container_size("minecraft:red_shulker_box"), Some(27));
        assert_eq!(vanilla_container_size("minecraft:stone"), None);
        assert_eq!(Mode::Take.line("Basin"), "Take items from Basin");
        assert_eq!(PointType::from_id("create:saw"), Some(PointType::Saw));
        assert_eq!(Mode::from_name(Mode::Take.name()), Mode::Take);
    }
}

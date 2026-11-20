//! Which blocks have value boxes, and what each box is: the table of every `addBehaviours` that adds a
//! `FilteringBehaviour`, `ScrollValueBehaviour` or `ScrollOptionBehaviour` of Create's blocks that this port
//! has, with the slot, label, range, default and option list taken from them. The shared machinery
//! (`value_box`, `value_box_board`, the glue in `native/api/src/machines/value_box*.rs`) works on this table
//! alone, so a block is moved over by adding its row here.
//!
//! What is in Create 6.0.10 and not in the brief this was done from: the Brass Funnel has a filter slot but no
//! "mode" box (its `Mode` enum is its working state), the Mechanical Arm has one option box (the selection
//! mode) and no range, and the Sequenced Gearshift, the Display Link and the Stockpile Switch's thresholds are
//! screens (`SequencedGearshiftScreen`, `DisplayLinkScreen`, `ThresholdSwitchScreen`), not value boxes: only the
//! Stockpile Switch's filter slot is one. Those screens keep their stand-ins (see the README).

use super::value_box::{Props, Slot, Vec3};
use super::value_box_board::Behaviour;
use crate::Dir;

/// What a filter slot filters (`FilteringBehaviour.forRecipes`, `forFluids`): its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Items,
    Recipes,
    Fluids,
    /// `RollerBlockEntity`'s filter: the block to pave with.
    Paving,
}

impl FilterKind {
    /// `FilteringBehaviour.getLabel`.
    pub fn label(self) -> &'static str {
        match self {
            FilterKind::Items => "create.logistics.filter",
            FilterKind::Recipes => "create.logistics.recipe_filter",
            FilterKind::Fluids => "create.logistics.fluid_filter",
            FilterKind::Paving => super::roller::FILTER_LABEL,
        }
    }
}

/// What a box holds.
#[derive(Clone, Copy)]
pub enum Kind {
    /// A filter slot: an item to filter by (`Filter`), and, where `count` says so, an amount (`FilterAmount`,
    /// `UpTo`): `FilteringBehaviour.showCountWhen`.
    Filter { kind: FilterKind, count: fn(&Props) -> bool },
    /// A number or an option (`ScrollValue`).
    Value { label: &'static str, behaviour: Behaviour },
}

/// One value box of a block.
#[derive(Clone, Copy)]
pub struct BoxDef {
    /// `ValueSettingsBehaviour.netId`: 0 for a scroll value or option, 1 for a filter. Says which of a block's
    /// boxes a message is about.
    pub net_id: u8,
    pub slot: Slot,
    pub kind: Kind,
    /// `isActive` (`onlyActiveWhen`): whether the box is there at all in this state.
    pub active: fn(&Props) -> bool,
}

impl BoxDef {
    /// The language key of the box's label.
    pub fn label(&self) -> &'static str {
        match self.kind {
            Kind::Filter { kind, .. } => kind.label(),
            Kind::Value { label, .. } => label,
        }
    }

    pub fn is_filter(&self) -> bool {
        matches!(self.kind, Kind::Filter { .. })
    }

    /// `FilteringBehaviour.isCountVisible` apart from the stack size: whether the box takes an amount.
    pub fn shows_count(&self, p: &Props) -> bool {
        match self.kind {
            Kind::Filter { count, .. } => count(p),
            Kind::Value { .. } => false,
        }
    }

    /// The board's behaviour for a box that opens one.
    pub fn behaviour(&self) -> Option<Behaviour> {
        match self.kind {
            Kind::Filter { .. } => Some(Behaviour::Amount),
            Kind::Value { behaviour, .. } => Some(behaviour),
        }
    }
}

fn always(_: &Props) -> bool {
    true
}

fn never(_: &Props) -> bool {
    false
}

/// `SmartChuteBlockEntity.isExtracting`, which the glue works out (the block it pulls from is not a chute and
/// is not replaceable).
fn smart_chute_extracts(p: &Props) -> bool {
    p.flag("fact:extracting")
}

/// `FunnelBlockEntity.supportsAmountOnFilter`: an extracting funnel, or a belt funnel that pushes or that has a
/// belt under it.
fn funnel_supports_amount(p: &Props) -> bool {
    if p.block.ends_with("belt_funnel") {
        p.is("shape", "pushing") || p.flag("fact:belt_below")
    } else {
        p.flag("extracting")
    }
}

/// `BrassTunnelBlockEntity.hasDistributionBehaviour`, which the glue reads from the tunnel's flaps and joins.
fn tunnel_distributes(p: &Props) -> bool {
    p.flag("fact:distribution")
}

/// `BrassTunnelBlockEntity.SelectionMode`, by `INamedIconOptions.getTranslationKey`.
pub const TUNNEL_MODES: &[&str] = &[
    "create.tunnel.selection_mode.split",
    "create.tunnel.selection_mode.forced_split",
    "create.tunnel.selection_mode.round_robin",
    "create.tunnel.selection_mode.forced_round_robin",
    "create.tunnel.selection_mode.prefer_nearest",
    "create.tunnel.selection_mode.randomize",
    "create.tunnel.selection_mode.synchronize",
];

/// `ArmBlockEntity.SelectionMode`.
pub const ARM_MODES: &[&str] = &[
    "create.mechanical_arm.selection_mode.round_robin",
    "create.mechanical_arm.selection_mode.forced_round_robin",
    "create.mechanical_arm.selection_mode.prefer_first",
];

/// `CreativeMotorBlockEntity` and `SpeedControllerBlockEntity`: -256 to 256, 16 to start
/// (`MAX_SPEED`, `DEFAULT_SPEED`; the controller's maximum is the config's `maxRotationSpeed`, 256).
const SPEED: Behaviour = Behaviour::Kinetic { min: -256, max: 256, default: 16 };

const fn filter(slot: Slot, kind: FilterKind, count: fn(&Props) -> bool, active: fn(&Props) -> bool) -> BoxDef {
    BoxDef { net_id: 1, slot, kind: Kind::Filter { kind, count }, active }
}

static SMART_CHUTE: [BoxDef; 1] = [filter(Slot::SmartChute, FilterKind::Items, smart_chute_extracts, always)];
// `FunnelBlockEntity`: only the brass ones filter (`supportsFiltering`), so the andesite funnels have no row.
static BRASS_FUNNEL: [BoxDef; 1] = [filter(Slot::Funnel, FilterKind::Items, funnel_supports_amount, always)];
static BRASS_TUNNEL: [BoxDef; 2] = [
    // `BrassTunnelBlockEntity.selectionMode`: `ScrollOptionBehaviour`, active only with a distribution.
    BoxDef {
        net_id: 0,
        slot: Slot::BrassTunnelMode,
        kind: Kind::Value {
            label: "create.logistics.when_multiple_outputs_available",
            behaviour: Behaviour::Options { options: TUNNEL_MODES },
        },
        active: tunnel_distributes,
    },
    // `BrassTunnelBlockEntity.filtering`: a `SidedFilteringBehaviour`, one filter for each open side.
    filter(Slot::BrassTunnelFilter, FilterKind::Items, never, always),
];
static SAW: [BoxDef; 1] = [filter(Slot::Saw, FilterKind::Recipes, never, always)];
static MOTOR: [BoxDef; 1] = [BoxDef {
    net_id: 0,
    slot: Slot::Motor,
    kind: Kind::Value { label: "create.kinetics.creative_motor.rotation_speed", behaviour: SPEED },
    active: always,
}];
static SPEED_CONTROLLER: [BoxDef; 1] = [BoxDef {
    net_id: 0,
    slot: Slot::SpeedController,
    kind: Kind::Value { label: "create.kinetics.speed_controller.rotation_speed", behaviour: SPEED },
    active: always,
}];
static ARM: [BoxDef; 1] = [BoxDef {
    net_id: 0,
    slot: Slot::Arm,
    kind: Kind::Value {
        label: "create.logistics.when_multiple_outputs_available",
        behaviour: Behaviour::Options { options: ARM_MODES },
    },
    active: always,
}];
static DEPLOYER: [BoxDef; 1] = [filter(Slot::Deployer, FilterKind::Items, never, always)];
static OBSERVER: [BoxDef; 1] = [filter(Slot::FilteredDetector { bottom: false }, FilterKind::Items, never, always)];
static STOCKPILE_SWITCH: [BoxDef; 1] =
    [filter(Slot::FilteredDetector { bottom: true }, FilterKind::Items, never, always)];
static SMART_FLUID_PIPE: [BoxDef; 1] = [filter(Slot::SmartPipe, FilterKind::Fluids, never, always)];
// `RollerBlockEntity.addBehaviours`: the paving filter (`RollerValueBox(3)`) and the mode (`RollerValueBox(-3)`), a
// `ScrollOptionBehaviour<RollingMode>`.
static ROLLER: [BoxDef; 2] = [
    filter(Slot::Roller { h_offset: 3 }, FilterKind::Paving, never, always),
    BoxDef {
        net_id: 0,
        slot: Slot::Roller { h_offset: -3 },
        kind: Kind::Value {
            label: super::roller::MODE_LABEL,
            behaviour: Behaviour::Options { options: &super::roller::MODE_KEYS },
        },
        active: always,
    },
];

/// The value boxes of block `block`, none for a block without any.
pub fn boxes(block: &str) -> &'static [BoxDef] {
    match block {
        "create:smart_chute" => &SMART_CHUTE,
        "create:brass_funnel" | "create:brass_belt_funnel" => &BRASS_FUNNEL,
        "create:brass_tunnel" => &BRASS_TUNNEL,
        "create:mechanical_saw" => &SAW,
        "create:creative_motor" => &MOTOR,
        "create:rotation_speed_controller" => &SPEED_CONTROLLER,
        "create:mechanical_arm" => &ARM,
        "create:deployer" => &DEPLOYER,
        "create:content_observer" => &OBSERVER,
        "create:stockpile_switch" => &STOCKPILE_SWITCH,
        "create:smart_fluid_pipe" => &SMART_FLUID_PIPE,
        "create:mechanical_roller" => &ROLLER,
        _ => &[],
    }
}

/// Every block id that has a box, for the glue to test a looked-at block cheaply.
pub fn has_boxes(block: &str) -> bool {
    !boxes(block).is_empty()
}

/// The first of the block's boxes that `local_hit` (on the face `side`) lands on and that is active
/// (`ValueSettingsInputHandler.onBlockActivated`: the behaviours in order, the first that is active and hit).
pub fn hit_box(block: &str, p: &Props, side: Dir, local_hit: Vec3) -> Option<&'static BoxDef> {
    boxes(block).iter().find(|def| (def.active)(p) && def.slot.test_hit(p, side, local_hit))
}

/// `BlockState.canBeReplaced` for the blocks that matter to `SmartChuteBlockEntity.isExtracting`: air, fluids and the
/// plants a placed block takes the place of. (Horizon can't ask a block this at run time; this is the list
/// of vanilla's `replaceable` blocks a chute can sit next to.)
pub fn is_replaceable(block: &str) -> bool {
    const BASE: &[&str] = &[
        "air", "cave_air", "void_air", "water", "lava", "short_grass", "tall_grass", "fern", "large_fern",
        "dead_bush", "vine", "seagrass", "tall_seagrass", "snow", "fire", "soul_fire", "light", "bubble_column",
        "glow_lichen", "hanging_roots",
    ];
    block.strip_prefix("minecraft:").is_some_and(|name| BASE.contains(&name))
}

/// The value a box starts with before anything is saved: `ScrollValueBehaviour`'s `value` (0 unless the block
/// sets it).
pub fn default_value(def: &BoxDef) -> i32 {
    match def.kind {
        Kind::Value { behaviour: Behaviour::Scroll { default, .. } | Behaviour::Kinetic { default, .. }, .. } => {
            default
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props<'a>(block: &'a str, get: &'a dyn Fn(&str) -> Option<String>) -> Props<'a> {
        Props::new(block, get)
    }

    #[test]
    fn every_listed_block_has_boxes_and_others_none() {
        for block in [
            "create:smart_chute",
            "create:brass_funnel",
            "create:brass_belt_funnel",
            "create:brass_tunnel",
            "create:mechanical_saw",
            "create:creative_motor",
            "create:rotation_speed_controller",
            "create:mechanical_arm",
            "create:deployer",
            "create:content_observer",
            "create:stockpile_switch",
            "create:smart_fluid_pipe",
            "create:mechanical_roller",
        ] {
            assert!(has_boxes(block), "{block}");
        }
        assert!(!has_boxes("create:andesite_funnel"));
        assert!(!has_boxes("create:shaft"));
    }

    #[test]
    fn the_motor_and_the_controller_start_at_16_and_go_to_256() {
        let get = |_: &str| None;
        let p = props("create:creative_motor", &get);
        let def = &boxes("create:creative_motor")[0];
        assert_eq!(default_value(def), 16);
        assert!(!def.is_filter());
        assert_eq!(def.label(), "create.kinetics.creative_motor.rotation_speed");
        let Kind::Value { behaviour: Behaviour::Kinetic { min, max, .. }, .. } = def.kind else {
            panic!("a kinetic value")
        };
        assert_eq!((min, max), (-256, 256));
        assert_eq!(
            boxes("create:rotation_speed_controller")[0].label(),
            "create.kinetics.speed_controller.rotation_speed"
        );
        assert!(!def.shows_count(&p));
    }

    #[test]
    fn the_roller_has_a_paving_filter_and_a_three_option_mode() {
        let defs = boxes("create:mechanical_roller");
        assert_eq!(defs.len(), 2);
        assert!(defs[0].is_filter());
        assert_eq!(defs[0].net_id, 1);
        assert_eq!(defs[0].label(), "create.contraptions.mechanical_roller.pave_material");
        assert_eq!(defs[1].net_id, 0);
        assert_eq!(defs[1].label(), "create.contraptions.roller_mode");
        let Kind::Value { behaviour: Behaviour::Options { options }, .. } = defs[1].kind else {
            panic!("an option box")
        };
        assert_eq!(options.len(), 3);
        // The filter shows no amount.
        let get = |_: &str| None;
        assert!(!defs[0].shows_count(&props("create:mechanical_roller", &get)));
    }

    #[test]
    fn the_roller_boxes_are_hit_on_top_whatever_face_the_ray_names() {
        let get = |key: &str| (key == "facing").then(|| "north".to_owned());
        let p = props("create:mechanical_roller", &get);
        let filter = hit_box("create:mechanical_roller", &p, Dir::Up, [11.0 / 16.0, 15.5 / 16.0, 11.0 / 16.0]).unwrap();
        assert_eq!(filter.net_id, 1);
        let mode = hit_box("create:mechanical_roller", &p, Dir::Up, [5.0 / 16.0, 15.5 / 16.0, 11.0 / 16.0]).unwrap();
        assert_eq!(mode.net_id, 0);
        assert!(hit_box("create:mechanical_roller", &p, Dir::Up, [0.5, 15.5 / 16.0, 0.5]).is_none());
    }

    #[test]
    fn the_option_lists_are_createss_in_order() {
        assert_eq!(TUNNEL_MODES.len(), 7);
        assert_eq!(TUNNEL_MODES[6], "create.tunnel.selection_mode.synchronize");
        assert_eq!(ARM_MODES.len(), 3);
        assert_eq!(ARM_MODES[2], "create.mechanical_arm.selection_mode.prefer_first");
    }

    #[test]
    fn a_funnel_takes_an_amount_when_it_extracts_or_pushes() {
        let state = |extracting: bool, shape: &'static str, belt: bool| {
            move |key: &str| match key {
                "extracting" => Some(extracting.to_string()),
                "shape" => Some(shape.to_owned()),
                "fact:belt_below" => Some(belt.to_string()),
                _ => None,
            }
        };
        let def = &boxes("create:brass_funnel")[0];
        let get = state(true, "retracted", false);
        assert!(def.shows_count(&props("create:brass_funnel", &get)));
        let get = state(false, "retracted", false);
        assert!(!def.shows_count(&props("create:brass_funnel", &get)));
        let get = state(false, "pushing", false);
        assert!(def.shows_count(&props("create:brass_belt_funnel", &get)));
        let get = state(false, "retracted", true);
        assert!(def.shows_count(&props("create:brass_belt_funnel", &get)));
        let get = state(false, "retracted", false);
        assert!(!def.shows_count(&props("create:brass_belt_funnel", &get)));
    }

    #[test]
    fn the_first_box_hit_wins() {
        // A Brass Tunnel with an opening on the south: the mode box on top, the filter on the south face.
        let get = |key: &str| match key {
            "fact:side_south" => Some("true".to_owned()),
            "fact:distribution" => Some("true".to_owned()),
            _ => None,
        };
        let p = props("create:brass_tunnel", &get);
        let top = hit_box("create:brass_tunnel", &p, Dir::Up, [0.5, 1.0, 0.5]).unwrap();
        assert_eq!(top.net_id, 0);
        let south = hit_box("create:brass_tunnel", &p, Dir::South, [0.5, 13.0 / 16.0, 1.0]).unwrap();
        assert_eq!(south.net_id, 1);
        // No opening on the west: no filter box there.
        assert!(hit_box("create:brass_tunnel", &p, Dir::West, [0.0, 13.0 / 16.0, 0.5]).is_none());
        // Without a distribution the mode box is inactive.
        let none = |_: &str| None;
        let p = props("create:brass_tunnel", &none);
        assert!(hit_box("create:brass_tunnel", &p, Dir::Up, [0.5, 1.0, 0.5]).is_none());
    }

    #[test]
    fn air_and_plants_are_replaceable_and_chests_are_not() {
        assert!(is_replaceable("minecraft:air"));
        assert!(is_replaceable("minecraft:water"));
        assert!(is_replaceable("minecraft:short_grass"));
        assert!(!is_replaceable("minecraft:chest"));
        assert!(!is_replaceable("create:chute"));
    }

    #[test]
    fn a_smart_chute_shows_its_amount_only_when_it_pulls() {
        let def = &boxes("create:smart_chute")[0];
        let yes = |key: &str| (key == "fact:extracting").then(|| "true".to_owned());
        assert!(def.shows_count(&props("create:smart_chute", &yes)));
        let no = |_: &str| None;
        assert!(!def.shows_count(&props("create:smart_chute", &no)));
    }
}

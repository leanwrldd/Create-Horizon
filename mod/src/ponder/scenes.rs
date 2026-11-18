//! Storyboards adapted from Create's Ponder scenes for the mechanics available in Horizon.

use super::scene::{Block, Category::*, Pos, Scene, Step};

const X: &[(&str, &str)] = &[("axis", "x")];
const Y: &[(&str, &str)] = &[("axis", "y")];
const Z: &[(&str, &str)] = &[("axis", "z")];
const EAST: &[(&str, &str)] = &[("facing", "east")];
const SOUTH: &[(&str, &str)] = &[("facing", "south")];
const BASE_RPM: f32 = 16.0;

const fn shaft(pos: Pos, axis: &'static [(&'static str, &'static str)], rpm: f32) -> Block {
    Block::new(pos, "create:shaft", axis).rotating(rpm)
}

const fn belt(pos: Pos, part: &'static [(&'static str, &'static str)]) -> Block {
    Block::new(pos, "create:belt", part).rotating(BASE_RPM)
}

const MOTOR: Block = Block::new([0, 1, 2], "create:creative_motor", EAST);
const CRANK: Block = Block::new([4, 1, 2], "create:hand_crank", EAST);
const DEPOT: Block = Block::new([2, 1, 2], "create:depot", &[]);
const PRESS: Block = Block::new([2, 3, 2], "create:mechanical_press", EAST);
const BELT_START: Block = belt(
    [1, 1, 1],
    &[
        ("facing", "south"),
        ("part", "start"),
        ("slope", "horizontal"),
        ("casing", "false"),
    ],
);
const BELT_MIDDLE: Block = belt(
    [1, 1, 2],
    &[
        ("facing", "south"),
        ("part", "middle"),
        ("slope", "horizontal"),
        ("casing", "false"),
    ],
);
const BELT_END: Block = belt(
    [1, 1, 3],
    &[
        ("facing", "south"),
        ("part", "end"),
        ("slope", "horizontal"),
        ("casing", "false"),
    ],
);
const BELT_MOTOR: Block = Block::new([0, 1, 1], "create:creative_motor", EAST).rotating(BASE_RPM);
const DEPOT_TOP: [f32; 3] = [0.5, 0.99, 0.5];

pub const SCENES: &[Scene] = &[
    Scene {
        title: "Relaying rotation with shafts",
        category: Rotation,
        items: &["create:shaft"],
        steps: &[
            Step::new(85, "A shaft carries rotation along its axis.", &[shaft([2, 1, 2], X, 0.0)]),
            Step::new(105, "Line shafts up end to end and connect them to a source.",
                &[MOTOR.rotating(BASE_RPM), shaft([1, 1, 2], X, BASE_RPM), shaft([2, 1, 2], X, BASE_RPM)]),
            Step::new(95, "Each shaft in the line turns at the same speed and in the same direction.",
                &[shaft([3, 1, 2], X, BASE_RPM), shaft([4, 1, 2], X, BASE_RPM)]),
        ],
    },
    Scene {
        title: "Meshing cogwheels",
        category: Rotation,
        items: &["create:cogwheel"],
        steps: &[
            Step::new(95, "Put the cogwheel's axle in line with a source of rotation.",
                &[Block::new([1, 1, 1], "create:creative_motor", SOUTH).rotating(BASE_RPM),
                  Block::new([1, 1, 2], "create:cogwheel", Z).rotating(BASE_RPM)]),
            Step::new(110, "Two small cogwheels mesh side by side. The second turns the other way at the same speed.",
                &[Block::new([2, 1, 2], "create:cogwheel", Z).rotating(-BASE_RPM),
                  shaft([2, 1, 3], Z, -BASE_RPM)]),
            Step::new(95, "A third cogwheel reverses the direction again.",
                &[Block::new([3, 1, 2], "create:cogwheel", Z).rotating(BASE_RPM),
                  shaft([3, 1, 3], Z, BASE_RPM)]),
        ],
    },
    Scene {
        title: "Changing rotation speed",
        category: Rotation,
        items: &["create:large_cogwheel", "create:cogwheel"],
        steps: &[
            Step::new(85, "Start with a powered large cogwheel.",
                &[Block::new([1, 2, 1], "create:creative_motor", SOUTH).rotating(BASE_RPM),
                  Block::new([1, 2, 2], "create:large_cogwheel", Z).rotating(BASE_RPM)]),
            Step::new(120, "A small cogwheel meshes diagonally with a large one on a parallel axle. It turns twice as fast in the opposite direction.",
                &[Block::new([2, 1, 2], "create:cogwheel", Z).rotating(-2.0 * BASE_RPM),
                  shaft([2, 1, 3], Z, -2.0 * BASE_RPM)]),
            Step::new(105, "Drive the small cogwheel instead to make the large wheel turn at half the input speed.",
                &[Block::new([1, 2, 1], "minecraft:air", &[]),
                  Block::new([2, 1, 1], "create:creative_motor", SOUTH).rotating(BASE_RPM),
                  Block::new([2, 1, 2], "create:cogwheel", Z).rotating(BASE_RPM),
                  shaft([2, 1, 3], Z, BASE_RPM),
                  Block::new([1, 2, 2], "create:large_cogwheel", Z).rotating(-0.5 * BASE_RPM)]),
        ],
    },
    Scene {
        title: "Turning corners with gearboxes",
        category: Rotation,
        items: &["create:gearbox"],
        steps: &[
            Step::new(85, "A horizontal gearbox has four connections. Drive it from any of its shaft faces.",
                &[MOTOR.rotating(BASE_RPM), shaft([1, 1, 2], X, BASE_RPM),
                  Block::new([2, 1, 2], "create:gearbox", Y).rotating(-BASE_RPM)]),
            Step::new(115, "Use a side connection to turn a corner. The shaft opposite the input reverses direction.",
                &[shaft([3, 1, 2], X, -BASE_RPM), shaft([2, 1, 1], Z, -BASE_RPM),
                  shaft([2, 1, 3], Z, BASE_RPM)]),
        ],
    },
    Scene {
        title: "Powering a line by hand",
        category: Rotation,
        items: &["create:hand_crank"],
        steps: &[
            Step::new(95, "A hand crank drives the shaft behind its handle.",
                &[CRANK.rotating(0.0), shaft([3, 1, 2], X, 0.0), shaft([2, 1, 2], X, 0.0)]),
            Step::new(90, "Use the handle to turn the shaft for a short time.",
                &[CRANK.rotating(BASE_RPM), shaft([3, 1, 2], X, BASE_RPM), shaft([2, 1, 2], X, BASE_RPM)]),
            Step::new(90, "Sneak while using it to reverse the rotation.",
                &[CRANK.rotating(-BASE_RPM), shaft([3, 1, 2], X, -BASE_RPM), shaft([2, 1, 2], X, -BASE_RPM)]),
        ],
    },
    Scene {
        title: "Using a Creative Motor",
        category: Rotation,
        items: &["create:creative_motor", "create:wrench"],
        steps: &[
            Step::new(105, "A Creative Motor supplies rotation through its single shaft. It starts at 16 RPM.",
                &[MOTOR.rotating(BASE_RPM), shaft([1, 1, 2], X, BASE_RPM), shaft([2, 1, 2], X, BASE_RPM)]),
            Step::new(125, "Use a wrench on the motor to step through its speed and direction settings.",
                &[MOTOR.rotating(32.0), shaft([1, 1, 2], X, 32.0), shaft([2, 1, 2], X, 32.0)]),
        ],
    },
    Scene {
        title: "Driving a Water Wheel",
        category: Rotation,
        items: &["create:water_wheel"],
        steps: &[
            Step::new(90, "Place the wheel with its axle lined up with the shaft you want to drive.",
                &[Block::new([2, 2, 2], "create:water_wheel", Z).rotating(0.0), shaft([2, 2, 3], Z, 0.0)]),
            Step::new(130, "Flowing water drives the wheel when it passes across the paddles. The blue blocks mark a possible water path.",
                &[Block::new([1, 2, 2], "minecraft:blue_stained_glass", &[]),
                  Block::new([1, 1, 2], "minecraft:blue_stained_glass", &[]),
                  Block::new([2, 2, 2], "create:water_wheel", Z).rotating(8.0), shaft([2, 2, 3], Z, 8.0)]),
        ],
    },
    Scene {
        title: "Connecting a mechanical belt",
        category: Transport,
        items: &["create:belt_connector"],
        steps: &[
            Step::new(115, "Place two parallel shafts at the same height. Leave a straight gap across their axes.",
                &[shaft([1, 1, 1], X, 0.0), shaft([1, 1, 3], X, 0.0)]),
            Step::new(115, "Use the Mechanical Belt on the first shaft, then on the second to connect them.",
                &[BELT_START.rotating(0.0), BELT_MIDDLE.rotating(0.0), BELT_END.rotating(0.0)]),
            Step::new(110, "Power a shaft at either end to run the belt.",
                &[BELT_MOTOR, BELT_START, BELT_MIDDLE, BELT_END]),
        ],
    },
    Scene {
        title: "Moving items on a belt",
        category: Transport,
        items: &["create:belt_connector", "create:depot"],
        steps: &[
            Step::new(105, "Items dropped onto a moving belt travel along it.",
                &[BELT_MOTOR, BELT_START, BELT_MIDDLE, BELT_END])
                .item("minecraft:iron_ingot", [1, 1, 1], [0.5, 0.9, 0.4], [0.5, 0.9, 2.5]),
            Step::new(115, "A depot at the end of a belt can receive the items it carries.",
                &[Block::new([1, 1, 4], "create:depot", &[])])
                .item("minecraft:iron_ingot", [1, 1, 3], [0.5, 0.9, 0.5], [0.5, 0.99, 1.5]),
        ],
    },
    Scene {
        title: "Placing items on a Depot",
        category: Transport,
        items: &["create:depot"],
        steps: &[
            Step::new(85, "A depot holds a stack in place for machines to work on.", &[DEPOT]),
            Step::new(100, "Right-click its top with a stack, or drop items onto it.", &[])
                .item("minecraft:iron_ingot", DEPOT.pos, [0.5, 1.8, 0.5], DEPOT_TOP),
            Step::new(95, "Right-click the top again to pick up the stack.", &[]),
        ],
    },
    Scene {
        title: "Pressing metal into sheets",
        category: Processing,
        items: &["create:mechanical_press", "create:depot"],
        steps: &[
            Step::new(110, "Mount a Mechanical Press two blocks above a depot. Power the press through its shaft.",
                &[DEPOT, PRESS.rotating(32.0),
                  Block::new([1, 3, 2], "create:creative_motor", EAST).rotating(32.0)]),
            Step::new(85, "Place an iron ingot on the depot. The press lowers its head to flatten it.", &[])
                .item("minecraft:iron_ingot", DEPOT.pos, DEPOT_TOP, DEPOT_TOP)
                .press(PRESS.pos),
            Step::new(110, "Pick up the iron sheet from the depot. A moving belt can also feed items under the press.", &[])
                .item("create:iron_sheet", DEPOT.pos, DEPOT_TOP, DEPOT_TOP),
        ],
    },
    Scene {
        title: "Mixing ingredients in a Basin",
        category: Processing,
        items: &["create:mechanical_mixer", "create:basin", "create:blaze_burner"],
        steps: &[
            Step::new(105, "Mount the mixer two blocks above a basin and power it with a shaft or cogwheel.",
                &[Block::new([2, 2, 2], "create:basin", &[]),
                  Block::new([2, 4, 2], "create:mechanical_mixer", &[]).rotating(64.0),
                  Block::new([1, 4, 2], "create:cogwheel", Y).rotating(-64.0)]),
            Step::new(115, "With enough speed, the mixer starts when the basin holds the ingredients for a mixing recipe.", &[])
                .item("minecraft:copper_ingot", [2, 2, 2], [0.5, 1.5, 0.5], [0.5, 0.5, 0.5]),
            Step::new(115, "Brass needs heat. Place a fueled Blaze Burner under the basin.",
                &[Block::new([2, 1, 2], "create:blaze_burner", &[("blaze", "kindled")]).rotating(0.0)])
                .item("create:brass_ingot", [2, 2, 2], [0.5, 0.6, 0.5], [0.5, 0.6, 0.5]),
        ],
    },
];

pub fn for_item(item: &str) -> Option<(usize, &'static str)> {
    SCENES
        .iter()
        .enumerate()
        .filter_map(|(index, scene)| {
            scene
                .items
                .iter()
                .position(|id| *id == item)
                .map(|slot| (slot, index, scene.items[slot]))
        })
        .min()
        .map(|(_, index, id)| (index, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_scenes_have_unique_titles_and_valid_storyboards() {
        let mut titles = std::collections::BTreeSet::new();
        for scene in SCENES {
            assert!(titles.insert(scene.title));
            assert!(!scene.items.is_empty());
            assert!(!scene.steps.is_empty());
            for (step, tick) in scene.steps.iter().zip(scene.markers()) {
                assert!(step.ticks > 0);
                assert!(!step.caption.is_empty());
                let frame = scene.frame(tick);
                if let Some(item) = &step.item {
                    assert!(frame.blocks.contains_key(&item.anchor));
                }
                if let Some(press) = step.press {
                    assert_eq!(frame.blocks[&press].0.state.0, "create:mechanical_press");
                }
            }
        }
    }

    #[test]
    fn tooltip_prefers_a_scene_about_the_hovered_item() {
        let (index, _) = for_item("create:depot").unwrap();
        assert_eq!(SCENES[index].items[0], "create:depot");
        assert_eq!(for_item("minecraft:stone"), None);
    }
}

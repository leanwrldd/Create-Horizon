//! Where the Train Hat sits on a mob's head: `CreateHatArmorLayer.render` and `TrainHatInfo` (content/equipment/hats/,
//! content/trains/schedule/hat/), as the chain of draw operations the layer replays. *When* a mob wears the hat is
//! `schedule_hand::wears_train_hat`; this is the geometry.
//!
//! Create walks the model's parts to the head (`translateAndRotate` of each), takes the head's first cube and lays the hat on it:
//! `translate(offset / 16, (cube.minY - cube.maxY + offset.y) / 16, offset.z / 16)`, `scale(max(width, depth) / 8 * info.scale)`,
//! `scale(1, -1, -1)`, `translate(0, -2.25 / 16, 0)`, `rotateX(-8.5)`, then the model `create:entity/train_hat`. Horizon's layers run
//! in the entity's own frame (feet at the origin, up, facing +z), so the model-space chain is put under
//! `scale(1, -1, -1)` and `translate(0, -1.501, 0)` first, as `LivingEntityRenderer` does and `render_living_pre`'s doc says.
//!
//! The head's place comes from the model, which Create reads off the live `ModelPart`s and a mod can't: a humanoid model's head is
//! given by Horizon (`PosedParts`); the other models here (the villagers and the illagers) have a head at the model's origin that
//! turns with the entity's head angles, as their `setupAnim` does. Every other mob is left without a hat (`mount_for` is `None`).
//! Types that have a `train_hat_info` file in Create's data (a cow, a pig, a witch, a piglin ...) aren't listed: their offsets
//! are read off model parts of their own.
//!
//! Unverified, to check in game: the head cubes (`-4, -8, -4, 8, 8, 8` humanoid, `-4, -10, -4, 8, 10, 8` villagers and illagers) are
//! the vanilla models', not read from Horizon's; the villagers' renderer scale of `0.9375`; the hat's yaw on a turned head.

pub use super::track_overlay::{Op, compose};
use super::vec::Vec3;

/// `LivingEntityRenderer`: the model's origin is 1.501 blocks above the feet.
pub const MODEL_ORIGIN: f64 = 1.501;
/// `TrainHatInfo`'s default (`new TrainHatInfo("", 0, Vec3.ZERO, 1.0F)`).
pub const DEFAULT_OFFSET: Vec3 = [0.0; 3];

/// Where a mob's head pose comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadSource {
    /// A humanoid model's `head` part as Horizon poses it (`PosedParts`).
    Posed,
    /// A head at the model's origin that turns with the entity's head angles (`LivingRenderState.y_rot`, `x_rot`).
    Free,
}

/// What the layer needs to know of a type's head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mount {
    pub source: HeadSource,
    /// The head's first cube: `minY`, `maxY` and the larger of its width and depth, in texels.
    pub min_y: f32,
    pub max_y: f32,
    pub width: f32,
    /// `TrainHatInfo.offset` (texels) and `scale`.
    pub offset: Vec3,
    pub scale: f32,
    /// The renderer's own scale of the whole model (`VillagerRenderer.scale`).
    pub renderer_scale: f32,
}

const HUMANOID: Mount =
    Mount { source: HeadSource::Posed, min_y: -8.0, max_y: 0.0, width: 8.0, offset: DEFAULT_OFFSET, scale: 1.0, renderer_scale: 1.0 };
const VILLAGER: Mount =
    Mount { source: HeadSource::Free, min_y: -10.0, max_y: 0.0, width: 8.0, offset: DEFAULT_OFFSET, scale: 1.0, renderer_scale: 0.9375 };
const ILLAGER: Mount = Mount { renderer_scale: 0.9375, ..VILLAGER };

/// The entity types that get a hat, with their head. The players of `register_entity_layers` are separate targets (their head
/// is the humanoid one and the player renderer scales the model by `0.9375`).
pub fn mount_for(entity_type: &str) -> Option<Mount> {
    Some(match entity_type {
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" | "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged"
        | "minecraft:wither_skeleton" => HUMANOID,
        "minecraft:villager" | "minecraft:wandering_trader" | "minecraft:zombie_villager" => VILLAGER,
        "minecraft:pillager" | "minecraft:vindicator" | "minecraft:evoker" | "minecraft:illusioner" => ILLAGER,
        _ => return None,
    })
}

/// The player's mount: a humanoid head under the player renderer's scale.
pub const PLAYER: Mount = Mount { renderer_scale: 0.9375, ..HUMANOID };

/// Every target `register_entity_layers` adds the hat layer to.
pub const TARGETS: [&str; 14] = [
    "minecraft:zombie",
    "minecraft:husk",
    "minecraft:drowned",
    "minecraft:skeleton",
    "minecraft:stray",
    "minecraft:bogged",
    "minecraft:wither_skeleton",
    "minecraft:villager",
    "minecraft:wandering_trader",
    "minecraft:zombie_villager",
    "minecraft:pillager",
    "minecraft:vindicator",
    "minecraft:evoker",
    "minecraft:illusioner",
];

/// What a head part's pose says (`ModelPart.translateAndRotate`): position in texels and the three angles in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeadPose {
    pub position: Vec3,
    pub angles: Vec3,
}

/// `rotationZYX(z, y, x)` as draw operations (angles in radians): the matrix is `Rz * Ry * Rx`, so the turns are taken z, y, x
/// (an earlier version took them x, y, z, which only differs once a head is both pitched and turned).
pub fn rotation_ops(angles: Vec3) -> Vec<Op> {
    let mut ops = Vec::new();
    for (axis, angle) in [([0.0, 0.0, 1.0], angles[2]), ([0.0, 1.0, 0.0], angles[1]), ([1.0, 0.0, 0.0], angles[0])] {
        if angle != 0.0 {
            ops.push(Op::Rotate(axis, angle.to_degrees() as f32));
        }
    }
    ops
}

/// The ops that put the hat model where Create's layer does, from the entity's frame. `entity_scale` is the state's
/// `scale * age_scale`.
pub fn hat_ops(mount: &Mount, head: &HeadPose, entity_scale: f32) -> Vec<Op> {
    let s = f64::from(entity_scale * mount.renderer_scale);
    let mut ops = vec![
        Op::Scale([s, s, s]),
        Op::Scale([1.0, -1.0, -1.0]),
        Op::Translate([0.0, -MODEL_ORIGIN, 0.0]),
        // `ModelPart.translateAndRotate`: `translate`, then `rotationZYX(z, y, x)`.
        Op::Translate([head.position[0] / 16.0, head.position[1] / 16.0, head.position[2] / 16.0]),
    ];
    ops.extend(rotation_ops(head.angles));
    let (min_y, max_y, off) = (f64::from(mount.min_y), f64::from(mount.max_y), mount.offset);
    ops.push(Op::Translate([off[0] / 16.0, (min_y - max_y + off[1]) / 16.0, off[2] / 16.0]));
    let size = f64::from(mount.width / 8.0 * mount.scale);
    ops.push(Op::Scale([size, size, size]));
    ops.push(Op::Scale([1.0, -1.0, -1.0]));
    ops.push(Op::Translate([0.0, -2.25 / 16.0, 0.0]));
    ops.push(Op::Rotate([1.0, 0.0, 0.0], -8.5));
    ops
}

/// The hat's colour: the layer draws it lit and untinted, with `disableDiffuse`.
pub const TINT: u32 = 0xFFFF_FFFF;
/// `AllPartialModels.TRAIN_HAT = entity("train_hat")`.
pub const MODEL: &str = "create:entity/train_hat";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zombie_s_hat_sits_on_top_of_its_head() {
        let mount = mount_for("minecraft:zombie").unwrap();
        // The head part at the humanoid origin, not turned: the model's origin is 1.501 up, the head is 8 texels tall above it.
        let ops = hat_ops(&mount, &HeadPose::default(), 1.0);
        let at = compose(&ops).apply([0.0; 3]);
        // The hat's foot is the head's top (1.501 + 0.5 blocks) sunk by 2.25 texels, and nothing sideways.
        assert!(at[0].abs() < 1e-9 && at[2].abs() < 1e-9, "{at:?}");
        assert!((at[1] - (1.501 + 0.5 - 2.25 / 16.0)).abs() < 1e-6, "{at:?}");
    }

    #[test]
    fn a_taller_head_carries_the_hat_higher_and_a_scale_shrinks_the_whole() {
        let villager = mount_for("minecraft:villager").unwrap();
        let tall = compose(&hat_ops(&villager, &HeadPose::default(), 1.0)).apply([0.0; 3]);
        assert!((tall[1] - (1.501 + 10.0 / 16.0 - 2.25 / 16.0) * 0.9375).abs() < 0.05, "{tall:?}");
        let baby = compose(&hat_ops(&villager, &HeadPose::default(), 0.5)).apply([0.0; 3]);
        assert!(baby[1] < tall[1]);
    }

    #[test]
    fn the_hat_s_model_stands_up_and_keeps_its_orientation() {
        let mount = mount_for("minecraft:zombie").unwrap();
        let a = compose(&hat_ops(&mount, &HeadPose::default(), 1.0));
        // A point three texels up the hat model is above its foot.
        assert!(a.apply([0.0, 3.0 / 16.0, 0.0])[1] > a.apply([0.0; 3])[1]);
        // The two flips (the model's frame and Create's `scale(1, -1, -1)`) cancel: the hat's +z is the mob's front.
        assert!(a.apply([0.0, 0.0, 4.0 / 16.0])[2] > a.apply([0.0, 0.0, -4.0 / 16.0])[2]);
    }

    #[test]
    fn a_turned_head_turns_the_hat() {
        let mount = mount_for("minecraft:villager").unwrap();
        let straight = compose(&hat_ops(&mount, &HeadPose::default(), 1.0));
        let turned = compose(&hat_ops(&mount, &HeadPose { position: [0.0; 3], angles: [0.0, std::f64::consts::FRAC_PI_2, 0.0] }, 1.0));
        let front = [0.0, 0.0, -0.25];
        let (a, b) = (straight.apply(front), turned.apply(front));
        assert!((a[0] - b[0]).abs() > 0.1 || (a[2] - b[2]).abs() > 0.1);
    }

    #[test]
    fn only_the_mobs_whose_heads_are_known_get_a_hat() {
        for kind in TARGETS {
            assert!(mount_for(kind).is_some(), "{kind}");
        }
        assert!(mount_for("minecraft:cow").is_none());
        assert!(mount_for("minecraft:witch").is_none());
        assert_eq!(PLAYER.source, HeadSource::Posed);
        assert_eq!(mount_for("minecraft:pillager").unwrap().renderer_scale, 0.9375);
    }
}

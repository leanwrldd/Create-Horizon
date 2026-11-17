//! The Sticker: a block that, powered, glues the block in front of it to a structure while it
//! moves (`StickerBlock`, `StickerBlockEntity`, `StickerRenderer`; the engine's side is
//! `contraption.rs`, `BlockMovementChecksImpl`). Each rise of the redstone signal toggles it.

use crate::Dir;

pub const ID: &str = "create:sticker";

/// `StickerBlock.POWERED` and `EXTENDED`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sticker {
    pub powered: bool,
    pub extended: bool,
}

impl Sticker {
    /// `StickerBlock.neighborChanged`: when the signal differs from `powered`, `powered` follows it,
    /// and a rise also toggles `extended`. `None` when nothing changes.
    pub fn power_update(self, signal: bool) -> Option<Sticker> {
        if self.powered == signal {
            return None;
        }
        let extended = if signal { !self.extended } else { self.extended };
        Some(Sticker { powered: signal, extended })
    }

    /// `getStateForPlacement`: powered by the signal it is placed in, not extended.
    pub fn placed(signal: bool) -> Sticker {
        Sticker { powered: signal, extended: false }
    }
}

/// `StickerBlockEntity.piston.chase(target, .4f, Chaser.LINEAR)`: the head's value `ticks` after
/// the state changed, from `from` towards `target` (0 or 1).
pub fn head_value(from: f32, target: f32, ticks: f32) -> f32 {
    let step = 0.4 * ticks.max(0.0);
    if target > from { (from + step).min(target) } else { (from - step).max(target) }
}

/// `StickerRenderer`: the head moves `value^2 * 4 / 16` blocks out of the block.
pub fn head_offset(value: f32) -> f32 {
    value * value * 4.0 / 16.0
}

/// `WrenchableDirectionalBlock.getRotatedBlockState`: the Wrench turns it about the face clicked.
pub fn wrenched(facing: Dir, clicked: Dir) -> Dir {
    if facing.axis() == clicked.axis() {
        facing.opposite()
    } else {
        crate::rules::drill::clockwise(facing, clicked.axis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rise_toggles_and_a_fall_only_unpowers() {
        let s = Sticker::placed(false);
        let up = s.power_update(true).expect("rises");
        assert_eq!(up, Sticker { powered: true, extended: true });
        assert_eq!(up.power_update(true), None);
        let down = up.power_update(false).expect("falls");
        assert_eq!(down, Sticker { powered: false, extended: true });
        let again = down.power_update(true).expect("rises");
        assert_eq!(again, Sticker { powered: true, extended: false });
    }

    #[test]
    fn placed_powered_it_is_not_extended() {
        assert_eq!(Sticker::placed(true), Sticker { powered: true, extended: false });
    }

    #[test]
    fn the_head_takes_three_ticks_to_come_out() {
        assert_eq!(head_value(0.0, 1.0, 0.0), 0.0);
        assert!((head_value(0.0, 1.0, 1.0) - 0.4).abs() < 1e-6);
        assert_eq!(head_value(0.0, 1.0, 3.0), 1.0);
        assert_eq!(head_value(1.0, 0.0, 3.0), 0.0);
        assert_eq!(head_offset(1.0), 0.25);
        assert_eq!(head_offset(0.5), 0.0625);
    }

    #[test]
    fn the_wrench_turns_it_about_the_face() {
        assert_eq!(wrenched(Dir::Up, Dir::Down), Dir::Down);
        assert_ne!(wrenched(Dir::North, Dir::Up), Dir::North);
    }
}

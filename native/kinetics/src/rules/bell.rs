//! The Peculiar Bell and the Haunted Bell: `PeculiarBellBlock.tryConvert` (a peculiar bell over soul fire
//! becomes a haunted one) and `HauntedBellPulser` / `SoulPulseEffect` (a player holding a haunted bell
//! sees the spots where a mob could spawn), with no game types.

pub const PECULIAR_BELL: &str = "create:peculiar_bell";
pub const HAUNTED_BELL: &str = "create:haunted_bell";
/// `HauntedBellPulser.DISTANCE`: how far the held bell's pulse reaches.
pub const DISTANCE: i32 = 3;
/// `HauntedBellPulser.RECHARGE_TICKS`: a pulse each 8 game ticks.
pub const RECHARGE_TICKS: i64 = 8;
/// `HauntedBellPulser.WARMUP_TICKS`: ticks of holding before the first pulse.
pub const WARMUP_TICKS: i32 = 10;

/// `PeculiarBellBlock.tryConvert`: the blocks that haunt a bell above them.
pub fn haunts(below: &str) -> bool {
    below == "minecraft:soul_fire" || below == "minecraft:soul_campfire"
}

/// `SoulPulseEffect.genLayers`: every offset within `distance` of the middle, by `round(sqrt(d²))`
/// (zero counts as 1, so the middle is in the first layer), the middle included.
pub fn pulse_offsets(distance: i32) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for x in -distance..=distance {
        for y in -distance..=distance {
            for z in -distance..=distance {
                let dist = (f64::from(x * x + y * y + z * z)).sqrt().round() as i32;
                if dist <= distance {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out
}

/// What the pulse says of a spot, `SoulPulseEffect.canSpawnSoulAt` and `isDark`: a mob (a zombie)
/// could stand there when the spot and the one above are free and the one below is solid; it is
/// dark when it has no block light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    No,
    Lit,
    Dark,
}

pub fn spot(free: bool, above_free: bool, below_solid: bool, block_light: u8) -> Spot {
    if !(free && above_free && below_solid) {
        Spot::No
    } else if block_light < 1 {
        Spot::Dark
    } else {
        Spot::Lit
    }
}

/// `HauntedBellPulser.hauntedBellCreatesPulse`: the warm-up before the first pulse, then (as the source
/// reads: `gameTime % RECHARGE_TICKS != 0`) a pulse on every game tick that is not a multiple of
/// `RECHARGE_TICKS`. Each pulse only adds the spots it hasn't shown, so the glue sends them as
/// often as it likes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pulser {
    ticks: i32,
}

impl Default for Pulser {
    fn default() -> Self {
        Self { ticks: WARMUP_TICKS }
    }
}

impl Pulser {
    /// One tick of holding a haunted bell: whether it pulses now.
    pub fn tick(&mut self, game_time: i64) -> bool {
        let first = self.ticks == 1;
        self.ticks -= 1;
        if self.ticks > 0 {
            return false;
        }
        self.ticks = 0;
        first || game_time % RECHARGE_TICKS != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_fire_and_a_soul_campfire_haunt_a_bell() {
        assert!(haunts("minecraft:soul_fire") && haunts("minecraft:soul_campfire"));
        assert!(!haunts("minecraft:campfire") && !haunts("minecraft:fire"));
    }

    #[test]
    fn the_pulse_reaches_a_ball_of_radius_three() {
        let offsets = pulse_offsets(DISTANCE);
        assert!(offsets.contains(&[0, 0, 0]));
        assert!(offsets.contains(&[3, 0, 0]) && offsets.contains(&[0, -3, 0]));
        // 2, 2, 2 is 3.46 away: round to 3, in; 3, 3, 0 is 4.2: out.
        assert!(offsets.contains(&[2, 2, 2]));
        assert!(!offsets.contains(&[3, 3, 0]));
        assert!(offsets.iter().all(|o| o.iter().all(|c| c.abs() <= 3)));
    }

    #[test]
    fn a_spot_needs_room_above_and_floor_below() {
        assert_eq!(spot(true, true, true, 0), Spot::Dark);
        assert_eq!(spot(true, true, true, 7), Spot::Lit);
        assert_eq!(spot(false, true, true, 0), Spot::No);
        assert_eq!(spot(true, false, true, 0), Spot::No);
        assert_eq!(spot(true, true, false, 0), Spot::No);
    }

    #[test]
    fn the_pulse_waits_ten_ticks_then_goes_on_the_ticks_that_are_not_multiples_of_eight() {
        let mut p = Pulser::default();
        let mut pulses = Vec::new();
        for t in 1..=40i64 {
            if p.tick(t) {
                pulses.push(t);
            }
        }
        // Nothing for the warm-up: the first pulse is the tenth tick, the first with no ticks left.
        assert_eq!(pulses[0], 10);
        // First pulse is the tick the warm-up ends; every later pulse is a tick that is not a multiple of 8.
        assert!(pulses.iter().skip(1).all(|t| t % 8 != 0), "{pulses:?}");
        assert!(!pulses.contains(&16) && !pulses.contains(&24) && pulses.contains(&17));
    }
}

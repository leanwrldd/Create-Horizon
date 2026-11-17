//! What the goggles say of a rotating block (`KineticBlockEntity`, `GeneratingKineticBlockEntity`,
//! `IRotate`, `SpeedGaugeBlockEntity`, `StressGaugeBlockEntity`, `PoweredShaftBlockEntity`):
//! its stress impact or capacity at the speed it turns, the speedometer's and stressometer's
//! readings, and the warnings anyone sees: "overstressed" and "speed requirement".
//!
//! `impact` and `capacity` are Create's default stress values (`CStress.setImpact`,
//! `CStress.setCapacity` in `AllBlocks` and `BuilderTransformers`): what a block costs or gives
//! per RPM, in su. No game types.
//!
//! Known differences from Create, for the README:
//! - A generator whose network is turned faster by another generator shows the capacity it would
//!   give at its own speed in Create (`stressBase *= generatedSpeed / speed`); here it shows the
//!   capacity at the network's speed, as the glue doesn't send what a generator makes.
//! - A belt's stress is Create's own, none (`CStress.setNoImpact`), so it shows no stats.

use super::gauges::{FAST_SPEED, MEDIUM_SPEED};
use super::goggles::{
    AQUA, Builder, DARK_GRAY, GOLD, GRAY, GREEN, LIGHT_PURPLE, Lang, Line, RED, YELLOW,
    cut_gray_and_white, progress_bar, push_for_goggles,
};

/// `Mth.equal`.
fn equal(a: f32, b: f32) -> bool {
    (b - a).abs() < 1.0e-5
}

/// `BlockStressValues.getImpact`: the stress a block adds per RPM, from `CStress.setImpact`.
pub fn impact(block: &str) -> f32 {
    match block.strip_prefix("create:").unwrap_or(block) {
        // `AllBlocks.ENCASED_FAN`, `MECHANICAL_CRAFTER`, `MECHANICAL_ARM`, `WEIGHTED_EJECTOR`.
        "encased_fan" | "mechanical_crafter" | "mechanical_arm" | "weighted_ejector" => 2.0,
        // `BuilderTransformers.cuckooClock`: `CStress.setImpact(1)`.
        "cuckoo_clock" | "mysterious_cuckoo_clock" => 1.0,
        // `AllBlocks.CHAIN_CONVEYOR`: `CStress.setImpact(1)`.
        "chain_conveyor" => 1.0,
        // `TURNTABLE`, `MILLSTONE`, `MECHANICAL_MIXER`, `MECHANICAL_PUMP`, `MECHANICAL_BEARING`,
        // `MECHANICAL_DRILL`, `MECHANICAL_SAW`, `DEPLOYER`, and `mechanicalPiston(type)`.
        "turntable" | "millstone" | "mechanical_mixer" | "mechanical_pump" | "mechanical_bearing" | "clockwork_bearing"
        | "mechanical_drill" | "mechanical_saw" | "deployer" | "mechanical_piston"
        | "sticky_mechanical_piston" => 4.0,
        // `ROPE_PULLEY`, `ELEVATOR_PULLEY` (the gantry sets none) and `HOSE_PULLEY`.
        "rope_pulley" | "elevator_pulley" | "hose_pulley" => 4.0,
        // `CRUSHING_WHEEL` and `MECHANICAL_PRESS`.
        "crushing_wheel" | "mechanical_press" => 8.0,
        _ => 0.0,
    }
}

/// `BlockStressValues.getCapacity`: the stress a generator adds per RPM, from `CStress.setCapacity`.
pub fn capacity(block: &str) -> f32 {
    match block.strip_prefix("create:").unwrap_or(block) {
        "creative_motor" => 16384.0,
        "water_wheel" => 32.0,
        "large_water_wheel" => 128.0,
        // `HAND_CRANK` and `COPPER_VALVE_HANDLE` (the dyed handles have none).
        "hand_crank" | "copper_valve_handle" => 8.0,
        "steam_engine" => 1024.0,
        // `WINDMILL_BEARING`.
        "windmill_bearing" => 512.0,
        _ => 0.0,
    }
}

/// `IRotate.getMinimumRequiredSpeedLevel`: the mixer and the display board want a medium speed.
pub fn minimum_speed(block: &str) -> f32 {
    match block.strip_prefix("create:").unwrap_or(block) {
        "mechanical_mixer" | "display_board" => MEDIUM_SPEED,
        _ => 0.0,
    }
}

/// `IRotate.SpeedLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedLevel {
    None,
    Slow,
    Medium,
    Fast,
}

impl SpeedLevel {
    /// `SpeedLevel.of`.
    pub fn of(speed: f32) -> Self {
        let speed = speed.abs();
        if speed >= FAST_SPEED {
            Self::Fast
        } else if speed >= MEDIUM_SPEED {
            Self::Medium
        } else if speed >= 1.0 {
            Self::Slow
        } else {
            Self::None
        }
    }

    pub fn text_color(self) -> u32 {
        match self {
            Self::None => DARK_GRAY,
            Self::Slow => GREEN,
            Self::Medium => AQUA,
            Self::Fast => LIGHT_PURPLE,
        }
    }

    /// `Lang.asId(name)`.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Slow => "slow",
            Self::Medium => "medium",
            Self::Fast => "fast",
        }
    }
}

/// `IRotate.StressImpact`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StressImpact {
    Low,
    Medium,
    High,
    Overstressed,
}

impl StressImpact {
    /// `StressImpact.of`.
    pub fn of(percent: f64) -> Self {
        if percent > 1.0 {
            Self::Overstressed
        } else if percent > 0.75 {
            Self::High
        } else if percent > 0.5 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    pub fn relative_color(self) -> u32 {
        match self {
            Self::Low => GREEN,
            Self::Medium => YELLOW,
            Self::High => GOLD,
            Self::Overstressed => RED,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Overstressed => "overstressed",
        }
    }

    fn ordinal(self) -> usize {
        self as usize
    }
}

/// `SpeedLevel.getFormattedSpeedText`: ` ██▒ Moderate (45 RPM) `, struck through and dark gray
/// when the network is overstressed.
pub fn speed_text(lang: &Lang, speed: f32, overstressed: bool) -> Builder {
    let level = SpeedLevel::of(speed);
    let mut builder = Builder::new()
        .text(progress_bar(3, level as usize))
        .tr(lang, &format!("tooltip.speedRequirement.{}", level.id()), &[])
        .space()
        .text("(")
        .number(f64::from(speed.abs()))
        .space()
        .tr(lang, "generic.unit.rpm", &[])
        .text(")")
        .space();
    if overstressed {
        builder = builder.style(DARK_GRAY).strikethrough();
    } else {
        builder = builder.style(level.text_color());
    }
    builder
}

/// `StressImpact.getFormattedStressText`: ` ███ High (80%) `.
pub fn stress_text(lang: &Lang, fraction: f64) -> Builder {
    let level = StressImpact::of(fraction);
    Builder::new()
        .text(progress_bar(3, (level.ordinal() + 1).min(3)))
        .tr(lang, &format!("tooltip.stressImpact.{}", level.id()), &[])
        .text(format!(" ({}%) ", (fraction * 100.0) as i64))
        .style(level.relative_color())
}

/// What the goggles need to know of a rotating block entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Kinetic {
    /// The block's id (`create:mechanical_mixer`).
    pub block: String,
    /// `getTheoreticalSpeed`: what the network turns it at, overstressed or not.
    pub theoretical: f32,
    /// `overStressed`.
    pub overstressed: bool,
}

impl Kinetic {
    /// `getSpeed`: stopped while overstressed.
    pub fn speed(&self) -> f32 {
        if self.overstressed { 0.0 } else { self.theoretical }
    }
}

/// `KineticBlockEntity.addStressImpactStats`: the impact's name and what it comes to.
pub fn stress_impact_stats(lang: &Lang, tooltip: &mut Vec<Line>, stress_at_base: f32, theoretical: f32) {
    Builder::new()
        .tr(lang, "tooltip.stressImpact", &[])
        .style(GRAY)
        .for_goggles(tooltip);
    let total = f64::from(stress_at_base) * f64::from(theoretical.abs());
    Builder::new()
        .number(total)
        .tr(lang, "generic.unit.stress", &[])
        .style(AQUA)
        .space()
        .add(Builder::new().tr(lang, "gui.goggles.at_current_speed", &[]).style(DARK_GRAY))
        .for_goggles_at(tooltip, 1);
}

/// `KineticBlockEntity.addToGoggleTooltip`: the stats of a block that costs stress.
pub fn kinetic_goggles(lang: &Lang, kinetic: &Kinetic, tooltip: &mut Vec<Line>) -> bool {
    let at_base = impact(&kinetic.block);
    if equal(at_base, 0.0) {
        return false;
    }
    Builder::new().tr(lang, "gui.goggles.kinetic_stats", &[]).for_goggles(tooltip);
    stress_impact_stats(lang, tooltip, at_base, kinetic.theoretical);
    true
}

/// `GeneratingKineticBlockEntity.addToGoggleTooltip` for a capacity of `capacity` su per RPM: the
/// stats of what the block costs, then of what it gives.
pub fn generating_goggles(
    lang: &Lang,
    kinetic: &Kinetic,
    capacity: f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    let added = kinetic_goggles(lang, kinetic, tooltip);
    if equal(capacity, 0.0) {
        return added;
    }
    Builder::new().tr(lang, "gui.goggles.generator_stats", &[]).for_goggles(tooltip);
    Builder::new().tr(lang, "tooltip.capacityProvided", &[]).style(GRAY).for_goggles(tooltip);
    let total = f64::from(capacity) * f64::from(kinetic.theoretical.abs());
    Builder::new()
        .number(total)
        .tr(lang, "generic.unit.stress", &[])
        .style(AQUA)
        .space()
        .add(Builder::new().tr(lang, "gui.goggles.at_current_speed", &[]).style(DARK_GRAY))
        .for_goggles_at(tooltip, 1);
    true
}

/// `KineticBlockEntity.addToTooltip`: the hint anyone sees, with or without goggles, when the
/// network is overstressed or the block turns too slowly for what it does.
pub fn kinetic_hover(
    lang: &Lang,
    kinetic: &Kinetic,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    let speed = kinetic.speed();
    let not_fast_enough = speed.abs() < minimum_speed(&kinetic.block) && speed != 0.0;
    if kinetic.overstressed {
        Builder::new().tr(lang, "gui.stressometer.overstressed", &[]).style(GOLD).for_goggles(tooltip);
        let hint = lang.tr("gui.contraptions.network_overstressed", &[]);
        push_for_goggles(tooltip, cut_gray_and_white(&hint, width));
        return true;
    }
    if not_fast_enough {
        Builder::new().tr(lang, "tooltip.speedRequirement", &[]).style(GOLD).for_goggles(tooltip);
        let name = lang.block_name(&kinetic.block);
        let hint = lang.tr("gui.contraptions.not_fast_enough", &[name]);
        push_for_goggles(tooltip, cut_gray_and_white(&hint, width));
        return true;
    }
    false
}

/// `GaugeBlockEntity.addToGoggleTooltip` and `SpeedGaugeBlockEntity.addToGoggleTooltip`.
pub fn speedometer_goggles(lang: &Lang, kinetic: &Kinetic, tooltip: &mut Vec<Line>) -> bool {
    Builder::new().tr(lang, "gui.gauge.info_header", &[]).for_goggles(tooltip);
    Builder::new().tr(lang, "gui.speedometer.title", &[]).style(GRAY).for_goggles(tooltip);
    speed_text(lang, kinetic.theoretical, kinetic.overstressed).for_goggles(tooltip);
    true
}

/// `StressGaugeBlockEntity.addToGoggleTooltip`: the network's `stress` and `capacity` in su.
pub fn stressometer_goggles(
    lang: &Lang,
    kinetic: &Kinetic,
    stress: f32,
    capacity: f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    Builder::new().tr(lang, "gui.gauge.info_header", &[]).for_goggles(tooltip);
    let capacity = f64::from(capacity);
    let stress = f64::from(stress);
    let fraction = stress / if capacity == 0.0 { 1.0 } else { capacity };
    Builder::new().tr(lang, "gui.stressometer.title", &[]).style(GRAY).for_goggles(tooltip);
    if kinetic.theoretical == 0.0 {
        Builder::new()
            .text(progress_bar(3, 0))
            .tr(lang, "gui.stressometer.no_rotation", &[])
            .style(DARK_GRAY)
            .for_goggles(tooltip);
    } else {
        stress_text(lang, fraction).for_goggles(tooltip);
        Builder::new().tr(lang, "gui.stressometer.capacity", &[]).style(GRAY).for_goggles(tooltip);
        let remaining = capacity - stress;
        let su = || Builder::new().tr(lang, "generic.unit.stress", &[]);
        let mut tip = Builder::new()
            .number(remaining)
            .add(su())
            .style(StressImpact::of(fraction).relative_color());
        if remaining != capacity {
            tip = tip
                .text_in(GRAY, " / ")
                .add(Builder::new().number(capacity).add(su()).style(DARK_GRAY));
        }
        tip.for_goggles_at(tooltip, 1);
    }
    true
}

/// `PoweredShaftBlockEntity.getGeneratedSpeed` for an engine share (`power`) and direction.
fn powered_generated(power: f32, direction: i32, engine_capacity: f32) -> f32 {
    super::steam_engine::generated_speed(direction, power, engine_capacity)
}

/// `PoweredShaftBlockEntity` (`GeneratingKineticBlockEntity.addToGoggleTooltip` with its own
/// `calculateAddedStressCapacity`): the capacity the shaft adds is the combined capacity of its
/// engine over its speed modifier. `power` and `direction` are the engine's share and way.
pub fn powered_shaft_goggles(
    lang: &Lang,
    kinetic: &Kinetic,
    power: f32,
    direction: i32,
    tooltip: &mut Vec<Line>,
) -> bool {
    let engine = capacity("steam_engine");
    let added = super::steam_engine::added_capacity(power, engine);
    // `stressBase *= generatedSpeed / speed` when the network turns it at another speed.
    let mut base = added;
    let generated = powered_generated(power, direction, engine);
    let speed = kinetic.theoretical;
    if speed != generated && speed != 0.0 && !equal(base, 0.0) {
        base *= generated / speed;
    }
    let mut kin = kinetic.clone();
    kin.block = "create:powered_shaft".to_owned();
    let added_stats = kinetic_goggles(lang, &kin, tooltip);
    if equal(added, 0.0) {
        return added_stats;
    }
    Builder::new().tr(lang, "gui.goggles.generator_stats", &[]).for_goggles(tooltip);
    Builder::new().tr(lang, "tooltip.capacityProvided", &[]).style(GRAY).for_goggles(tooltip);
    let total = (f64::from(base) * f64::from(speed)).abs();
    Builder::new()
        .number(total)
        .tr(lang, "generic.unit.stress", &[])
        .style(AQUA)
        .space()
        .add(Builder::new().tr(lang, "gui.goggles.at_current_speed", &[]).style(DARK_GRAY))
        .for_goggles_at(tooltip, 1);
    true
}

#[cfg(test)]
mod tests {
    use super::super::goggles::plain;
    use super::*;
    use std::collections::BTreeMap;

    /// Create's English strings for the keys used here.
    fn lang() -> Lang {
        let pairs = [
            ("create.gui.goggles.kinetic_stats", "Kinetic Stats:"),
            ("create.gui.goggles.generator_stats", "Generator Stats:"),
            ("create.gui.goggles.at_current_speed", "at current speed"),
            ("create.gui.gauge.info_header", "Gauge Information:"),
            ("create.gui.speedometer.title", "Rotation Speed"),
            ("create.gui.stressometer.title", "Network Stress"),
            ("create.gui.stressometer.capacity", "Remaining Capacity"),
            ("create.gui.stressometer.no_rotation", "No Rotation"),
            ("create.gui.stressometer.overstressed", "Overstressed"),
            ("create.tooltip.stressImpact", "Kinetic Stress Impact:"),
            ("create.tooltip.stressImpact.low", "Low"),
            ("create.tooltip.stressImpact.medium", "Moderate"),
            ("create.tooltip.stressImpact.high", "High"),
            ("create.tooltip.stressImpact.overstressed", "Overstressed"),
            ("create.tooltip.speedRequirement", "Speed Requirement:"),
            ("create.tooltip.speedRequirement.none", "None"),
            ("create.tooltip.speedRequirement.slow", "Slow"),
            ("create.tooltip.speedRequirement.medium", "Moderate"),
            ("create.tooltip.speedRequirement.fast", "Fast"),
            ("create.tooltip.capacityProvided", "Kinetic Stress Capacity:"),
            ("create.generic.unit.stress", "su"),
            ("create.generic.unit.rpm", "RPM"),
            (
                "create.gui.contraptions.network_overstressed",
                "It appears that this contraption is _overstressed_. Add more sources or _slow_ \
                 _down_ the components with a high _stress_ _impact_.",
            ),
            (
                "create.gui.contraptions.not_fast_enough",
                "It appears that this %1$s is _not_ rotating with _enough_ _speed_.",
            ),
            ("block.create.mechanical_mixer", "Mechanical Mixer"),
        ];
        Lang::new(pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<BTreeMap<_, _>>())
    }

    fn six(text: &str) -> f32 {
        // One pixel a character: no line is cut.
        text.chars().count() as f32
    }

    fn texts(tooltip: &[Line]) -> Vec<String> {
        tooltip.iter().map(|line| plain(line)).collect()
    }

    fn kinetic(block: &str, theoretical: f32, overstressed: bool) -> Kinetic {
        Kinetic { block: block.to_owned(), theoretical, overstressed }
    }

    #[test]
    fn stress_values_are_creates_defaults() {
        assert_eq!(impact("create:mechanical_press"), 8.0);
        assert_eq!(impact("create:crushing_wheel"), 8.0);
        assert_eq!(impact("create:mechanical_mixer"), 4.0);
        assert_eq!(impact("create:mechanical_crafter"), 2.0);
        assert_eq!(impact("create:cuckoo_clock"), 1.0);
        assert_eq!(impact("create:chain_conveyor"), 1.0);
        assert_eq!(impact("create:shaft"), 0.0);
        assert_eq!(impact("create:belt"), 0.0);
        assert_eq!(capacity("create:creative_motor"), 16384.0);
        assert_eq!(capacity("create:water_wheel"), 32.0);
        assert_eq!(capacity("create:large_water_wheel"), 128.0);
        assert_eq!(capacity("create:hand_crank"), 8.0);
        assert_eq!(capacity("create:steam_engine"), 1024.0);
        assert_eq!(capacity("create:white_valve_handle"), 0.0);
        assert_eq!(minimum_speed("create:mechanical_mixer"), 30.0);
        assert_eq!(minimum_speed("create:mechanical_press"), 0.0);
    }

    #[test]
    fn a_machine_shows_its_impact_at_the_speed_it_turns() {
        let mut tooltip = Vec::new();
        let kin = kinetic("create:mechanical_press", -32.0, false);
        assert!(kinetic_goggles(&lang(), &kin, &mut tooltip));
        assert_eq!(
            texts(&tooltip),
            ["    Kinetic Stats:", "    Kinetic Stress Impact:", "     256su at current speed"]
        );
        // The number and the unit are aqua, the rest dark gray.
        assert_eq!(tooltip[2][1].color, AQUA);
        assert_eq!(tooltip[2][4].color, DARK_GRAY);
    }

    #[test]
    fn a_block_that_costs_nothing_shows_no_stats() {
        let mut tooltip = Vec::new();
        assert!(!kinetic_goggles(&lang(), &kinetic("create:shaft", 64.0, false), &mut tooltip));
        assert!(tooltip.is_empty());
    }

    #[test]
    fn an_overstressed_block_counts_its_theoretical_speed() {
        let mut tooltip = Vec::new();
        kinetic_goggles(&lang(), &kinetic("create:mechanical_press", 64.0, true), &mut tooltip);
        assert_eq!(plain(&tooltip[2]), "     512su at current speed");
    }

    #[test]
    fn a_generator_shows_what_it_costs_and_what_it_gives() {
        let mut tooltip = Vec::new();
        let kin = kinetic("create:creative_motor", 16.0, false);
        assert!(generating_goggles(&lang(), &kin, 16384.0, &mut tooltip));
        assert_eq!(
            texts(&tooltip),
            [
                "    Generator Stats:",
                "    Kinetic Stress Capacity:",
                "     262,144su at current speed"
            ]
        );
        // A generator of no capacity shows only the costs.
        let mut tooltip = Vec::new();
        assert!(!generating_goggles(&lang(), &kinetic("create:shaft", 8.0, false), 0.0, &mut tooltip));
    }

    #[test]
    fn the_speedometer_shows_its_level_and_strikes_an_overstressed_one() {
        let mut tooltip = Vec::new();
        speedometer_goggles(&lang(), &kinetic("create:speedometer", 45.0, false), &mut tooltip);
        assert_eq!(
            texts(&tooltip),
            [
                "    Gauge Information:",
                "    Rotation Speed",
                "     \u{2588}\u{2588}\u{2592} Moderate (45 RPM) "
            ]
        );
        assert_eq!(tooltip[2][1].color, AQUA);
        let mut struck = Vec::new();
        speedometer_goggles(&lang(), &kinetic("create:speedometer", 45.0, true), &mut struck);
        assert!(struck[2].iter().skip(1).all(|seg| seg.strike && seg.color == DARK_GRAY));
        let mut fast = Vec::new();
        speedometer_goggles(&lang(), &kinetic("create:speedometer", -128.0, false), &mut fast);
        assert!(plain(&fast[2]).contains("Fast (128 RPM)"));
        assert_eq!(fast[2][1].color, LIGHT_PURPLE);
    }

    #[test]
    fn the_stressometer_shows_the_network_and_what_is_left() {
        let mut tooltip = Vec::new();
        let kin = kinetic("create:stressometer", 16.0, false);
        stressometer_goggles(&lang(), &kin, 4096.0, 16384.0, &mut tooltip);
        assert_eq!(
            texts(&tooltip),
            [
                "    Gauge Information:",
                "    Network Stress",
                "     \u{2588}\u{2592}\u{2592} Low (25%) ",
                "    Remaining Capacity",
                "     12,288su / 16,384su"
            ]
        );
        assert_eq!(tooltip[2][1].color, GREEN);
        assert_eq!(tooltip[4][1].color, GREEN);
        assert_eq!(tooltip[4][3].color, GRAY);
        assert_eq!(tooltip[4][4].color, DARK_GRAY);
        // Nothing used: the remaining capacity is the whole, shown once.
        let mut idle = Vec::new();
        stressometer_goggles(&lang(), &kin, 0.0, 16384.0, &mut idle);
        assert_eq!(plain(&idle[4]), "     16,384su");
        // Standing still: no rotation.
        let mut still = Vec::new();
        stressometer_goggles(&lang(), &kinetic("create:stressometer", 0.0, false), 0.0, 0.0, &mut still);
        assert_eq!(plain(&still[2]), "     \u{2592}\u{2592}\u{2592} No Rotation");
        assert_eq!(still.len(), 3);
    }

    #[test]
    fn the_stress_level_follows_the_fraction() {
        assert_eq!(StressImpact::of(0.5), StressImpact::Low);
        assert_eq!(StressImpact::of(0.51), StressImpact::Medium);
        assert_eq!(StressImpact::of(0.76), StressImpact::High);
        assert_eq!(StressImpact::of(1.0), StressImpact::High);
        assert_eq!(StressImpact::of(1.1), StressImpact::Overstressed);
        let mut tooltip = Vec::new();
        stressometer_goggles(
            &lang(),
            &kinetic("create:stressometer", 16.0, true),
            20000.0,
            16384.0,
            &mut tooltip,
        );
        assert_eq!(plain(&tooltip[2]), "     \u{2588}\u{2588}\u{2588} Overstressed (122%) ");
        assert_eq!(tooltip[2][1].color, RED);
        // Over capacity: nothing remains, it reads negative as Create's does.
        assert_eq!(plain(&tooltip[4]), "     -3,616su / 16,384su");
    }

    #[test]
    fn speed_levels_start_at_1_30_and_100_rpm() {
        assert_eq!(SpeedLevel::of(0.5), SpeedLevel::None);
        assert_eq!(SpeedLevel::of(1.0), SpeedLevel::Slow);
        assert_eq!(SpeedLevel::of(29.9), SpeedLevel::Slow);
        assert_eq!(SpeedLevel::of(-30.0), SpeedLevel::Medium);
        assert_eq!(SpeedLevel::of(100.0), SpeedLevel::Fast);
    }

    #[test]
    fn an_overstressed_block_warns_with_or_without_goggles() {
        let mut tooltip = Vec::new();
        let kin = kinetic("create:mechanical_press", 64.0, true);
        assert!(kinetic_hover(&lang(), &kin, &six, &mut tooltip));
        assert_eq!(plain(&tooltip[0]), "    Overstressed");
        assert_eq!(tooltip[0][1].color, GOLD);
        assert_eq!(tooltip.len(), 2);
        let hint: String = tooltip[1..].iter().map(|l| plain(l).trim_start().to_owned() + " ").collect();
        assert!(hint.starts_with("It appears that this contraption is overstressed."));
    }

    #[test]
    fn a_mixer_that_turns_too_slowly_says_so_by_name() {
        let mut tooltip = Vec::new();
        assert!(kinetic_hover(&lang(), &kinetic("create:mechanical_mixer", 16.0, false), &six, &mut tooltip));
        assert_eq!(plain(&tooltip[0]), "    Speed Requirement:");
        let hint: String = tooltip[1..].iter().map(|l| plain(l).trim_start().to_owned() + " ").collect();
        assert!(hint.contains("this Mechanical Mixer is not rotating"));
        // Fast enough, standing still and an unfussy block say nothing.
        let mut none = Vec::new();
        assert!(!kinetic_hover(&lang(), &kinetic("create:mechanical_mixer", 32.0, false), &six, &mut none));
        assert!(!kinetic_hover(&lang(), &kinetic("create:mechanical_mixer", 0.0, false), &six, &mut none));
        assert!(!kinetic_hover(&lang(), &kinetic("create:mechanical_press", 1.0, false), &six, &mut none));
        assert!(none.is_empty());
    }

    #[test]
    fn a_powered_shaft_gives_the_engines_capacity() {
        // One engine at full power: 1024 su per RPM over a modifier of 4, at 64 RPM.
        let mut tooltip = Vec::new();
        let kin = kinetic("create:powered_shaft", 64.0, false);
        assert!(powered_shaft_goggles(&lang(), &kin, 1.0, 1, &mut tooltip));
        assert_eq!(
            texts(&tooltip),
            [
                "    Generator Stats:",
                "    Kinetic Stress Capacity:",
                "     16,384su at current speed"
            ]
        );
        // No engine: nothing to say.
        let mut none = Vec::new();
        assert!(!powered_shaft_goggles(&lang(), &kinetic("create:powered_shaft", 0.0, false), 0.0, 0, &mut none));
    }
}

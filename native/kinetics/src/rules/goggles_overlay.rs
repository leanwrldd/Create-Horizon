//! Which lines the goggle overlay shows for the block under the crosshair
//! (`GoggleOverlayRenderer.renderOverlay`): the goggle tooltip when goggles are worn, the hovering
//! tooltip always, a blank line between them, nothing when neither has anything to say. The block
//! is described by a [`Subject`] of plain values the glue reads from the client's block entity.
//! No game types.
//!
//! Known differences from Create, for the README:
//! - Only the blocks this port has are subjects. Create's `IHaveHoveringInformation` of the
//!   Funnel adds nothing (its `addToTooltip` is the default), so the Funnel is not one.
//! - `IHaveCustomOverlayIcon` isn't used: the icon is always the goggles.

use super::goggles::{Lang, Line, blank};
use super::goggles_blocks::{
    FluidTank, analog_lever_goggles, arm_hover, assembly_exception, basin_goggles,
    bearing_hover, boiler_goggles, brass_tunnel_goggles, chute_goggles, contained_fluid,
    deployer_goggles, deployer_hover, hose_pulley_goggles, pole_length, whistle_goggles,
};
use super::goggles_kinetic::{
    Kinetic, capacity, generating_goggles, kinetic_goggles, kinetic_hover, powered_shaft_goggles,
    speedometer_goggles, stress_impact_stats, stressometer_goggles, impact,
};
use super::steam_engine::Boiler;

/// What a Steam Engine's shaft says (`PoweredShaftBlockEntity`): the shaft's rotation and the
/// engine's share of the boiler and way.
#[derive(Clone, Debug, PartialEq)]
pub struct PoweredShaft {
    pub kinetic: Kinetic,
    pub power: f32,
    pub direction: i32,
}

/// The block entity under the crosshair, as the goggles need it.
#[derive(Clone, Debug, PartialEq)]
pub enum Special {
    /// A rotating block with nothing more to say: its stress impact.
    Plain,
    /// A generator (Creative Motor, wheels, Hand Crank, valve handle): impact and capacity.
    Generator,
    Speedometer,
    Stressometer { stress: f32, capacity: f32 },
    PoweredShaft { power: f32, direction: i32 },
    /// A Steam Engine, with the shaft it turns if it is there.
    SteamEngine(Option<PoweredShaft>),
    /// A Fluid Tank: its controller's boiler and size (`width * width * height`) and fluid.
    Tank { boiler: Boiler, size: i32, tank: FluidTank, creative: bool },
    /// A Spout or an Item Drain.
    FluidContainer(FluidTank),
    Basin { items: Vec<(String, i32)>, fluids: Vec<(String, i32)> },
    Chute { pull: f32, push: f32, downward: bool, item: Option<(String, i32)> },
    Deployer { using: bool, held: Option<(String, i32)>, overflow: bool },
    AnalogLever(i32),
    Whistle(i32),
    BrassTunnel(Vec<(String, i32)>),
    Arm { has_targets: bool },
    Bearing { running: bool, front_replaceable: bool, windmill: bool },
    /// A Hose Pulley, and whether the body of fluid it works on is infinite.
    HosePulley { infinite: bool },
    /// A Piston Extension Pole: the poles in its line, if a Mechanical Piston is at either end.
    Pole(Option<usize>),
}

/// What is under the crosshair.
#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    /// The rotation of a `KineticBlockEntity`, for the blocks that are one.
    pub kinetic: Option<Kinetic>,
    pub special: Special,
    /// `IDisplayAssemblyExceptions.getLastAssemblyException`'s text, for a bearing or piston.
    pub exception: Option<String>,
}

impl Subject {
    pub fn plain(special: Special) -> Self {
        Self { kinetic: None, special, exception: None }
    }
}

fn has_goggle_information(subject: &Subject) -> bool {
    !matches!(subject.special, Special::Pole(_))
}

fn has_hovering_information(subject: &Subject) -> bool {
    subject.kinetic.is_some()
}

/// `addToGoggleTooltip` of the block.
fn goggle_tooltip(
    lang: &Lang,
    subject: &Subject,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    let kinetic = subject.kinetic.as_ref();
    match (&subject.special, kinetic) {
        (Special::Plain, Some(k)) => kinetic_goggles(lang, k, tooltip),
        (Special::Generator, Some(k)) => generating_goggles(lang, k, capacity(&k.block), tooltip),
        (Special::Speedometer, Some(k)) => speedometer_goggles(lang, k, tooltip),
        (Special::Stressometer { stress, capacity }, Some(k)) => {
            stressometer_goggles(lang, k, *stress, *capacity, tooltip)
        }
        (Special::PoweredShaft { power, direction }, Some(k)) => {
            powered_shaft_goggles(lang, k, *power, *direction, tooltip)
        }
        // `SteamEngineBlockEntity.addToGoggleTooltip`: what its shaft says.
        (Special::SteamEngine(Some(shaft)), _) => {
            powered_shaft_goggles(lang, &shaft.kinetic, shaft.power, shaft.direction, tooltip)
        }
        (Special::SteamEngine(None), _) => false,
        // `CreativeFluidTankBlockEntity.addToGoggleTooltip` says nothing; the others ask the
        // controller's boiler first, then its fluid.
        (Special::Tank { creative: true, .. }, _) => false,
        (Special::Tank { boiler, size, tank, .. }, _) => {
            boiler_goggles(lang, boiler, *size, tooltip)
                || contained_fluid(lang, std::slice::from_ref(tank), tooltip)
        }
        (Special::FluidContainer(tank), _) => contained_fluid(lang, std::slice::from_ref(tank), tooltip),
        (Special::Basin { items, fluids }, _) => basin_goggles(lang, items, fluids, tooltip),
        (Special::Chute { pull, push, downward, item }, _) => chute_goggles(
            lang,
            *pull,
            *push,
            *downward,
            item.as_ref().map(|(id, count)| (id.as_str(), *count)),
            tooltip,
        ),
        (Special::Deployer { using, held, .. }, Some(k)) => {
            deployer_goggles(
                lang,
                *using,
                held.as_ref().map(|(id, count)| (id.as_str(), *count)),
                tooltip,
            );
            let at_base = impact(&k.block);
            if at_base.abs() >= 1.0e-5 {
                tooltip.push(blank());
                stress_impact_stats(lang, tooltip, at_base, k.theoretical);
            }
            true
        }
        (Special::AnalogLever(state), _) => analog_lever_goggles(lang, *state, tooltip),
        (Special::Whistle(pitch), _) => whistle_goggles(lang, *pitch, tooltip),
        (Special::BrassTunnel(stacks), _) => brass_tunnel_goggles(lang, stacks, tooltip),
        // The arm, the bearing and any other rotating block have the plain `KineticBlockEntity`.
        (Special::Arm { .. } | Special::Bearing { .. }, Some(k)) => kinetic_goggles(lang, k, tooltip),
        (Special::HosePulley { infinite }, Some(k)) => hose_pulley_goggles(lang, k, *infinite, width, tooltip),
        _ => false,
    }
}

/// `addToTooltip` of the block.
fn hovering_tooltip(
    lang: &Lang,
    subject: &Subject,
    sneaking: bool,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    let Some(kinetic) = subject.kinetic.as_ref() else {
        return false;
    };
    if kinetic_hover(lang, kinetic, width, tooltip) {
        return true;
    }
    match &subject.special {
        Special::Deployer { overflow, .. } => {
            deployer_hover(lang, kinetic.speed(), *overflow, width, tooltip)
        }
        Special::Arm { has_targets } => arm_hover(lang, sneaking, *has_targets, width, tooltip),
        Special::Bearing { running, front_replaceable, windmill } => bearing_hover(
            lang,
            sneaking,
            *windmill || kinetic.speed() != 0.0,
            *running,
            *front_replaceable,
            width,
            tooltip,
        ),
        _ => false,
    }
}

/// `GoggleOverlayRenderer.renderOverlay`: the lines of the tooltip, none when there is nothing
/// to show. `width` measures a string in GUI pixels.
pub fn overlay(
    lang: &Lang,
    subject: &Subject,
    wearing_goggles: bool,
    sneaking: bool,
    width: &dyn Fn(&str) -> f32,
) -> Vec<Line> {
    let mut tooltip: Vec<Line> = Vec::new();
    let has_goggle = has_goggle_information(subject);
    let mut has_hover = has_hovering_information(subject);
    let mut goggle_added = false;
    let mut hover_added = false;

    if has_goggle && wearing_goggles {
        goggle_added = goggle_tooltip(lang, subject, width, &mut tooltip);
    }
    if has_hover {
        if !tooltip.is_empty() {
            tooltip.push(blank());
        }
        hover_added = hovering_tooltip(lang, subject, sneaking, width, &mut tooltip);
        if goggle_added && !hover_added {
            tooltip.pop();
        }
    }
    if let Some(text) = &subject.exception {
        if assembly_exception(lang, text, width, &mut tooltip) {
            has_hover = true;
            hover_added = true;
        }
    }
    // Both asked and both answered nothing.
    if (has_goggle && !goggle_added) && (has_hover && !hover_added) {
        return Vec::new();
    }
    // A pole's length shows to those who wear goggles, when a piston is at its end.
    if let Special::Pole(poles) = &subject.special {
        if !wearing_goggles {
            return Vec::new();
        }
        let Some(poles) = poles else {
            return Vec::new();
        };
        pole_length(lang, *poles, &mut tooltip);
    }
    tooltip
}

#[cfg(test)]
mod tests {
    use super::super::goggles::plain;
    use super::*;
    use std::collections::BTreeMap;

    fn lang() -> Lang {
        let pairs = [
            ("create.gui.goggles.kinetic_stats", "Kinetic Stats:"),
            ("create.gui.goggles.generator_stats", "Generator Stats:"),
            ("create.gui.goggles.at_current_speed", "at current speed"),
            ("create.gui.goggles.pole_length", "Pole Length:"),
            ("create.gui.assembly.exception", "This Contraption was unable to assemble:"),
            ("create.gui.stressometer.overstressed", "Overstressed"),
            ("create.tooltip.stressImpact", "Kinetic Stress Impact:"),
            ("create.tooltip.capacityProvided", "Kinetic Stress Capacity:"),
            ("create.generic.unit.stress", "su"),
            ("create.gui.contraptions.network_overstressed", "Add more sources."),
            ("create.tooltip.analogStrength", "Analog Strength: %1$s/15"),
            ("create.hint.mechanical_arm_no_targets", "No targets."),
            ("create.hint.mechanical_arm_no_targets.title", "No Targets"),
            ("create.tooltip.deployer.header", "Deployer Information"),
            ("create.tooltip.deployer.using", "Mode: Use"),
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

    fn press(theoretical: f32, overstressed: bool) -> Subject {
        Subject {
            kinetic: Some(Kinetic {
                block: "create:mechanical_press".to_owned(),
                theoretical,
                overstressed,
            }),
            special: Special::Plain,
            exception: None,
        }
    }

    #[test]
    fn goggles_show_the_stats_and_no_goggles_show_nothing() {
        let subject = press(16.0, false);
        let with = overlay(&lang(), &subject, true, false, &six);
        assert_eq!(
            texts(&with),
            ["    Kinetic Stats:", "    Kinetic Stress Impact:", "     128su at current speed"]
        );
        assert!(overlay(&lang(), &subject, false, false, &six).is_empty());
    }

    #[test]
    fn an_overstressed_block_warns_without_goggles_and_after_the_stats_with_them() {
        let subject = press(16.0, true);
        let without = overlay(&lang(), &subject, false, false, &six);
        assert_eq!(texts(&without)[0], "    Overstressed");
        let with = overlay(&lang(), &subject, true, false, &six);
        // Stats, a blank line, then the warning.
        assert_eq!(texts(&with)[0], "    Kinetic Stats:");
        assert_eq!(texts(&with)[3], "");
        assert_eq!(texts(&with)[4], "    Overstressed");
    }

    #[test]
    fn a_lever_shows_its_strength_only_with_goggles() {
        let subject = Subject::plain(Special::AnalogLever(9));
        assert_eq!(texts(&overlay(&lang(), &subject, true, false, &six)), ["    Analog Strength: 9/15"]);
        assert!(overlay(&lang(), &subject, false, false, &six).is_empty());
    }

    #[test]
    fn a_generator_shows_its_capacity() {
        let subject = Subject {
            kinetic: Some(Kinetic {
                block: "create:creative_motor".to_owned(),
                theoretical: 32.0,
                overstressed: false,
            }),
            special: Special::Generator,
            exception: None,
        };
        let lines = texts(&overlay(&lang(), &subject, true, false, &six));
        assert_eq!(lines[0], "    Generator Stats:");
        assert_eq!(lines[2], "     524,288su at current speed");
    }

    #[test]
    fn a_creative_tank_shows_nothing() {
        let tank = FluidTank { fluid: Some("minecraft:water".into()), amount: 8000, capacity: 8000 };
        let subject = Subject::plain(Special::Tank {
            boiler: Boiler::default(),
            size: 1,
            tank,
            creative: true,
        });
        assert!(overlay(&lang(), &subject, true, false, &six).is_empty());
    }

    #[test]
    fn a_deployer_adds_its_stress_after_a_blank_line() {
        let subject = Subject {
            kinetic: Some(Kinetic {
                block: "create:deployer".to_owned(),
                theoretical: 8.0,
                overstressed: false,
            }),
            special: Special::Deployer { using: true, held: None, overflow: false },
            exception: None,
        };
        let lines = texts(&overlay(&lang(), &subject, true, false, &six));
        assert_eq!(lines[0], "    Deployer Information");
        assert_eq!(lines[2], "");
        assert_eq!(lines[3], "    Kinetic Stress Impact:");
        assert_eq!(lines[4], "     32su at current speed");
    }

    #[test]
    fn an_arm_with_no_targets_hints_even_without_goggles_unless_sneaking() {
        let subject = Subject {
            kinetic: Some(Kinetic { block: "create:mechanical_arm".to_owned(), theoretical: 0.0, overstressed: false }),
            special: Special::Arm { has_targets: false },
            exception: None,
        };
        assert_eq!(texts(&overlay(&lang(), &subject, false, false, &six))[0], "    No Targets");
        assert!(overlay(&lang(), &subject, false, true, &six).is_empty());
    }

    #[test]
    fn an_assembly_exception_shows_with_or_without_goggles() {
        let subject = Subject {
            kinetic: Some(Kinetic { block: "create:mechanical_piston".to_owned(), theoretical: 0.0, overstressed: false }),
            special: Special::Bearing { running: true, front_replaceable: false, windmill: false },
            exception: Some("The Piston is missing some extension Poles".to_owned()),
        };
        let lines = texts(&overlay(&lang(), &subject, false, false, &six));
        assert_eq!(lines[0], "    This Contraption was unable to assemble:");
        assert_eq!(lines[1], "    The Piston is missing some extension Poles");
    }

    #[test]
    fn a_pole_shows_its_length_to_goggles_when_a_piston_is_at_its_end() {
        let pole = |poles| Subject::plain(Special::Pole(poles));
        assert_eq!(texts(&overlay(&lang(), &pole(Some(3)), true, false, &six)), ["    Pole Length: 3"]);
        assert!(overlay(&lang(), &pole(Some(3)), false, false, &six).is_empty());
        assert!(overlay(&lang(), &pole(None), true, false, &six).is_empty());
    }
}

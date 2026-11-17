//! What the goggles say of the blocks that aren't just rotating: fluid containers
//! (`IHaveGoggleInformation.containedFluidTooltip`, `BoilerData.addToGoggleTooltip`), the Basin,
//! Chute, Deployer, Analog Lever, Steam Whistle, the Brass Tunnel, the hints under a Mechanical
//! Arm, Bearing and Deployer (`TooltipHelper.addHint`), an assembly exception
//! (`IDisplayAssemblyExceptions`) and a Piston Extension Pole's length. No game types.
//!
//! Known differences from Create, for the README:
//! - The Basin lists what the client is told it holds: every item slot and both tanks.
//! - A Fluid Pipe shows nothing, as in Create (`FluidPipeBlockEntity` has no goggle tooltip).

use super::goggles::{
    AQUA, BLUE, Builder, DARK_GRAY, DARK_GREEN, DARK_RED, GOLD, GRAY, GREEN, Lang, Line, WHITE,
    YELLOW, blank, cut_gray_and_white, push_for_goggles,
};
use super::steam_engine::{Boiler, ENGINE_CAPACITY, MAX_LEVEL, WATER_SUPPLY_PER_LEVEL};

/// A fluid tank as a fluid handler shows it: the fluid (`minecraft:water`), the millibuckets and
/// the capacity. An empty tank has no fluid.
#[derive(Clone, Debug, PartialEq)]
pub struct FluidTank {
    pub fluid: Option<String>,
    pub amount: i32,
    pub capacity: i32,
}

fn mb(lang: &Lang) -> Builder {
    Builder::new().tr(lang, "generic.unit.millibuckets", &[])
}

/// `IHaveGoggleInformation.containedFluidTooltip` for a handler of `tanks`.
pub fn contained_fluid(lang: &Lang, tanks: &[FluidTank], tooltip: &mut Vec<Line>) -> bool {
    if tanks.is_empty() {
        return false;
    }
    Builder::new().tr(lang, "gui.goggles.fluid_container", &[]).for_goggles(tooltip);
    let header = tooltip.len() - 1;
    let mut is_empty = true;
    for tank in tanks {
        let Some(fluid) = tank.fluid.as_deref().filter(|_| tank.amount > 0) else {
            continue;
        };
        Builder::new()
            .text(lang.fluid_name(fluid))
            .style(GRAY)
            .for_goggles_at(tooltip, 1);
        Builder::new()
            .add(Builder::new().number(f64::from(tank.amount)).add(mb(lang)).style(GOLD))
            .text_in(GRAY, " / ")
            .add(Builder::new().number(f64::from(tank.capacity)).add(mb(lang)).style(DARK_GRAY))
            .for_goggles_at(tooltip, 1);
        is_empty = false;
    }
    if tanks.len() > 1 {
        if is_empty {
            tooltip.truncate(header);
        }
        return true;
    }
    if !is_empty {
        return true;
    }
    Builder::new()
        .tr(lang, "gui.goggles.fluid_container.capacity", &[])
        .add(Builder::new().number(f64::from(tanks[0].capacity)).add(mb(lang)).style(GOLD))
        .style(GRAY)
        .for_goggles_at(tooltip, 1);
    true
}

// --- the boiler ---------------------------------------------------------------------------------

/// The levels `BoilerData.calcMinMaxForSize` works out for the bars.
struct Levels {
    for_size: i32,
    for_water: i32,
    min: i32,
    max: i32,
}

fn levels(boiler: &Boiler, size: i32) -> Levels {
    let for_size = Boiler::max_level_for_size(size);
    let for_water = boiler.max_level_for_water();
    let heat = if boiler.passive_heat { 1 } else { boiler.active_heat };
    Levels {
        for_size,
        for_water,
        min: heat.min(for_water.min(for_size)),
        max: heat.max(for_water.max(for_size)),
    }
}

/// `BoilerData.barComponent`: one `|` for each level, green up to what is reached, red for what
/// the lowest of heat, size and water holds back, gray for the rest of the scale.
fn bars(level: i32, levels: &Levels) -> Builder {
    let run = |count: i32, color: u32| {
        Builder::new().text_in(color, "|".repeat(count.max(0) as usize))
    };
    let (min, max) = (levels.min, levels.max);
    Builder::new()
        .add(run((min - 1).max(0), DARK_GREEN))
        .add(run(i32::from(min > 0), GREEN))
        .add(run((level - min).max(0), DARK_GREEN))
        .add(run((max - level).max(0), DARK_RED))
        .add(run((MAX_LEVEL - max).min((max / 5 + 1) * 5 - max).max(0), DARK_GRAY))
}

/// `BoilerData.componentHelper` for the goggles: the label, its dots and the bars.
fn bar_line(lang: &Lang, label: &str, level: i32, levels: &Levels) -> Builder {
    Builder::new()
        .add(Builder::new().tr(lang, &format!("boiler.{label}"), &[]).style(GRAY))
        .add(Builder::new().tr(lang, &format!("boiler.{label}_dots"), &[]).style(DARK_GRAY))
        .add(bars(level, levels))
}

/// `BoilerData.getHeatLevelTextComponent`.
fn heat_level_text(lang: &Lang, boiler: &Boiler, size: i32) -> String {
    let l = levels(boiler, size);
    let level = boiler.active_heat.min(l.for_water.min(l.for_size));
    if boiler.is_passive(size) {
        lang.tr("boiler.passive", &[])
    } else if level == 0 {
        lang.tr("boiler.idle", &[])
    } else if level == MAX_LEVEL {
        lang.tr("boiler.max_lvl", &[])
    } else {
        lang.tr("boiler.lvl", &[level.to_string()])
    }
}

/// `BoilerData.addToGoggleTooltip` for a tank of `size` blocks (`width * width * height`).
pub fn boiler_goggles(lang: &Lang, boiler: &Boiler, size: i32, tooltip: &mut Vec<Line>) -> bool {
    if !boiler.is_active() {
        return false;
    }
    let l = levels(boiler, size);
    Builder::new()
        .tr_styled(lang, "boiler.status", &[(heat_level_text(lang, boiler, size), Some(GREEN))])
        .for_goggles(tooltip);
    bar_line(lang, "size", l.for_size, &l).for_goggles_at(tooltip, 1);
    bar_line(lang, "water", l.for_water, &l).for_goggles_at(tooltip, 1);
    let heat = if boiler.passive_heat { 1 } else { boiler.active_heat };
    bar_line(lang, "heat", heat, &l).for_goggles_at(tooltip, 1);

    if boiler.engines == 0 {
        return true;
    }
    let boiler_level = boiler.active_heat.min(l.for_water.min(l.for_size));
    let total = f64::from(boiler.engine_efficiency(size))
        * 16.0
        * f64::from(boiler_level.max(boiler.engines))
        * f64::from(ENGINE_CAPACITY);
    tooltip.push(blank());
    if boiler.engines > 0 && l.for_size > 0 && l.for_water == 0 && heat > 0 {
        Builder::new().tr(lang, "boiler.water_input_rate", &[]).style(GRAY).for_goggles(tooltip);
        Builder::new()
            .number(f64::from(boiler.water_supply))
            .style(BLUE)
            .add(mb(lang))
            .add(Builder::new().text(" / ").style(GRAY))
            .add(
                Builder::new()
                    .tr(
                        lang,
                        "boiler.per_tick",
                        &[format!(
                            "{}{}",
                            super::goggles::number(f64::from(WATER_SUPPLY_PER_LEVEL)),
                            lang.tr("generic.unit.millibuckets", &[])
                        )],
                    )
                    .style(DARK_GRAY),
            )
            .for_goggles_at(tooltip, 1);
        return true;
    }
    Builder::new().tr(lang, "tooltip.capacityProvided", &[]).style(GRAY).for_goggles(tooltip);
    let via = if boiler.engines == 1 {
        lang.tr("boiler.via_one_engine", &[])
    } else {
        lang.tr("boiler.via_engines", &[boiler.engines.to_string()])
    };
    Builder::new()
        .number(total)
        .tr(lang, "generic.unit.stress", &[])
        .style(AQUA)
        .space()
        .add(Builder::new().text(via).style(DARK_GRAY))
        .for_goggles_at(tooltip, 1);
    true
}

// --- the Basin ----------------------------------------------------------------------------------

/// `BasinBlockEntity.addToGoggleTooltip`: `items` are `(item id, count)`, `fluids` `(fluid id, mB)`.
pub fn basin_goggles(
    lang: &Lang,
    items: &[(String, i32)],
    fluids: &[(String, i32)],
    tooltip: &mut Vec<Line>,
) -> bool {
    Builder::new().tr(lang, "gui.goggles.basin_contents", &[]).for_goggles(tooltip);
    let header = tooltip.len() - 1;
    let mut is_empty = true;
    for (item, count) in items {
        Builder::new()
            .add(Builder::new().text(lang.item_name(item)).style(GRAY))
            .add(Builder::new().text(format!(" x{count}")).style(GREEN))
            .for_goggles_at(tooltip, 1);
        is_empty = false;
    }
    for (fluid, amount) in fluids {
        Builder::new()
            .add(
                Builder::new()
                    .text(lang.fluid_name(fluid))
                    .add(Builder::new().text(" "))
                    .style(GRAY)
                    .add(Builder::new().number(f64::from(*amount)).add(mb(lang)).style(BLUE)),
            )
            .for_goggles_at(tooltip, 1);
        is_empty = false;
    }
    if is_empty {
        tooltip.remove(header);
    }
    true
}

// --- chute, deployer, lever, whistle, tunnel -----------------------------------------------------------

/// `ChuteBlockEntity.addToGoggleTooltip`: the fans' `pull` and `push`, which way items go
/// (`downward`) and the item it holds.
pub fn chute_goggles(
    lang: &Lang,
    pull: f32,
    push: f32,
    downward: bool,
    item: Option<(&str, i32)>,
    tooltip: &mut Vec<Line>,
) -> bool {
    Builder::new().tr(lang, "tooltip.chute.header", &[]).for_goggles(tooltip);
    if pull == 0.0 && push == 0.0 {
        Builder::new().tr(lang, "tooltip.chute.no_fans_attached", &[]).style(GRAY).for_goggles(tooltip);
    }
    if pull != 0.0 {
        let key = format!("tooltip.chute.fans_{}", if pull > 0.0 { "pull_up" } else { "push_down" });
        Builder::new().tr(lang, &key, &[]).style(GRAY).for_goggles(tooltip);
    }
    if push != 0.0 {
        let key = format!("tooltip.chute.fans_{}", if push > 0.0 { "push_up" } else { "pull_down" });
        Builder::new().tr(lang, &key, &[]).style(GRAY).for_goggles(tooltip);
    }
    let motion = format!("tooltip.chute.items_move_{}", if downward { "down" } else { "up" });
    Builder::new().text("-> ").tr(lang, &motion, &[]).style(YELLOW).for_goggles(tooltip);
    if let Some((id, count)) = item {
        Builder::new()
            .tr(lang, "tooltip.chute.contains", &[lang.item_name(id), count.to_string()])
            .style(GREEN)
            .for_goggles(tooltip);
    }
    true
}

/// `DeployerBlockEntity.addToGoggleTooltip` without the stress stats (the caller adds them).
pub fn deployer_goggles(
    lang: &Lang,
    using: bool,
    held: Option<(&str, i32)>,
    tooltip: &mut Vec<Line>,
) {
    Builder::new().tr(lang, "tooltip.deployer.header", &[]).for_goggles(tooltip);
    let mode = format!("tooltip.deployer.{}", if using { "using" } else { "punching" });
    Builder::new().tr(lang, &mode, &[]).style(YELLOW).for_goggles(tooltip);
    if let Some((id, count)) = held {
        Builder::new()
            .tr(lang, "tooltip.deployer.contains", &[lang.item_name(id), count.to_string()])
            .style(GREEN)
            .for_goggles(tooltip);
    }
}

/// `AnalogLeverBlockEntity.addToGoggleTooltip`.
pub fn analog_lever_goggles(lang: &Lang, state: i32, tooltip: &mut Vec<Line>) -> bool {
    Builder::new().tr(lang, "tooltip.analogStrength", &[state.to_string()]).for_goggles(tooltip);
    true
}

/// `WhistleBlockEntity.addToGoggleTooltip`: the pitch as a note (`generic.notes`).
pub fn whistle_goggles(lang: &Lang, pitch: i32, tooltip: &mut Vec<Line>) -> bool {
    let notes = lang.tr("generic.notes", &[]);
    let notes: Vec<&str> = notes.split(';').collect();
    let note = notes[pitch.rem_euclid(notes.len() as i32) as usize];
    Builder::new().tr(lang, "generic.pitch", &[note.to_owned()]).for_goggles(tooltip);
    true
}

/// `BrassTunnelBlockEntity.addToGoggleTooltip`: what the group of tunnels holds, as
/// `(item id, count)`; nothing held is no tooltip.
pub fn brass_tunnel_goggles(lang: &Lang, stacks: &[(String, i32)], tooltip: &mut Vec<Line>) -> bool {
    if stacks.is_empty() {
        return false;
    }
    Builder::new().tr(lang, "tooltip.brass_tunnel.contains", &[]).style(WHITE).for_goggles(tooltip);
    for (id, count) in stacks {
        Builder::new()
            .tr(
                lang,
                "tooltip.brass_tunnel.contains_entry",
                &[lang.item_name(id), count.to_string()],
            )
            .style(GRAY)
            .for_goggles(tooltip);
    }
    Builder::new().tr(lang, "tooltip.brass_tunnel.retrieve", &[]).style(DARK_GRAY).for_goggles(tooltip);
    true
}

// --- hints --------------------------------------------------------------------------------------

/// `TooltipHelper.addHint`: the hint's title in gold and its text cut into gray and white lines.
pub fn add_hint(lang: &Lang, key: &str, width: &dyn Fn(&str) -> f32, tooltip: &mut Vec<Line>) {
    Builder::new().tr(lang, &format!("{key}.title"), &[]).style(GOLD).for_goggles(tooltip);
    push_for_goggles(tooltip, cut_gray_and_white(&lang.tr(key, &[]), width));
}

/// `DeployerBlockEntity.addToTooltip` after the kinetic hint: a stopped or empty deployer says
/// nothing; one that turns with items stuck in its overflow asks to have them taken out.
pub fn deployer_hover(
    lang: &Lang,
    speed: f32,
    overflow: bool,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    if speed == 0.0 || !overflow {
        return false;
    }
    add_hint(lang, "hint.full_deployer", width, tooltip);
    true
}

/// `ArmBlockEntity.addToTooltip` after the kinetic hint.
pub fn arm_hover(
    lang: &Lang,
    sneaking: bool,
    has_targets: bool,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    if sneaking || has_targets {
        return false;
    }
    add_hint(lang, "hint.mechanical_arm_no_targets", width, tooltip);
    true
}

/// `MechanicalBearingBlockEntity.addToTooltip` after the kinetic hint: a bearing that turns (or is
/// a windmill), isn't running and has something solid in front asks to be clicked.
pub fn bearing_hover(
    lang: &Lang,
    sneaking: bool,
    turning: bool,
    running: bool,
    front_replaceable: bool,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    if sneaking || !turning || running || front_replaceable {
        return false;
    }
    add_hint(lang, "hint.empty_bearing", width, tooltip);
    true
}

/// `IDisplayAssemblyExceptions.addExceptionToTooltip`: why a structure did not assemble.
pub fn assembly_exception(
    lang: &Lang,
    text: &str,
    width: &dyn Fn(&str) -> f32,
    tooltip: &mut Vec<Line>,
) -> bool {
    if text.is_empty() {
        return false;
    }
    if !tooltip.is_empty() {
        tooltip.push(blank());
    }
    Builder::new().tr(lang, "gui.assembly.exception", &[]).style(GOLD).for_goggles(tooltip);
    for line in text.split('\n') {
        push_for_goggles(tooltip, cut_gray_and_white(line, width));
    }
    true
}

/// The pole length line `GoggleOverlayRenderer` adds under a Piston Extension Pole.
pub fn pole_length(lang: &Lang, poles: usize, tooltip: &mut Vec<Line>) {
    if !tooltip.is_empty() {
        tooltip.push(blank());
    }
    Builder::new()
        .tr(lang, "gui.goggles.pole_length", &[])
        .text(format!(" {poles}"))
        .for_goggles(tooltip);
}

#[cfg(test)]
mod tests {
    use super::super::goggles::plain;
    use super::*;
    use std::collections::BTreeMap;

    fn lang() -> Lang {
        let pairs = [
            ("create.generic.unit.millibuckets", "mB"),
            ("create.generic.unit.stress", "su"),
            ("create.gui.goggles.fluid_container", "Fluid Container Info:"),
            ("create.gui.goggles.fluid_container.capacity", "Capacity: "),
            ("create.gui.goggles.basin_contents", "Basin Contents:"),
            ("create.gui.goggles.pole_length", "Pole Length:"),
            ("create.gui.assembly.exception", "This Contraption was unable to assemble:"),
            ("create.boiler.heat", "Heat"),
            ("create.boiler.heat_dots", "...... "),
            ("create.boiler.size", "Size"),
            ("create.boiler.size_dots", "....... "),
            ("create.boiler.water", "Water"),
            ("create.boiler.water_dots", "... "),
            ("create.boiler.idle", "Idle"),
            ("create.boiler.passive", "Passive"),
            ("create.boiler.lvl", "Lvl %1$s"),
            ("create.boiler.max_lvl", "Max"),
            ("create.boiler.status", "Boiler Status:  %1$s"),
            ("create.boiler.per_tick", "%1$s per Tick"),
            ("create.boiler.via_engines", "via %1$s engines"),
            ("create.boiler.via_one_engine", "via 1 engine"),
            ("create.boiler.water_input_rate", "Water input rate"),
            ("create.tooltip.capacityProvided", "Kinetic Stress Capacity:"),
            ("create.tooltip.chute.header", "Chute Information"),
            ("create.tooltip.chute.no_fans_attached", "No attached fans"),
            ("create.tooltip.chute.fans_pull_up", "Fans pull from Above"),
            ("create.tooltip.chute.fans_push_down", "Fans push from Above"),
            ("create.tooltip.chute.fans_push_up", "Fans push from Below"),
            ("create.tooltip.chute.fans_pull_down", "Fans pull from Below"),
            ("create.tooltip.chute.items_move_down", "Items move Downward"),
            ("create.tooltip.chute.items_move_up", "Items move Upward"),
            ("create.tooltip.chute.contains", "Contains: %1$s x%2$s"),
            ("create.tooltip.deployer.header", "Deployer Information"),
            ("create.tooltip.deployer.using", "Mode: Use"),
            ("create.tooltip.deployer.punching", "Mode: Attack"),
            ("create.tooltip.deployer.contains", "Item: %1$s x%2$s"),
            ("create.tooltip.analogStrength", "Analog Strength: %1$s/15"),
            ("create.generic.pitch", "Pitch: %1$s"),
            ("create.generic.notes", "F#;F;E;D#;D;C#;C;B;A#;A;G#;G"),
            ("create.tooltip.brass_tunnel.contains", "Currently distributing:"),
            ("create.tooltip.brass_tunnel.contains_entry", "> %1$s x%2$s"),
            ("create.tooltip.brass_tunnel.retrieve", "Right-Click to retrieve"),
            ("create.hint.empty_bearing", "_Right-click_ the bearing."),
            ("create.hint.empty_bearing.title", "Update Bearing"),
            ("create.hint.mechanical_arm_no_targets", "It appears this _Mechanical_ _Arm_ has no _targets._"),
            ("create.hint.mechanical_arm_no_targets.title", "No Targets"),
            ("create.hint.full_deployer", "It appears this _Deployer_ is full."),
            ("create.hint.full_deployer.title", "Deployer cannot activate"),
            ("block.minecraft.water", "Water"),
            ("block.create.honey", "Honey"),
            ("item.minecraft.iron_ingot", "Iron Ingot"),
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

    fn tank(fluid: Option<&str>, amount: i32, capacity: i32) -> FluidTank {
        FluidTank { fluid: fluid.map(str::to_owned), amount, capacity }
    }

    #[test]
    fn a_tank_with_fluid_shows_the_fluid_and_how_full() {
        let mut tooltip = Vec::new();
        assert!(contained_fluid(&lang(), &[tank(Some("minecraft:water"), 1500, 8000)], &mut tooltip));
        assert_eq!(
            texts(&tooltip),
            ["    Fluid Container Info:", "     Water", "     1,500mB / 8,000mB"]
        );
        assert_eq!(tooltip[1][1].color, GRAY);
        assert_eq!(tooltip[2][1].color, GOLD);
        assert_eq!(tooltip[2][3].color, GRAY);
        assert_eq!(tooltip[2][4].color, DARK_GRAY);
    }

    #[test]
    fn an_empty_single_tank_shows_its_capacity() {
        let mut tooltip = Vec::new();
        assert!(contained_fluid(&lang(), &[tank(None, 0, 1000)], &mut tooltip));
        assert_eq!(texts(&tooltip), ["    Fluid Container Info:", "     Capacity: 1,000mB"]);
    }

    #[test]
    fn empty_tanks_among_several_leave_no_header() {
        let mut tooltip = Vec::new();
        assert!(contained_fluid(&lang(), &[tank(None, 0, 1000), tank(None, 0, 1000)], &mut tooltip));
        assert!(tooltip.is_empty());
        assert!(!contained_fluid(&lang(), &[], &mut tooltip));
    }

    fn boiler(active_heat: i32, passive: bool, supply: f32, engines: i32) -> Boiler {
        Boiler { water_supply: supply, active_heat, passive_heat: passive, engines, whistles: 0 }
    }

    #[test]
    fn a_boiler_shows_its_level_its_bars_and_what_it_gives() {
        // 2 x 2 x 4 tank: 16 blocks, size level 4; water 40 mB a tick: level 4; two burners: 2.
        let mut tooltip = Vec::new();
        let b = boiler(2, false, 40.0, 1);
        assert!(boiler_goggles(&lang(), &b, 16, &mut tooltip));
        let lines = texts(&tooltip);
        assert_eq!(lines[0], "    Boiler Status:  Lvl 2");
        assert_eq!(tooltip[0][2].color, GREEN);
        assert_eq!(lines[1], "     Size....... |||||");
        assert_eq!(lines[2], "     Water... |||||");
        assert_eq!(lines[3], "     Heat...... |||||");
        assert_eq!(lines[4], "");
        assert_eq!(lines[5], "    Kinetic Stress Capacity:");
        // Efficiency 1 (one engine under level 2): 16 * max(2, 1) * 1024.
        assert_eq!(lines[6], "     32,768su via 1 engine");
    }

    #[test]
    fn a_boiler_without_water_asks_for_it() {
        let mut tooltip = Vec::new();
        let b = boiler(1, false, 0.0, 2);
        assert!(boiler_goggles(&lang(), &b, 16, &mut tooltip));
        let lines = texts(&tooltip);
        assert_eq!(lines[0], "    Boiler Status:  Idle");
        assert_eq!(lines[5], "    Water input rate");
        assert_eq!(lines[6], "     0mB / 10mB per Tick");
    }

    #[test]
    fn a_boiler_that_nothing_is_attached_to_says_nothing() {
        let mut tooltip = Vec::new();
        assert!(!boiler_goggles(&lang(), &boiler(2, false, 40.0, 0), 16, &mut tooltip));
        assert!(tooltip.is_empty());
    }

    #[test]
    fn a_passive_boiler_reads_passive_with_one_heat_bar() {
        let mut tooltip = Vec::new();
        let b = boiler(0, true, 40.0, 1);
        assert!(boiler_goggles(&lang(), &b, 16, &mut tooltip));
        let lines = texts(&tooltip);
        assert_eq!(lines[0], "    Boiler Status:  Passive");
        assert!(lines[3].starts_with("     Heat...... |"));
    }

    #[test]
    fn a_basin_lists_its_items_then_its_fluids() {
        let mut tooltip = Vec::new();
        let items = [("minecraft:iron_ingot".to_owned(), 3)];
        let fluids = [("minecraft:water".to_owned(), 250), ("create:honey".to_owned(), 100)];
        assert!(basin_goggles(&lang(), &items, &fluids, &mut tooltip));
        assert_eq!(
            texts(&tooltip),
            [
                "    Basin Contents:",
                "     Iron Ingot x3",
                "     Water 250mB",
                "     Honey 100mB"
            ]
        );
        assert_eq!(tooltip[1][2].color, GREEN);
        assert_eq!(tooltip[2][3].color, BLUE);
        // An empty basin still answers true, with the header taken away: nothing shown.
        let mut none = Vec::new();
        assert!(basin_goggles(&lang(), &[], &[], &mut none));
        assert!(none.is_empty());
    }

    #[test]
    fn a_chute_tells_its_fans_and_its_direction() {
        let mut tooltip = Vec::new();
        chute_goggles(&lang(), 0.0, 0.0, true, None, &mut tooltip);
        assert_eq!(
            texts(&tooltip),
            ["    Chute Information", "    No attached fans", "    -> Items move Downward"]
        );
        assert_eq!(tooltip[2][1].color, YELLOW);
        let mut busy = Vec::new();
        chute_goggles(&lang(), 64.0, -32.0, false, Some(("minecraft:iron_ingot", 5)), &mut busy);
        assert_eq!(
            texts(&busy),
            [
                "    Chute Information",
                "    Fans pull from Above",
                "    Fans pull from Below",
                "    -> Items move Upward",
                "    Contains: Iron Ingot x5"
            ]
        );
    }

    #[test]
    fn a_deployer_tells_its_mode_and_what_it_holds() {
        let mut tooltip = Vec::new();
        deployer_goggles(&lang(), false, Some(("minecraft:iron_ingot", 2)), &mut tooltip);
        assert_eq!(
            texts(&tooltip),
            ["    Deployer Information", "    Mode: Attack", "    Item: Iron Ingot x2"]
        );
        let mut using = Vec::new();
        deployer_goggles(&lang(), true, None, &mut using);
        assert_eq!(texts(&using)[1], "    Mode: Use");
    }

    #[test]
    fn the_lever_the_whistle_and_the_tunnel() {
        let mut tooltip = Vec::new();
        analog_lever_goggles(&lang(), 7, &mut tooltip);
        whistle_goggles(&lang(), 14, &mut tooltip);
        assert_eq!(texts(&tooltip), ["    Analog Strength: 7/15", "    Pitch: E"]);
        let mut tunnel = Vec::new();
        assert!(!brass_tunnel_goggles(&lang(), &[], &mut tunnel));
        assert!(brass_tunnel_goggles(
            &lang(),
            &[("minecraft:iron_ingot".to_owned(), 4)],
            &mut tunnel
        ));
        assert_eq!(
            texts(&tunnel),
            [
                "    Currently distributing:",
                "    > Iron Ingot x4",
                "    Right-Click to retrieve"
            ]
        );
    }

    #[test]
    fn hints_have_a_gold_title_and_cut_text() {
        let mut tooltip = Vec::new();
        assert!(arm_hover(&lang(), false, false, &six, &mut tooltip));
        assert_eq!(plain(&tooltip[0]), "    No Targets");
        assert_eq!(tooltip[0][1].color, GOLD);
        assert_eq!(plain(&tooltip[1]), "    It appears this Mechanical Arm has no targets.");
        // Sneaking, or with targets, no hint.
        assert!(!arm_hover(&lang(), true, false, &six, &mut Vec::new()));
        assert!(!arm_hover(&lang(), false, true, &six, &mut Vec::new()));
    }

    #[test]
    fn a_bearing_and_a_deployer_hint_only_when_they_have_cause() {
        let mut t = Vec::new();
        assert!(bearing_hover(&lang(), false, true, false, false, &six, &mut t));
        assert_eq!(plain(&t[0]), "    Update Bearing");
        for (sneak, turning, running, replaceable) in
            [(true, true, false, false), (false, false, false, false), (false, true, true, false), (false, true, false, true)]
        {
            assert!(!bearing_hover(&lang(), sneak, turning, running, replaceable, &six, &mut Vec::new()));
        }
        assert!(deployer_hover(&lang(), 16.0, true, &six, &mut Vec::new()));
        assert!(!deployer_hover(&lang(), 0.0, true, &six, &mut Vec::new()));
        assert!(!deployer_hover(&lang(), 16.0, false, &six, &mut Vec::new()));
    }

    #[test]
    fn an_assembly_exception_follows_what_is_already_shown() {
        let mut tooltip = Vec::new();
        assert!(!assembly_exception(&lang(), "", &six, &mut tooltip));
        assert!(assembly_exception(&lang(), "The Piston is missing some extension Poles", &six, &mut tooltip));
        assert_eq!(plain(&tooltip[0]), "    This Contraption was unable to assemble:");
        assert_eq!(plain(&tooltip[1]), "    The Piston is missing some extension Poles");
        let mut after = vec![vec![]];
        assembly_exception(&lang(), "x", &six, &mut after);
        assert_eq!(after.len(), 4);
        assert!(after[1].is_empty());
    }

    #[test]
    fn a_pole_tells_its_length() {
        let mut tooltip = Vec::new();
        pole_length(&lang(), 4, &mut tooltip);
        assert_eq!(texts(&tooltip), ["    Pole Length: 4"]);
    }
}

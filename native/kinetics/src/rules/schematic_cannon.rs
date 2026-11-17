//! The Schematicannon's state machine (`SchematicannonBlockEntity.tickPrinter`, `initializePrinter`,
//! `finishedPrinting`, `shouldPlace`, `refillFuelIfPossible`, `grabItemsFromAttachedInventories`'
//! accounting, `LaunchedItem` and `SchematicannonRenderer`'s aim and flight maths,
//! `ConfigureSchematicannonPacket.Option`, `CSchematics`): when it prints, what it needs, what it
//! skips and where its shot flies. No game types: the glue holds the block entity and answers [`Io`].
//!
//! Known differences from Create, for the README:
//! - The Material List printer (a Book or a Clipboard in the input slot, `tickPaperPrinter`) is not
//!   ported: the checklist is told in chat instead.
//! - Entities are not launched, and belts are not printed (`schematic_printer`).
//! - A flying block is a function of the game time since its launch (`Launched`), not a counter
//!   the block entity ticks down on both sides, so a client draws it with no update per tick.
//! - The cannon's settings (replace mode, skip missing, replace block entities, play, pause, stop)
//!   are set from `/schematicannon ...` and the Wrench, not from Create's screen.

use crate::rules::contraption::State;
use crate::rules::schematic_nbt::Nbt;
use crate::rules::schematic_printer::{
    Candidate, Checklist, Existing, Printer, Requirement, conductor, has_block_entity, is_air, placement_state,
    should_ignore,
};
use crate::rules::schematic_structure::Pos;

/// `CSchematics.schematicannonDelay`.
pub const DELAY: i32 = 10;
/// `CSchematics.schematicannonShotsPerGunpowder`.
pub const SHOTS_PER_GUNPOWDER: i32 = 400;
/// `SchematicannonBlockEntity.MAX_ANCHOR_DISTANCE`.
pub const MAX_ANCHOR_DISTANCE: i32 = 256;
/// `SchematicannonBlockEntity.NEIGHBOUR_CHECKING`: ticks between looks at the containers beside.
pub const NEIGHBOUR_CHECKING: i32 = 100;
/// `tickPrinter` runs again within a tick while it skips blocks, up to this many times.
pub const SKIPS_PER_TICK: i32 = 1000;

/// `SchematicannonBlockEntity.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Stopped,
    Paused,
    Running,
}

impl RunState {
    pub fn name(self) -> &'static str {
        match self {
            RunState::Stopped => "STOPPED",
            RunState::Paused => "PAUSED",
            RunState::Running => "RUNNING",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "PAUSED" => RunState::Paused,
            "RUNNING" => RunState::Running,
            _ => RunState::Stopped,
        }
    }
}

/// `ConfigureSchematicannonPacket.Option` for the replace mode (its ordinals are the mode).
pub const REPLACE_MODES: [&str; 4] = ["dont_replace", "replace_solid", "replace_any", "replace_empty"];

/// What the Schematicannon's screen says for each status, in `en_us` (`create.schematicannon.status.*`).
pub fn status_text(status: &str) -> &'static str {
    match status {
        "idle" => "Idle",
        "ready" => "Ready",
        "running" => "Running",
        "paused" => "Paused",
        "stopped" => "Stopped",
        "finished" => "Finished",
        "placing" => "Placing",
        "clearing" => "Clearing Blocks",
        "searching" => "Searching",
        "skipping" => "Skipping",
        "missingBlock" => "Missing Item:",
        "noGunpowder" => "Out of Gunpowder",
        "targetNotLoaded" => "Target is not loaded",
        "targetOutsideRange" => "Target too far away",
        "schematicInvalid" => "Schematic Invalid",
        "schematicNotPlaced" => "Schematic not Positioned",
        "schematicErrored" => "Incompatible Blocks",
        "schematicExpired" => "Schematic File Expired",
        _ => "",
    }
}

// --- the schematic in its slot --------------------------------------------------------------------

/// What slot 0 holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blueprint {
    Empty,
    /// A Schematic with no anchor.
    Invalid,
    /// `SCHEMATIC_DEPLOYED` false.
    NotDeployed,
    Deployed,
}

/// Why a Schematic can't be loaded for printing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// `printer.isErrored()`: the file could not be read or placed.
    Errored,
    /// `printer.isWorldEmpty()`: the file holds nothing (or is gone).
    Expired,
}

/// What the cannon needs of the world and of the glue, one tick at a time.
pub trait Io {
    fn blueprint(&self) -> Blueprint;
    /// Slot 1 holds nothing.
    fn output_empty(&self) -> bool;
    /// `SchematicPrinter.loadSchematic`: the printer for the schematic in slot 0.
    fn load(&mut self) -> Result<Printer, LoadError>;
    /// `Level.isLoaded(pos)` and inside the world border.
    fn is_loaded(&self, pos: Pos) -> bool;
    /// What stands at `pos` now.
    fn world(&mut self, pos: Pos) -> Existing;
    fn item_exists(&self, item: &str) -> bool;
    /// `grabItemsFromAttachedInventories`: whether `count` of `item` can be had (`simulate`) or
    /// takes them from the containers beside the cannon.
    fn take(&mut self, item: &str, count: u32, simulate: bool) -> bool;
    /// `refillFuelIfPossible`: one gunpowder from slot 4, else from a container beside.
    fn take_gunpowder(&mut self) -> bool;
}

/// What the glue does for the cannon after a step.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `playFiringSound`.
    Fired,
    /// `SCHEMATICANNON_FINISH`.
    FinishSound,
    /// Slot 0 emptied; `give_empty` adds an Empty Schematic to slot 1 (`finishedPrinting`) or, for a
    /// schematic that can't be printed, puts one there.
    ClearBlueprint { give_empty: bool },
    /// `sendUpdate`: tell the clients.
    Update,
    /// The printer was reset; forget it.
    ResetPrinter,
}

// --- flying blocks --------------------------------------------------------------------------------

/// `LaunchedItem.ForBlockState`.
#[derive(Clone, Debug, PartialEq)]
pub struct Flying {
    /// Game time of the shot.
    pub launched: i64,
    pub total_ticks: i32,
    pub target: Pos,
    pub icon: String,
    pub state: State,
    /// `BlockHelper.prepareBlockEntityData`.
    pub data: Option<Nbt>,
}

impl Flying {
    /// `LaunchedItem.ticksForDistance`: `max(10, (distSqr^(1/4)) * 4)`, truncated.
    pub fn ticks_for_distance(from: Pos, to: Pos) -> i32 {
        let d: f64 = (0..3).map(|i| f64::from(to[i] - from[i]).powi(2)).sum();
        ((d.sqrt().sqrt() * 4.0) as i32).max(10)
    }

    /// `ticksRemaining` at game time `now`.
    pub fn remaining(&self, now: i64) -> i32 {
        (i64::from(self.total_ticks) - (now - self.launched)).max(0) as i32
    }

    /// `LaunchedItem.update` on the server: true once it has landed (the glue places it).
    pub fn landed(&self, now: i64) -> bool {
        self.remaining(now) == 0
    }
}

/// Where a flying block is drawn, relative to the cannon's block, and how far it has turned
/// (`SchematicannonRenderer.renderLaunchedBlocks`). `None` once it has landed.
pub fn flight_pose(cannon: Pos, flying: &Flying, now: i64, partial: f64) -> Option<([f64; 3], f64)> {
    let remaining = flying.remaining(now);
    if remaining == 0 {
        return None;
    }
    let total = f64::from(flying.total_ticks);
    let start = [f64::from(cannon[0]) + 0.5, f64::from(cannon[1]) + 1.5, f64::from(cannon[2]) + 0.5];
    let target = [
        f64::from(flying.target[0]) + 0.5,
        f64::from(flying.target[1]) + 0.5,
        f64::from(flying.target[2]) + 0.5,
    ];
    let distance = [target[0] - start[0], target[1] - start[1], target[2] - start[2]];
    let y_difference = target[1] - start[1];
    let length = (distance[0].powi(2) + distance[1].powi(2) + distance[2].powi(2)).sqrt();
    let throw_height = length * 0.6 + y_difference;
    let lift = [distance[0], distance[1] + throw_height, distance[2]];
    let lift_length = (lift[0].powi(2) + lift[1].powi(2) + lift[2].powi(2)).sqrt().max(1e-9);
    let offset = [lift[0] / lift_length * 2.0, lift[1] / lift_length * 2.0, lift[2] / lift_length * 2.0];
    let start = [start[0] + offset[0], start[1] + offset[1], start[2] + offset[2]];
    let y_difference = target[1] - start[1];
    let progress = (total - (f64::from(remaining) + 1.0 - partial)) / total;
    let xz = [(target[0] - start[0]) * progress, (target[2] - start[2]) * progress];
    // A bezier for the height.
    let t = progress;
    let y = 2.0 * (1.0 - t) * t * throw_height + t * t * y_difference;
    Some(([xz[0] + 0.5 + offset[0], y + 1.5 + offset[1], xz[1] + 0.5 + offset[2]], 360.0 * t))
}

/// `SchematicannonRenderer.getCannonAngles`: the yaw and pitch in degrees. `target` is the
/// printer's current target, `previous` the one a tick ago; `partial` blends them.
pub fn cannon_angles(
    cannon: Pos,
    target: Option<Pos>,
    previous: Option<Pos>,
    partial: f64,
    default_yaw: f64,
) -> (f64, f64) {
    let Some(target) = target else {
        return (default_yaw, 40.0);
    };
    let at = |p: Pos| [f64::from(p[0] - cannon[0]), f64::from(p[1] - cannon[1]), f64::from(p[2] - cannon[2])];
    let mut diff = at(target);
    if let Some(previous) = previous {
        let (a, b) = (at(previous), at(target));
        diff = [a[0] + (b[0] - a[0]) * partial, a[1] + (b[1] - a[1]) * partial, a[2] + (b[2] - a[2]) * partial];
    }
    let yaw = diff[0].atan2(diff[2]).to_degrees();
    let distance = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt();
    let y_offset = distance * 2.0;
    let pitch = distance.atan2(diff[1] * 3.0 + y_offset).to_degrees() + 10.0;
    (yaw, pitch)
}

/// `SchematicannonRenderer.getRecoil`.
pub fn recoil(flying: &[Flying], now: i64, partial: f64) -> f64 {
    let mut recoil: f64 = 0.0;
    for f in flying {
        let remaining = f.remaining(now);
        if remaining == 0 {
            continue;
        }
        let left = f64::from(remaining) + 1.0 - partial;
        if left > f64::from(f.total_ticks) - 10.0 {
            recoil = recoil.max(left - f64::from(f.total_ticks) + 10.0);
        }
    }
    recoil
}

// --- block entity data a schematic carries --------------------------------------------------------

/// `BlockHelper.prepareBlockEntityData`: the saved data of a block entity that is safe to copy
/// (`create:safe_nbt` blocks, and signs' text), else none. Create's `NBTProcessors` also strip
/// containers' items and the like; here what is kept is named.
pub fn safe_block_entity_data(state: &State, nbt: &Nbt) -> Option<Nbt> {
    let name = state.name.as_str();
    let path = state.path();
    let create_safe = matches!(
        name,
        "create:creative_motor"
            | "create:creative_fluid_tank"
            | "create:mechanical_piston"
            | "create:sticky_mechanical_piston"
            | "create:windmill_bearing"
    );
    let sign = name.starts_with("minecraft:") && (path.ends_with("_sign") || path.ends_with("_hanging_sign"));
    if !create_safe && !sign {
        return None;
    }
    let mut data = nbt.clone();
    for key in ["x", "y", "z", "Items"] {
        data.remove(key);
    }
    Some(data)
}

// --- the cannon -----------------------------------------------------------------------------------

/// The saved state of one Schematicannon.
#[derive(Clone, Debug, PartialEq)]
pub struct Cannon {
    pub pos: Pos,
    pub state: RunState,
    pub status: String,
    pub missing_item: Option<String>,
    pub position_not_loaded: bool,
    pub has_creative_crate: bool,
    pub printer_cooldown: i32,
    pub remaining_fuel: i32,
    pub blocks_placed: i32,
    pub blocks_to_place: i32,
    /// 0 to 3, `REPLACE_MODES`.
    pub replace_mode: i32,
    pub skip_missing: bool,
    pub replace_block_entities: bool,
    pub flying: Vec<Flying>,
    /// Set by `tick_printer` when it ran past a block without printing (`blockSkipped`).
    pub block_skipped: bool,
    pub previous_target: Option<Pos>,
    pub checklist: Checklist,
}

impl Cannon {
    /// `SchematicannonBlockEntity`'s constructor.
    pub fn new(pos: Pos) -> Self {
        Cannon {
            pos,
            state: RunState::Stopped,
            status: "idle".to_owned(),
            missing_item: None,
            position_not_loaded: false,
            has_creative_crate: false,
            printer_cooldown: 0,
            remaining_fuel: 0,
            blocks_placed: 0,
            blocks_to_place: 0,
            replace_mode: 2,
            skip_missing: false,
            replace_block_entities: false,
            flying: Vec::new(),
            block_skipped: false,
            previous_target: None,
            checklist: Checklist::default(),
        }
    }

    /// `getShotsPerGunpowder`.
    pub fn shots_per_gunpowder(&self) -> i32 {
        if self.has_creative_crate { 0 } else { SHOTS_PER_GUNPOWDER }
    }

    /// `ConfigureSchematicannonPacket`'s play, pause and stop.
    pub fn play(&mut self) {
        self.state = RunState::Running;
        self.status = "running".to_owned();
    }

    pub fn pause(&mut self) {
        self.state = RunState::Paused;
        self.status = "paused".to_owned();
    }

    pub fn stop(&mut self) {
        self.state = RunState::Stopped;
        self.status = "stopped".to_owned();
    }

    /// `SchematicannonBlockEntity.shouldPlace`.
    pub fn should_place(&self, c: &Candidate, item_exists: &dyn Fn(&str) -> bool) -> bool {
        // `pos.closerThan(getBlockPos(), 2f)`.
        let d: i32 = (0..3).map(|i| (c.target[i] - self.pos[i]).pow(2)).sum();
        if d < 4 {
            return false;
        }
        let other_entity = c.to_replace_other.as_ref().is_some_and(|o| o.has_block_entity);
        if !self.replace_block_entities && (c.to_replace.has_block_entity || other_entity) {
            return false;
        }
        if should_ignore(&c.state, item_exists) {
            return false;
        }
        let placing_air = is_air(&c.state);
        let replace_conductor = conductor(&c.to_replace.state);
        let other_conductor = c.to_replace_other.as_ref().is_some_and(|o| conductor(&o.state));
        match self.replace_mode {
            3 => true,
            2 => !placing_air,
            1 => (c.is_normal_cube || (!replace_conductor && !other_conductor)) && !placing_air,
            0 => !replace_conductor && !other_conductor && !placing_air,
            _ => false,
        }
    }

    /// `updateChecklist`'s count: how many blocks this schematic would place from here.
    pub fn update_checklist(&mut self, printer: Option<&Printer>, io: &mut dyn Io) {
        let mut checklist = Checklist::default();
        if let Some(printer) = printer {
            let cannon = self.clone();
            let io_cell = std::cell::RefCell::new(&mut *io);
            let exists = |item: &str| io_cell.borrow().item_exists(item);
            let mut world = |pos: Pos| {
                let mut io = io_cell.borrow_mut();
                io.is_loaded(pos).then(|| io.world(pos))
            };
            let placed =
                printer.count_requirements(&mut world, &|c| cannon.should_place(c, &exists), &exists, &mut checklist);
            self.blocks_to_place = self.blocks_placed + placed;
        }
        self.checklist = checklist;
    }

    /// `refillFuelIfPossible`.
    pub fn refill_fuel_if_possible(&mut self, io: &mut dyn Io, effects: &mut Vec<Effect>) {
        if self.has_creative_crate {
            return;
        }
        if self.remaining_fuel > self.shots_per_gunpowder() {
            self.remaining_fuel = self.shots_per_gunpowder();
            effects.push(Effect::Update);
            return;
        }
        if self.remaining_fuel > 0 {
            return;
        }
        if !io.take_gunpowder() {
            return;
        }
        self.remaining_fuel += self.shots_per_gunpowder();
        if self.status == "noGunpowder" {
            if self.blocks_placed > 0 {
                self.state = RunState::Running;
            }
            self.status = "ready".to_owned();
        }
        effects.push(Effect::Update);
    }

    /// `resetPrinter`.
    fn reset_printer(&mut self, printer: &mut Option<Printer>, effects: &mut Vec<Effect>) {
        *printer = None;
        self.missing_item = None;
        self.blocks_placed = 0;
        self.blocks_to_place = 0;
        effects.push(Effect::ResetPrinter);
        effects.push(Effect::Update);
    }

    /// `finishedPrinting`.
    fn finished_printing(&mut self, printer: &mut Option<Printer>, effects: &mut Vec<Effect>) {
        effects.push(Effect::ClearBlueprint { give_empty: true });
        self.state = RunState::Stopped;
        self.status = "finished".to_owned();
        self.reset_printer(printer, effects);
        effects.push(Effect::FinishSound);
        effects.push(Effect::Update);
    }

    /// `initializePrinter`.
    fn initialize_printer(&mut self, printer: &mut Option<Printer>, io: &mut dyn Io, effects: &mut Vec<Effect>) {
        match io.blueprint() {
            // `!blueprint.has(SCHEMATIC_ANCHOR)`: an empty slot has none either.
            Blueprint::Invalid | Blueprint::Empty => {
                self.state = RunState::Stopped;
                self.status = "schematicInvalid".to_owned();
                effects.push(Effect::Update);
                return;
            }
            Blueprint::NotDeployed => {
                self.state = RunState::Stopped;
                self.status = "schematicNotPlaced".to_owned();
                effects.push(Effect::Update);
                return;
            }
            Blueprint::Deployed => {}
        }
        let loaded = io.load();
        let expired = match &loaded {
            Err(LoadError::Errored) => Some("schematicErrored"),
            Err(LoadError::Expired) => Some("schematicExpired"),
            Ok(p) if p.layout.is_empty() => Some("schematicExpired"),
            Ok(_) => None,
        };
        if let Some(status) = expired {
            self.state = RunState::Stopped;
            self.status = status.to_owned();
            effects.push(Effect::ClearBlueprint { give_empty: true });
            *printer = None;
            effects.push(Effect::ResetPrinter);
            effects.push(Effect::Update);
            return;
        }
        let Ok(loaded) = loaded else {
            return;
        };
        // `printer.getAnchor().closerThan(getBlockPos(), MAX_ANCHOR_DISTANCE)`.
        let d: i64 = (0..3).map(|i| i64::from(loaded.anchor[i] - self.pos[i]).pow(2)).sum();
        if d >= i64::from(MAX_ANCHOR_DISTANCE) * i64::from(MAX_ANCHOR_DISTANCE) {
            self.state = RunState::Stopped;
            self.status = "targetOutsideRange".to_owned();
            *printer = None;
            effects.push(Effect::ResetPrinter);
            effects.push(Effect::Update);
            return;
        }
        *printer = Some(loaded);
        self.state = RunState::Paused;
        self.status = "ready".to_owned();
        self.update_checklist(printer.as_ref(), io);
        effects.push(Effect::Update);
        self.blocks_to_place += self.blocks_placed;
    }

    /// The part of `tickPaperPrinter` that loads the schematic: a cannon with a Schematic in its slot
    /// reads it (and waits, paused, for Play) whatever its state. Called each tick before
    /// [`Cannon::refill_fuel_if_possible`] and [`Cannon::tick_printer`].
    pub fn tick_loader(&mut self, printer: &mut Option<Printer>, io: &mut dyn Io) -> Vec<Effect> {
        let mut effects = Vec::new();
        if printer.is_none() && io.blueprint() != Blueprint::Empty {
            self.initialize_printer(printer, io, &mut effects);
        }
        effects
    }

    /// `tickPrinter`: one step. The glue calls it again, up to [`SKIPS_PER_TICK`] times, while
    /// `block_skipped` is set.
    pub fn tick_printer(&mut self, printer: &mut Option<Printer>, io: &mut dyn Io, now: i64) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.block_skipped = false;
        let blueprint = io.blueprint();
        if blueprint == Blueprint::Empty && self.status != "idle" && io.output_empty() {
            self.state = RunState::Stopped;
            self.status = "idle".to_owned();
            effects.push(Effect::Update);
            return effects;
        }
        // Skip if not active.
        if self.state == RunState::Stopped {
            if printer.is_some() {
                self.reset_printer(printer, &mut effects);
            }
            return effects;
        }
        if self.state == RunState::Paused
            && !self.position_not_loaded
            && self.missing_item.is_none()
            && self.remaining_fuel > 0
        {
            return effects;
        }
        // Initialize.
        if printer.is_none() {
            self.initialize_printer(printer, io, &mut effects);
            return effects;
        }
        // Cooldown from the last shot.
        if self.printer_cooldown > 0 {
            self.printer_cooldown -= 1;
            return effects;
        }
        // Fuel.
        if self.remaining_fuel <= 0 && !self.has_creative_crate {
            self.refill_fuel_if_possible(io, &mut effects);
            if self.remaining_fuel <= 0 {
                self.state = RunState::Paused;
                self.status = "noGunpowder".to_owned();
                effects.push(Effect::Update);
                return effects;
            }
        }
        if self.has_creative_crate {
            self.remaining_fuel = 0;
            if self.missing_item.is_some() {
                self.missing_item = None;
                self.state = RunState::Running;
            }
        }
        let Some(print) = printer.as_mut() else {
            return effects;
        };
        // Update the target.
        if self.missing_item.is_none() && !self.position_not_loaded {
            if !print.advance() {
                self.finished_printing(printer, &mut effects);
                return effects;
            }
            effects.push(Effect::Update);
        }
        let target = print.target();
        if !io.is_loaded(target) {
            self.position_not_loaded = true;
            self.status = "targetNotLoaded".to_owned();
            self.state = RunState::Paused;
            return effects;
        } else if self.position_not_loaded {
            self.position_not_loaded = false;
            self.state = RunState::Running;
        }
        // The item requirement.
        let exists = |item: &str| io.item_exists(item);
        let needs = print.current_requirement(&exists);
        let cannon = self.clone();
        let io_cell = std::cell::RefCell::new(&mut *io);
        let place = {
            let mut world = |pos: Pos| io_cell.borrow_mut().world(pos);
            let item_exists = |item: &str| io_cell.borrow().item_exists(item);
            print.should_place(target, true, &mut world, &|c| cannon.should_place(c, &item_exists))
        };
        let io = io_cell.into_inner();
        if needs == Requirement::Invalid || !place {
            if self.status != "searching" {
                effects.push(Effect::Update);
            }
            self.status = "searching".to_owned();
            self.block_skipped = true;
            return effects;
        }
        // Find the items.
        let mut icon: Option<String> = None;
        if let Requirement::Items(list) = &needs {
            for (item, count) in list {
                if !io.take(item, *count, true) {
                    if self.skip_missing {
                        self.status = "skipping".to_owned();
                        self.block_skipped = true;
                        if self.missing_item.is_some() {
                            self.missing_item = None;
                            self.state = RunState::Running;
                        }
                        return effects;
                    }
                    self.missing_item = Some(item.clone());
                    self.state = RunState::Paused;
                    self.status = "missingBlock".to_owned();
                    return effects;
                }
            }
            for (item, count) in list {
                io.take(item, *count, false);
            }
            icon = list.first().map(|(item, _)| item.clone());
        }
        // Success: launch the block.
        self.state = RunState::Running;
        let state = crate::rules::schematic_printer::zero_age(&print.state_at(target));
        let rel = [target[0] - print.anchor[0], target[1] - print.anchor[1], target[2] - print.anchor[2]];
        let data = print.layout.nbt(rel).and_then(|nbt| safe_block_entity_data(&state, nbt));
        self.status = if state.name != "minecraft:air" { "placing" } else { "clearing" }.to_owned();
        self.launch(target, icon.unwrap_or_default(), state, data, now);
        effects.push(Effect::Fired);
        self.printer_cooldown = DELAY;
        self.remaining_fuel -= 1;
        effects.push(Effect::Update);
        self.missing_item = None;
        effects
    }

    /// `launchBlock`.
    fn launch(&mut self, target: Pos, icon: String, state: State, data: Option<Nbt>, now: i64) {
        if !is_air(&state) {
            self.blocks_placed += 1;
        }
        let total = Flying::ticks_for_distance(self.pos, target);
        self.flying.push(Flying { launched: now, total_ticks: total, target, icon, state, data });
    }

    /// `tickFlyingBlocks` on the server: the blocks that have landed, to be placed
    /// (`LaunchedItem.place`).
    pub fn landed(&mut self, now: i64) -> Vec<Flying> {
        let (done, flying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.flying).into_iter().partition(|f| f.landed(now));
        self.flying = flying;
        done
    }

    /// What `BlockHelper.placeSchematicBlock` puts in the world for a landed block.
    pub fn landing_state(flying: &Flying) -> State {
        placement_state(&flying.state)
    }

    /// Whether a block entity is held by this block: used for `Existing`.
    pub fn existing(state: State, own_block_entity: bool) -> Existing {
        let has_block_entity = has_block_entity(&state, own_block_entity);
        Existing { state, has_block_entity }
    }

    /// The Schematicannon's status in words, for the chat.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} ({}): {} / {} blocks placed, {} shots of fuel left",
            status_text(&self.status),
            self.state.name().to_lowercase(),
            self.blocks_placed,
            self.blocks_to_place,
            if self.has_creative_crate { "unlimited".to_owned() } else { self.remaining_fuel.max(0).to_string() },
        )];
        if let Some(item) = &self.missing_item {
            lines.push(format!("Missing: {item}"));
        }
        lines.push(format!(
            "Settings: {}, {} missing blocks, {} block entities",
            REPLACE_MODES[self.replace_mode.clamp(0, 3) as usize],
            if self.skip_missing { "skip" } else { "wait for" },
            if self.replace_block_entities { "replace" } else { "keep" },
        ));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::schematic_printer::Layout;
    use crate::rules::schematic_structure::{Block, Template};
    use crate::rules::schematic_transform::{Mirror, Rotation};
    use std::collections::BTreeMap;

    /// A cannon at the origin, a floor of stone to print at y 1 and a world the test controls.
    struct Fake {
        blueprint: Blueprint,
        template: Template,
        anchor: Pos,
        stock: BTreeMap<String, u32>,
        fuel: u32,
        world: BTreeMap<Pos, State>,
        unloaded: Vec<Pos>,
        output_empty: bool,
    }

    impl Fake {
        fn new(blocks: &[(Pos, &str)]) -> Self {
            let mut template = Template { size: [3, 1, 3], ..Template::default() };
            for (pos, text) in blocks {
                let state = State::parse(text);
                let index = template.palette.iter().position(|s| *s == state).unwrap_or_else(|| {
                    template.palette.push(state.clone());
                    template.palette.len() - 1
                });
                template.blocks.push(Block { pos: *pos, state: index, nbt: None });
            }
            Fake {
                blueprint: Blueprint::Deployed,
                template,
                anchor: [10, 0, 10],
                stock: BTreeMap::new(),
                fuel: 0,
                world: BTreeMap::new(),
                unloaded: Vec::new(),
                output_empty: true,
            }
        }
    }

    impl Io for Fake {
        fn blueprint(&self) -> Blueprint {
            self.blueprint
        }
        fn output_empty(&self) -> bool {
            self.output_empty
        }
        fn load(&mut self) -> Result<Printer, LoadError> {
            Ok(Printer::new(self.anchor, Layout::new(&self.template, Rotation::None, Mirror::None)))
        }
        fn is_loaded(&self, pos: Pos) -> bool {
            !self.unloaded.contains(&pos)
        }
        fn world(&mut self, pos: Pos) -> Existing {
            Existing { state: self.world.get(&pos).cloned().unwrap_or_else(|| State::new("minecraft:air")), has_block_entity: false }
        }
        fn item_exists(&self, _: &str) -> bool {
            true
        }
        fn take(&mut self, item: &str, count: u32, simulate: bool) -> bool {
            let have = self.stock.get(item).copied().unwrap_or(0);
            if have < count {
                return false;
            }
            if !simulate {
                self.stock.insert(item.to_owned(), have - count);
            }
            true
        }
        fn take_gunpowder(&mut self) -> bool {
            if self.fuel == 0 {
                return false;
            }
            self.fuel -= 1;
            true
        }
    }

    /// Runs the cannon for `ticks` game ticks like the block entity does, placing what lands.
    fn run(cannon: &mut Cannon, printer: &mut Option<Printer>, io: &mut Fake, from: i64, ticks: i64) -> Vec<Effect> {
        let mut all = Vec::new();
        for now in from..from + ticks {
            for landed in cannon.landed(now) {
                io.world.insert(landed.target, Cannon::landing_state(&landed));
            }
            let mut step = cannon.tick_loader(printer, io);
            cannon.refill_fuel_if_possible(io, &mut step);
            let mut skips = SKIPS_PER_TICK;
            loop {
                step.extend(cannon.tick_printer(printer, io, now));
                skips -= 1;
                if !cannon.block_skipped || skips <= 0 {
                    break;
                }
            }
            // The glue empties the slot of a finished schematic and puts an empty one out.
            if step.contains(&Effect::ClearBlueprint { give_empty: true }) {
                io.blueprint = Blueprint::Empty;
                io.output_empty = false;
            }
            all.extend(step);
        }
        all
    }

    #[test]
    fn a_cannon_starts_paused_then_prints_when_played_and_stops_when_done() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "minecraft:dirt")]);
        io.stock.insert("minecraft:stone".into(), 5);
        io.stock.insert("minecraft:dirt".into(), 5);
        io.fuel = 1;
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        cannon.state = RunState::Paused;
        cannon.status = "ready".into();
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        // It loaded the schematic and counted what it would place, and waits to be played.
        assert!(printer.is_some());
        assert_eq!(cannon.blocks_to_place, 2);
        assert_eq!(cannon.state, RunState::Paused);
        assert_eq!(cannon.status, "ready");
        cannon.play();
        let effects = run(&mut cannon, &mut printer, &mut io, 2, 200);
        assert_eq!(io.world.get(&[10, 0, 10]).map(|s| s.name.as_str()), Some("minecraft:stone"));
        assert_eq!(io.world.get(&[11, 0, 10]).map(|s| s.name.as_str()), Some("minecraft:dirt"));
        // Two shots, the finish and the schematic put into the output.
        assert_eq!(effects.iter().filter(|e| **e == Effect::Fired).count(), 2);
        assert!(effects.contains(&Effect::FinishSound));
        assert!(effects.contains(&Effect::ClearBlueprint { give_empty: true }));
        assert_eq!(cannon.status, "finished");
        assert_eq!(cannon.state, RunState::Stopped);
        assert_eq!(io.stock["minecraft:stone"], 4);
        // One gunpowder was burned.
        assert_eq!(io.fuel, 0);
        assert_eq!(cannon.remaining_fuel, SHOTS_PER_GUNPOWDER - 2);
    }

    #[test]
    fn shots_are_ten_ticks_apart() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "minecraft:stone"), ([2, 0, 0], "minecraft:stone")]);
        io.stock.insert("minecraft:stone".into(), 5);
        io.fuel = 1;
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        // Loading leaves it paused; Play starts it.
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        assert_eq!(cannon.status, "ready");
        cannon.play();
        let mut shots = Vec::new();
        for now in 2..80 {
            if run(&mut cannon, &mut printer, &mut io, now, 1).contains(&Effect::Fired) {
                shots.push(now);
            }
        }
        assert_eq!(shots.len(), 3);
        // `schematicannonDelay` ticks of cooldown between shots.
        assert!(shots.windows(2).all(|w| w[1] - w[0] == i64::from(DELAY) + 1), "{shots:?}");
    }

    #[test]
    fn a_missing_item_pauses_it_and_a_skip_goes_on() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "minecraft:dirt")]);
        io.stock.insert("minecraft:dirt".into(), 1);
        io.fuel = 1;
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        cannon.play();
        run(&mut cannon, &mut printer, &mut io, 2, 50);
        // Stone comes first and there is none: it waits there.
        assert_eq!(cannon.status, "missingBlock");
        assert_eq!(cannon.missing_item.as_deref(), Some("minecraft:stone"));
        assert_eq!(cannon.state, RunState::Paused);
        assert!(io.world.is_empty());
        // Handing in the stone lets it go on (the glue's cannon is polled each tick).
        io.stock.insert("minecraft:stone".into(), 1);
        run(&mut cannon, &mut printer, &mut io, 52, 100);
        assert_eq!(io.world.len(), 2);
        // The same with skipping on: it passes the block it can't make.
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "minecraft:dirt")]);
        io.stock.insert("minecraft:dirt".into(), 1);
        io.fuel = 1;
        let mut cannon = Cannon::new([0, 0, 0]);
        cannon.skip_missing = true;
        let mut printer = None;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        cannon.play();
        run(&mut cannon, &mut printer, &mut io, 2, 100);
        assert_eq!(io.world.len(), 1);
        assert_eq!(io.world.get(&[11, 0, 10]).map(|s| s.name.as_str()), Some("minecraft:dirt"));
        assert_eq!(cannon.status, "finished");
    }

    #[test]
    fn no_gunpowder_pauses_and_gunpowder_resumes() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone")]);
        io.stock.insert("minecraft:stone".into(), 5);
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        cannon.play();
        run(&mut cannon, &mut printer, &mut io, 2, 5);
        assert_eq!(cannon.status, "noGunpowder");
        assert_eq!(cannon.state, RunState::Paused);
        io.fuel = 1;
        run(&mut cannon, &mut printer, &mut io, 7, 30);
        // It went on by itself once powder arrived (`blocksPlaced` was 0, so it says ready: played again).
        assert_eq!(cannon.status, "ready");
        cannon.play();
        run(&mut cannon, &mut printer, &mut io, 37, 60);
        assert_eq!(cannon.status, "finished");
    }

    #[test]
    fn an_unloaded_target_waits() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone")]);
        io.stock.insert("minecraft:stone".into(), 5);
        io.fuel = 1;
        io.unloaded.push([10, 0, 10]);
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        cannon.play();
        run(&mut cannon, &mut printer, &mut io, 2, 5);
        assert_eq!(cannon.status, "targetNotLoaded");
        io.unloaded.clear();
        run(&mut cannon, &mut printer, &mut io, 7, 60);
        assert_eq!(io.world.len(), 1);
    }

    #[test]
    fn a_far_anchor_and_a_bare_slot_are_refused() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone")]);
        io.anchor = [300, 0, 0];
        let mut cannon = Cannon::new([0, 0, 0]);
        let mut printer = None;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        assert_eq!(cannon.status, "targetOutsideRange");
        assert_eq!(cannon.state, RunState::Stopped);
        let mut io = Fake::new(&[]);
        io.blueprint = Blueprint::NotDeployed;
        let mut cannon = Cannon::new([0, 0, 0]);
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 0, 2);
        assert_eq!(cannon.status, "schematicNotPlaced");
        io.blueprint = Blueprint::Empty;
        cannon.state = RunState::Paused;
        run(&mut cannon, &mut printer, &mut io, 2, 2);
        assert_eq!(cannon.status, "idle");
    }

    #[test]
    fn a_slot_emptied_while_running_gives_no_free_schematic() {
        let mut io = Fake::new(&[([0, 0, 0], "minecraft:stone")]);
        io.blueprint = Blueprint::Empty;
        let mut cannon = Cannon::new([0, 0, 0]);
        cannon.state = RunState::Running;
        let mut printer = None;
        let effects = run(&mut cannon, &mut printer, &mut io, 0, 1);
        assert_eq!(cannon.status, "schematicInvalid");
        assert_eq!(cannon.state, RunState::Stopped);
        // The next tick settles it at idle, as Create's does.
        let effects = [effects, run(&mut cannon, &mut printer, &mut io, 1, 2)].concat();
        assert_eq!(cannon.status, "idle");
        assert!(!effects.iter().any(|e| matches!(e, Effect::ClearBlueprint { .. })));
    }

    #[test]
    fn replace_modes_decide_what_is_overwritten() {
        let stone = State::new("minecraft:stone");
        let dirt = State::new("minecraft:dirt");
        let glass = State::new("minecraft:glass");
        let air = State::new("minecraft:air");
        let candidate = |wanted: &State, there: &State| Candidate {
            target: [10, 0, 10],
            state: wanted.clone(),
            to_replace: Existing { state: there.clone(), has_block_entity: false },
            to_replace_other: None,
            is_normal_cube: conductor(wanted),
        };
        let ok = |_: &str| true;
        let mut cannon = Cannon::new([0, 0, 0]);
        // Replace any (the default): anything but air is placed over anything.
        assert!(cannon.should_place(&candidate(&stone, &dirt), &ok));
        assert!(!cannon.should_place(&candidate(&air, &dirt), &ok));
        // Don't replace: only into what isn't a solid block.
        cannon.replace_mode = 0;
        assert!(!cannon.should_place(&candidate(&stone, &dirt), &ok));
        assert!(cannon.should_place(&candidate(&stone, &glass), &ok));
        assert!(cannon.should_place(&candidate(&stone, &air), &ok));
        // Replace solid with solid.
        cannon.replace_mode = 1;
        assert!(cannon.should_place(&candidate(&stone, &dirt), &ok));
        assert!(!cannon.should_place(&candidate(&glass, &dirt), &ok));
        // Replace with empty: air clears.
        cannon.replace_mode = 3;
        assert!(cannon.should_place(&candidate(&air, &dirt), &ok));
        // Blocks right at the cannon are left alone, and block entities too unless asked.
        let mut near = candidate(&stone, &air);
        near.target = [1, 0, 1];
        assert!(!cannon.should_place(&near, &ok));
        let mut chest = candidate(&stone, &State::new("minecraft:chest"));
        chest.to_replace.has_block_entity = true;
        assert!(!cannon.should_place(&chest, &ok));
        cannon.replace_block_entities = true;
        assert!(cannon.should_place(&chest, &ok));
    }

    #[test]
    fn the_aim_and_the_flight_are_create_s() {
        // A target due south and level: yaw 0.
        let (yaw, pitch) = cannon_angles([0, 0, 0], Some([0, 0, 10]), None, 0.0, 90.0);
        assert!(yaw.abs() < 1e-9);
        // atan2(10, 0 + 20) = 26.565 degrees, plus 10.
        assert!((pitch - (10f64.atan2(20.0).to_degrees() + 10.0)).abs() < 1e-9);
        // To the east: yaw 90.
        let (yaw, _) = cannon_angles([0, 0, 0], Some([10, 0, 0]), None, 0.0, 0.0);
        assert!((yaw - 90.0).abs() < 1e-9);
        // With no target it rests at its yaw and 40 degrees.
        assert_eq!(cannon_angles([0, 0, 0], None, None, 0.0, 135.0), (135.0, 40.0));
        // The previous target blends in.
        let (yaw, _) = cannon_angles([0, 0, 0], Some([0, 0, 10]), Some([10, 0, 0]), 0.5, 0.0);
        assert!((yaw - 45.0).abs() < 1e-9);
        // `ticksForDistance`: ten at least, `dist^(1/2)` times four otherwise.
        assert_eq!(Flying::ticks_for_distance([0, 0, 0], [3, 0, 0]), 10);
        assert_eq!(Flying::ticks_for_distance([0, 0, 0], [0, 0, 100]), 40);
        let flying = Flying {
            launched: 100,
            total_ticks: 40,
            target: [0, 0, 100],
            icon: "minecraft:stone".into(),
            state: State::new("minecraft:stone"),
            data: None,
        };
        assert_eq!(flying.remaining(100), 40);
        assert_eq!(flying.remaining(139), 1);
        assert!(flying.landed(140));
        assert!(flight_pose([0, 0, 0], &flying, 140, 0.0).is_none());
        // Early on it is near the cannon, near the end near the target (z 100.5 relative).
        let early = flight_pose([0, 0, 0], &flying, 101, 0.0).unwrap().0;
        let late = flight_pose([0, 0, 0], &flying, 139, 0.0).unwrap().0;
        assert!(early[2] < 10.0 && late[2] > 90.0, "{early:?} {late:?}");
        // The recoil is strongest at the shot and gone ten ticks later.
        let shots = [flying.clone()];
        assert!((recoil(&shots, 100, 0.0) - 10.0).abs() < 1e-9 + 1.0);
        assert_eq!(recoil(&shots, 120, 0.0), 0.0);
    }

    #[test]
    fn only_safe_block_entity_data_is_copied() {
        let mut data = Nbt::compound();
        data.put("GeneratedSpeed", Nbt::Float(16.0));
        data.put("Items", Nbt::List(Vec::new()));
        let motor = State::new("create:creative_motor");
        let kept = safe_block_entity_data(&motor, &data).unwrap();
        assert!(kept.get("GeneratedSpeed").is_some());
        assert!(kept.get("Items").is_none());
        assert!(safe_block_entity_data(&State::new("minecraft:chest"), &data).is_none());
        assert!(safe_block_entity_data(&State::new("minecraft:oak_sign"), &data).is_some());
    }
}

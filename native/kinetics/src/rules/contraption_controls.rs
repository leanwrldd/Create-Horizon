//! The Contraption Controls block as it stands in the world (`ContraptionControlsBlock`,
//! `ContraptionControlsBlockEntity`, `ContraptionControlsRenderer`): a click toggles it, redstone
//! power turns it off, it keeps a filter naming a kind of actor, and the button and indicator
//! show which.
//!
//! What it is *for* is to switch actors (drills, saws, harvesters...) on and off on a moving
//! contraption by that filter (`ContraptionControlsMovement`, `Contraption.isActorTypeDisabled`).
//! This port has no actors on contraptions yet, so the block keeps the state and the filter in
//! its block entity, in Create's own keys (`Disabled`, `Powered`, `Filter`), as an assembled
//! contraption would carry them, and does nothing more.
//!
//! Known differences from Create, for the README:
//! - Nothing reads the state: no actor can be switched. The block's Ponder scene and the
//!   `ElevatorContraption` floor display are likewise not here.
//! - Create sets the filter in a value box (the item's name shows beside it); here an item that
//!   is in the `create:contraption_controlled` tag, right-clicked on the block, sets it (a
//!   sneaking empty hand clears it).
//! - No waterlogging, and the button's spring is the same exponential ease from where it was
//!   pressed, worked out from the game time of the click.
//! - The comparator doesn't read it: Create's block has no analogue output either.

/// `LerpedFloat.chase(0, 0.125, EXP)`: the button climbs back up by 1/8 of the way each tick.
pub const BUTTON_SPEED: f64 = 0.125;
/// `-1 / 24 * button.getValue()` blocks: how far a pressed button sinks.
pub const BUTTON_DEPTH: f32 = 1.0 / 24.0;
/// The indicator turns to `4 * 45` degrees when off (`ContraptionControlsBlockEntity.tick`).
pub const OFF_ANGLE: i32 = 4 * 45;
/// `AllShapes.CONTRAPTION_CONTROLS`' boxes for a block facing north (texels), and the collision
/// boxes (`CONTRAPTION_CONTROLS_COLLISION`).
pub const SHAPE: [[f32; 6]; 5] = [
    [0.0, 0.0, 6.0, 2.0, 16.0, 16.0],
    [14.0, 0.0, 6.0, 16.0, 16.0, 16.0],
    [0.0, 0.0, 14.0, 16.0, 16.0, 16.0],
    [0.0, 0.0, 6.0, 16.0, 12.0, 16.0],
    [0.0, 0.0, 4.0, 16.0, 2.0, 16.0],
];
pub const COLLISION: [[f32; 6]; 4] = [
    [0.0, 0.0, 6.0, 2.0, 16.0, 16.0],
    [14.0, 0.0, 6.0, 16.0, 16.0, 16.0],
    [0.0, 0.0, 14.0, 16.0, 16.0, 16.0],
    [0.0, 0.0, 7.0, 16.0, 12.0, 16.0],
];

/// The block entity's two flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// `disabled`: the actors it targets are off.
    pub disabled: bool,
    /// `powered`: redstone power was on it at the last look.
    pub powered: bool,
}

impl Controls {
    /// `useWithoutItem`: a click flips `disabled`.
    pub fn clicked(self) -> Controls {
        Controls {
            disabled: !self.disabled,
            ..self
        }
    }

    /// `updatePoweredState`: when the signal changes, the block follows it (power off, no power
    /// on). `None` when nothing changed.
    pub fn power_update(self, signal: bool) -> Option<Controls> {
        (self.powered != signal).then_some(Controls {
            disabled: signal,
            powered: signal,
        })
    }

    /// The indicator model to draw (`indicator_0` to `indicator_7`): the dial sits at the angle
    /// `value / 45`.
    pub fn indicator(self) -> usize {
        let angle = if self.disabled { OFF_ANGLE } else { 0 };
        ((angle / 45) % 8) as usize
    }
}

/// How far the button is pressed `ticks` after a click: 1 at the click, easing to 0.
pub fn button_value(ticks: f64) -> f32 {
    if ticks < 0.0 {
        return 0.0;
    }
    (1.0 - BUTTON_SPEED).powf(ticks) as f32
}

/// Whether the button still moves visibly (for keeping the drawing).
pub fn button_at_rest(ticks: f64) -> bool {
    button_value(ticks) < 1.0e-3
}

/// `sendStatus`: the action bar line for a toggle. `filter` is the filter item's name.
pub fn status(filter: Option<&str>, enabled: bool) -> String {
    let state = if enabled { "On" } else { "Off" };
    match filter {
        None => format!("All Actors: {state}"),
        Some(name) => format!("{name} Actors: {state}"),
    }
}

/// The item tag a filter must be in (`filtering.withPredicate(CONTRAPTION_CONTROLLED::matches)`).
pub const FILTER_TAG: &str = "create:contraption_controlled";

/// The language keys of the status line and their English, for a message the client words
/// itself: `[(key, english) of the line, (key, english) of the state]`. The line's `%1$s` is the
/// state (`%1$s` the filter's name and `%2$s` the state for a named one).
pub fn status_keys(filtered: bool, enabled: bool) -> [(&'static str, &'static str); 2] {
    [
        if filtered {
            ("create.contraption.controls.specific_actor_toggle", "%1$s Actors: %2$s")
        } else {
            ("create.contraption.controls.all_actor_toggle", "All Actors: %1$s")
        },
        if enabled {
            ("create.contraption.controls.actor_toggle.on", "On")
        } else {
            ("create.contraption.controls.actor_toggle.off", "Off")
        },
    ]
}

/// The colour of "On" and "Off" in the status (`DyeColor.LIME` and `ORANGE`'s first colour).
pub fn status_colour(enabled: bool) -> u32 {
    if enabled { 0x80_C7_1F } else { 0xF9_80_1D }
}

/// The language key of an item that is a block (every item the filter takes is one): the name
/// Create shows for the filter (`ItemStack.getHoverName`).
pub fn block_item_key(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("block.{namespace}.{}", path.replace('/', "."))
}

/// `ContraptionControlsMovement.isSameFilter`.
pub fn same_filter(a: Option<&str>, b: Option<&str>) -> bool {
    a == b
}

/// The sound of a click (`AllSoundEvents.CONTROLLER_CLICK`: the item frame's `add_item`, volume
/// 0.35, pitch 1) played at pitch 0.8 turning off and 1.5 turning on.
pub fn click_pitch(disabled: bool) -> f32 {
    if disabled { 0.8 } else { 1.5 }
}

/// The blockstate's turn of the model about y, in degrees clockwise seen from above, by the
/// facing it names (`blockstates/contraption_controls.json`: north none, east 90, south 180,
/// west 270).
pub fn model_turn(facing: &str) -> f32 {
    match facing {
        "east" => 90.0,
        "south" => 180.0,
        "west" => 270.0,
        _ => 0.0,
    }
}

/// The direction the button sinks in, for a block facing `facing`: `(0, 1, -0.325)` turned like
/// the model (`ContraptionControlsRenderer`), not normalised.
pub fn button_axis(facing: &str) -> [f32; 3] {
    // Turning by y degrees clockwise seen from above takes north (-z) towards east (+x).
    let turn = model_turn(facing).to_radians();
    let (sin, cos) = turn.sin_cos();
    let (x, z) = (0.0f32, -0.325f32);
    [x * cos - z * sin, 1.0, x * sin + z * cos]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_flips_it_and_power_overrides() {
        let on = Controls::default();
        assert!(!on.disabled);
        let off = on.clicked();
        assert!(off.disabled && !off.powered);
        assert_eq!(off.clicked(), on);
        // Power turns it off, and taking the power away turns it back on.
        let powered = on.power_update(true).unwrap();
        assert_eq!(powered, Controls { disabled: true, powered: true });
        assert_eq!(powered.power_update(true), None);
        assert_eq!(powered.power_update(false), Some(Controls::default()));
        // A click while powered flips only `disabled`, which the signal's next change overwrites.
        assert_eq!(
            powered.clicked(),
            Controls { disabled: false, powered: true }
        );
        // An unpowered, switched-off block stays off until it is clicked or powered.
        assert_eq!(off.power_update(false), None);
    }

    #[test]
    fn the_indicator_turns_a_half_turn_when_off() {
        assert_eq!(Controls::default().indicator(), 0);
        assert_eq!(Controls { disabled: true, powered: false }.indicator(), 4);
    }

    #[test]
    fn the_button_eases_back_by_an_eighth_each_tick() {
        assert_eq!(button_value(0.0), 1.0);
        assert!((button_value(1.0) - 0.875).abs() < 1e-6);
        assert!((button_value(2.0) - 0.765_625).abs() < 1e-6);
        assert_eq!(button_value(-3.0), 0.0);
        assert!(!button_at_rest(10.0));
        assert!(button_at_rest(80.0));
    }

    #[test]
    fn the_status_line_names_the_filter() {
        assert_eq!(status(None, true), "All Actors: On");
        assert_eq!(status(Some("Mechanical Drill"), false), "Mechanical Drill Actors: Off");
        assert_ne!(status_colour(true), status_colour(false));
        assert_eq!(status_keys(false, true)[1].1, "On");
        assert_eq!(status_keys(true, false)[0].1, "%1$s Actors: %2$s");
        assert_eq!(block_item_key("create:mechanical_drill"), "block.create.mechanical_drill");
        assert_eq!(block_item_key("minecraft:bell"), "block.minecraft.bell");
        assert!(click_pitch(true) < click_pitch(false));
        assert!(same_filter(None, None));
        assert!(!same_filter(Some("a"), None));
    }

    #[test]
    fn the_button_sinks_along_the_way_the_block_faces() {
        let north = button_axis("north");
        assert!((north[2] + 0.325).abs() < 1e-6 && north[0].abs() < 1e-6);
        let east = button_axis("east");
        assert!((east[0] - 0.325).abs() < 1e-6, "{east:?}");
        let south = button_axis("south");
        assert!((south[2] - 0.325).abs() < 1e-6);
        assert_eq!(model_turn("west"), 270.0);
        assert_eq!(SHAPE.len(), 5);
    }
}

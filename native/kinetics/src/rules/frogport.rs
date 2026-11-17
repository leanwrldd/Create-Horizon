//! The Package Frogport and the Postbox (`PackagePortBlockEntity`, `FrogportBlockEntity`,
//! `FrogportRenderer`, `PackagePortAutomationInventoryWrapper`, `PostboxBlockEntity`): a port
//! holds up to 18 packages, catches the ones a Chain Conveyor brings that match its address,
//! and sends the ones that don't out along the chain. No game types: `B` is a package. The glue
//! is `machines/frogport.rs` and `machines/postbox.rs`; the chain is `rules::chain_conveyor`.
//!
//! Known differences from Create, for the README:
//! - Create sets a Frogport's chain point by clicking on the chain (a client selection); here a
//!   Frogport attaches to the nearest point of the conveyors in range when it is placed, and
//!   the Wrench picks the nearest point again (`rules::chain_conveyor::nearest_target`).
//! - The port's screen (address, "accepts packages" switch) is not here: the address comes from
//!   a Name Tag, and a port always accepts packages. A Frogport's head doesn't open when it is
//!   opened by hand or when a package is about to arrive (`anticipate`).
//! - The Postbox's train station link isn't there (trains come last).

/// `PackagePortBlockEntity.inventory`: `SmartInventory(18)`.
pub const SLOTS: usize = 18;

/// `LerpedFloat.chase(1, 0.1, LINEAR)`: the animation's progress per tick, 10 ticks in all.
pub const PROGRESS_PER_TICK: f32 = 0.1;
/// The length of the animation in ticks.
pub const ANIMATION_TICKS: i64 = 10;

/// `lazyTick`'s period.
pub const LAZY_PERIOD: i64 = 10;

/// `PackagePortAutomationInventoryWrapper.extractItem`: automation (a hopper, a funnel) takes
/// only packages that match the port's own address: the ones that have arrived.
pub fn automation_can_extract(is_package: bool, address_matches: Option<bool>) -> bool {
    is_package && address_matches == Some(true)
}

/// `PackagePortAutomationInventoryWrapper.insertItem`: automation puts in only packages that do
/// not match the port's address: the ones to send.
pub fn automation_can_insert(is_package: bool, address_matches: Option<bool>) -> bool {
    is_package && address_matches != Some(true)
}

/// `tryPullingFrom`'s test: a package the port sends away (it doesn't match its own address),
/// or any package from a Packager (`handler instanceof PackagerItemHandler`) or when the port has
/// no filter.
pub fn pulls(is_package: bool, filter_matches: Option<bool>, from_packager: bool) -> bool {
    is_package && (filter_matches.is_none() || from_packager || filter_matches == Some(false))
}

/// `PackagePortBlockEntity.getComparatorOutput`: `ItemHandlerHelper.calcRedstoneFromInventory`
/// for a port whose slots each hold one package (a package stacks to one).
pub fn comparator(filled: usize) -> u8 {
    if filled == 0 {
        return 0;
    }
    let fraction = filled as f32 / SLOTS as f32;
    (fraction * 14.0).floor() as u8 + 1
}

/// `FrogportBlock.setPlacedBy`: the yaw a placed port faces, towards the placer, to 11.25 degrees.
pub fn placed_yaw(dx: f64, dz: f64) -> f32 {
    let yaw = dx.atan2(dz).to_degrees() as f32;
    (yaw / 11.25).round() * 11.25
}

/// `FrogportBlockEntity.getYaw` with a target: towards it, turned round.
pub fn yaw_towards(dx: f64, dz: f64) -> f32 {
    dx.atan2(dz).to_degrees() as f32 + 180.0
}

/// `FrogportRenderer`'s pose of the tongue, head and package for an animation `progress`
/// (0 to 1), `depositing` or catching, to a target `diff` away from the port's centre (its y
/// already lowered by 0.75 where the renderer lowers it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub tongue_pitch: f32,
    pub tongue_length: f32,
    pub head_pitch: f32,
    /// How far along the tongue the package is.
    pub item_distance: f32,
    /// The package's scale.
    pub scale: f32,
}

/// The renderer's maths. `diff` is the vector to the target, from the port's centre, as the
/// renderer takes it. `progress` is `None` when nothing is happening (the head at rest).
pub fn pose(diff: [f64; 3], progress: Option<f32>, depositing: bool, opened_by_hand: f32) -> Pose {
    let horizontal = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt();
    let norm = (horizontal * horizontal + diff[1] * diff[1]).sqrt() as f32;
    let tongue_pitch = (diff[1].atan2(horizontal + 3.0 / 16.0)).to_degrees() as f32;
    let mut tongue_length = norm.max(1.0);
    let head_pitch = (tongue_pitch * 2.0).clamp(60.0, 100.0);
    let (modifier, item_distance, scale);
    match progress {
        Some(progress) if depositing => {
            let m = (1.0 - ((progress - 0.25) * 4.0 - 1.0).powi(4)).max(0.0);
            item_distance = (tongue_length * (1.0f32).min((progress - 0.25) * 3.0)).max(tongue_length * m);
            tongue_length *= (1.0 - ((progress * 1.25 - 0.25) * 4.0 - 1.0).powi(4)).max(0.0);
            modifier = (1.0 - ((progress * 1.25) * 2.0 - 1.0).powi(4)).max(0.0);
            scale = 0.25 + progress * 3.0 / 4.0;
        }
        Some(progress) => {
            tongue_length *= (1.0 - progress * 1.25).max(0.0).powi(5);
            modifier = 1.0 - (((progress * 1.5).powi(2) - 0.5) * 2.0).clamp(0.0, 1.0);
            scale = (1.0 - progress * 1.25).max(0.5);
            item_distance = tongue_length;
        }
        None => {
            tongue_length = 0.0;
            modifier = 0.0;
            scale = 1.0;
            item_distance = 0.0;
        }
    }
    Pose {
        tongue_pitch: tongue_pitch,
        tongue_length: tongue_length.max(opened_by_hand * 0.25),
        head_pitch: (head_pitch * modifier).max(opened_by_hand * 60.0),
        item_distance,
        scale,
    }
}

/// What a tick of the port's animation did.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect<B> {
    /// Halfway through sending: the package goes to the target (or drops if there is none or it
    /// refuses it).
    Send(B),
    /// The catch is over: the package goes into the inventory (or drops if it is full).
    Receive(B),
}

/// The animation in progress (`animatedPackage`, `currentlyDepositing`, `animationProgress`).
#[derive(Clone, Debug, PartialEq)]
pub struct Animation<B> {
    pub start: i64,
    pub depositing: bool,
    pub item: Option<B>,
}

/// A port's state: its packages and its animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Port<B> {
    pub slots: Vec<Option<B>>,
    pub animation: Option<Animation<B>>,
    /// `failedLastExport`.
    pub failed_last_export: bool,
}

impl<B> Default for Port<B> {
    fn default() -> Self {
        Self { slots: (0..SLOTS).map(|_| None).collect(), animation: None, failed_last_export: false }
    }
}

impl<B: Clone> Port<B> {
    /// `isBackedUp`: no free slot.
    pub fn is_backed_up(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }

    pub fn filled(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// `isAnimationInProgress`.
    pub fn animating(&self, now: i64) -> bool {
        self.animation.as_ref().is_some_and(|a| now - a.start < ANIMATION_TICKS)
    }

    /// The animation's progress, 0 to 1, `partial` ticks after `now`.
    pub fn progress(&self, now: i64, partial: f32) -> Option<f32> {
        let animation = self.animation.as_ref()?;
        let progress = ((now - animation.start) as f32 + partial) * PROGRESS_PER_TICK;
        (progress < 1.0).then_some(progress.max(0.0))
    }

    /// `startAnimation(box, deposit)`: the caller has checked the target (`deposit` needs one).
    pub fn start_animation(&mut self, item: B, depositing: bool, now: i64) {
        self.animation = Some(Animation { start: now, depositing, item: Some(item) });
    }

    /// The first slot that holds a box the port should send (see [`pulls`]); `test` is asked
    /// of each box.
    pub fn first_slot(&self, test: &mut dyn FnMut(&B) -> bool) -> Option<usize> {
        self.slots.iter().position(|slot| slot.as_ref().is_some_and(&mut *test))
    }

    /// `ItemHandlerHelper.insertItem` of a package (one per slot): the first free slot.
    pub fn insert(&mut self, item: B) -> Result<(), B> {
        match self.slots.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => {
                *slot = Some(item);
                Ok(())
            }
            None => Err(item),
        }
    }

    /// `tick`: the animation's halfway point (a package being sent leaves) and its end (a package
    /// being caught arrives).
    pub fn tick(&mut self, now: i64) -> Vec<Effect<B>> {
        let mut effects = Vec::new();
        let Some(animation) = self.animation.as_mut() else {
            return effects;
        };
        let value = ((now - animation.start) as f32 * PROGRESS_PER_TICK).min(1.0);
        if animation.depositing && value > 0.5 {
            if let Some(item) = animation.item.take() {
                effects.push(Effect::Send(item));
            }
        }
        if value >= 1.0 {
            if !animation.depositing {
                if let Some(item) = animation.item.take() {
                    effects.push(Effect::Receive(item));
                }
            }
            self.animation = None;
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_takes_what_arrived_and_gives_what_is_to_go() {
        // A box for this port's own address: it can be taken out, not put in.
        assert!(automation_can_extract(true, Some(true)));
        assert!(!automation_can_insert(true, Some(true)));
        // Another address: put in to be sent, not taken out.
        assert!(!automation_can_extract(true, Some(false)));
        assert!(automation_can_insert(true, Some(false)));
        // No filter at all takes any package in and gives none out.
        assert!(automation_can_insert(true, None));
        assert!(!automation_can_extract(true, None));
        // Nothing else but packages.
        assert!(!automation_can_insert(false, Some(false)));
        assert!(!automation_can_extract(false, Some(true)));
    }

    #[test]
    fn a_port_sends_the_boxes_that_are_not_its_own() {
        assert!(pulls(true, Some(false), false));
        assert!(!pulls(true, Some(true), false));
        assert!(pulls(true, None, false));
        // Whatever a packager holds is sent.
        assert!(pulls(true, Some(true), true));
        assert!(!pulls(false, Some(false), true));
    }

    #[test]
    fn the_comparator_follows_the_inventory() {
        assert_eq!(comparator(0), 0);
        assert_eq!(comparator(1), 1);
        assert_eq!(comparator(9), 8);
        assert_eq!(comparator(18), 15);
    }

    #[test]
    fn a_placed_port_faces_the_placer_in_steps_of_11_and_a_quarter_degrees() {
        assert_eq!(placed_yaw(0.0, 1.0), 0.0);
        assert_eq!(placed_yaw(1.0, 0.0), 90.0);
        assert_eq!(placed_yaw(1.0, 1.0), 45.0);
        assert_eq!(placed_yaw(1.0, 8.0), 11.25);
        assert!((yaw_towards(0.0, 1.0) - 180.0).abs() < 1e-4);
    }

    #[test]
    fn the_tongue_reaches_out_and_back_while_sending() {
        let diff = [0.0, 2.0, 3.0];
        let start = pose(diff, Some(0.0), true, 0.0);
        let middle = pose(diff, Some(0.5), true, 0.0);
        let end = pose(diff, Some(0.95), true, 0.0);
        assert!(start.tongue_length < 0.5);
        assert!(middle.tongue_length > start.tongue_length);
        assert!(middle.item_distance > 3.0);
        assert!(end.tongue_length < middle.tongue_length);
        // The package grows as it is sent out.
        assert!(end.scale > start.scale);
        // At rest: no tongue, head up.
        let rest = pose(diff, None, false, 0.0);
        assert_eq!((rest.tongue_length, rest.head_pitch), (0.0, 0.0));
        // Held open by a click.
        assert_eq!(pose(diff, None, false, 1.0).head_pitch, 60.0);
    }

    #[test]
    fn a_caught_box_arrives_at_the_end_and_a_sent_one_leaves_at_the_middle() {
        let mut port: Port<&str> = Port::default();
        port.start_animation("out", true, 100);
        assert!(port.animating(105));
        assert!(port.tick(104).is_empty());
        // After the half: it leaves, once.
        assert_eq!(port.tick(106), vec![Effect::Send("out")]);
        assert!(port.tick(108).is_empty());
        assert!(port.tick(110).is_empty());
        assert!(!port.animating(110));
        assert_eq!(port.animation, None);

        port.start_animation("in", false, 200);
        assert!(port.tick(205).is_empty());
        assert_eq!(port.tick(210), vec![Effect::Receive("in")]);
        assert_eq!(port.animation, None);
    }

    #[test]
    fn a_full_port_is_backed_up_and_takes_a_box_in_the_first_free_slot() {
        let mut port: Port<&str> = Port::default();
        assert!(!port.is_backed_up());
        assert_eq!(port.insert("a"), Ok(()));
        assert_eq!(port.slots[0], Some("a"));
        for _ in 1..SLOTS {
            port.insert("x").unwrap();
        }
        assert!(port.is_backed_up());
        assert_eq!(port.insert("late"), Err("late"));
        assert_eq!(port.filled(), SLOTS);
        port.slots[3] = None;
        assert_eq!(port.first_slot(&mut |b| *b == "x"), Some(1));
    }

    #[test]
    fn progress_is_known_while_the_animation_runs() {
        let mut port: Port<&str> = Port::default();
        assert_eq!(port.progress(0, 0.0), None);
        port.start_animation("a", true, 10);
        assert_eq!(port.progress(10, 0.0), Some(0.0));
        assert!((port.progress(15, 0.5).unwrap() - 0.55).abs() < 1e-5);
        assert_eq!(port.progress(20, 0.0), None);
    }
}

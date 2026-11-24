//! Handing a Schedule to a train's conductor: `ScheduleItem.handScheduleTo`, `ScheduleItemEntityInteraction`
//! (a click on a seated conductor), `ConductorBlockInteractionBehavior.handlePlayerInteraction` (a click on the Blaze
//! Burner that conducts), and the Train Hat's rules (`EntityHats`, `BlazeBurnerMovementBehaviour`,
//! `BlazeBurnerRenderer.renderShared`). No game types: the glue says what it found, and applies the answer.
//!
//! The messages are Create's English (`assets/create/lang/en_us.json`); the sounds are `AllSoundEvents.CONFIRM`,
//! `DENY` and `playItemPickup`.

/// The lines the player is told (`player.displayClientMessage(.., true)`, the action bar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// `schedule.continued`.
    Continued,
    /// `schedule.remove_with_empty_hand`.
    RemoveWithEmptyHand,
    /// `schedule.auto_removed_from_train`.
    AutoRemoved,
    /// `schedule.removed_from_train`.
    Removed,
    /// `schedule.non_controlling_seat`.
    NonControllingSeat,
    /// `schedule.no_stops`.
    NoStops,
    /// `schedule.applied_to_train` (shown green).
    Applied,
}

impl Message {
    pub fn key(self) -> &'static str {
        match self {
            Message::Continued => "create.schedule.continued",
            Message::RemoveWithEmptyHand => "create.schedule.remove_with_empty_hand",
            Message::AutoRemoved => "create.schedule.auto_removed_from_train",
            Message::Removed => "create.schedule.removed_from_train",
            Message::NonControllingSeat => "create.schedule.non_controlling_seat",
            Message::NoStops => "create.schedule.no_stops",
            Message::Applied => "create.schedule.applied_to_train",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Message::Continued => "Schedule resumed",
            Message::RemoveWithEmptyHand => "Remove current Schedule with an Empty Hand",
            Message::AutoRemoved => "Auto-Schedule discarded",
            Message::Removed => "Schedule retrieved from Train",
            Message::NonControllingSeat => "Conductor needs to sit in front of a Controls block",
            Message::NoStops => "This Schedule does not have any Stops yet",
            Message::Applied => "Train is now following this Schedule",
        }
    }
}

/// `AllSoundEvents.CONFIRM`, `DENY` and `playItemPickup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Confirm,
    Deny,
    ItemPickup,
}

/// What the train's schedule runtime says (`train.runtime`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeView {
    /// `getSchedule() != null`.
    pub has_schedule: bool,
    pub paused: bool,
    pub completed: bool,
    /// `isAutoSchedule`.
    pub auto: bool,
}

/// What the glue found when a Schedule was held out to something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// `getRootVehicle() instanceof CarriageContraptionEntity`.
    pub on_carriage: bool,
    /// The carriage's train, if it has one (`getCarriage().train != null`).
    pub train: Option<RuntimeView>,
    /// `contraption.getSeatMapping().get(uuid) != null`: the entity sits on one of the carriage's seats.
    pub seated: bool,
    /// `cc.conductorSeats.get(seatPos) != null`: that seat is in front of Train Controls.
    pub conductor_seat: bool,
}

/// `ScheduleItem.handScheduleTo`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    /// `InteractionResult.PASS`: not for this item (the click goes on).
    Pass,
    /// `SUCCESS`, nothing else happens.
    Nothing,
    /// `SUCCESS` after the deny sound and this message.
    Deny(Message),
    /// The train takes the Schedule (`setSchedule(schedule, false)`), the item shrinks by one, the confirm sound and
    /// `schedule.applied_to_train` follow.
    Apply,
}

impl Hand {
    /// `result.consumesAction()`.
    pub fn consumes(self) -> bool {
        self != Hand::Pass
    }
}

/// `ScheduleItem.handScheduleTo`. `stops` is the held item's schedule (`None` when the stack holds none).
pub fn hand_schedule_to(stops: Option<usize>, server: bool, target: &Target) -> Hand {
    let Some(stops) = stops else { return Hand::Pass };
    if !target.on_carriage {
        return Hand::Pass;
    }
    if !server {
        return Hand::Nothing;
    }
    let Some(train) = target.train else { return Hand::Nothing };
    if !target.seated {
        return Hand::Nothing;
    }
    if !target.conductor_seat {
        return Hand::Deny(Message::NonControllingSeat);
    }
    if train.has_schedule {
        return Hand::Deny(Message::RemoveWithEmptyHand);
    }
    if stops == 0 {
        return Hand::Deny(Message::NoStops);
    }
    Hand::Apply
}

/// What `ScheduleItemEntityInteraction.interactWithConductor` found out of the click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityClick {
    pub spectator: bool,
    /// `entity instanceof LivingEntity`.
    pub living: bool,
    /// The Schedule item is on cooldown for the player.
    pub on_cooldown: bool,
    pub main_hand: bool,
    pub hand_empty: bool,
    pub server: bool,
    /// Whether the held item is the Schedule item, and its stops.
    pub held_schedule: Option<usize>,
    pub target: Target,
}

/// The effect of a click on a carriage's passenger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// The click goes on (the event is not cancelled).
    Ignore,
    /// The held Schedule was handed over, or refused: the click is cancelled with this result, and the Schedule item is
    /// on cooldown for 5 ticks.
    Handed(Hand),
    /// A paused train goes on (`runtime.paused = false`, confirm sound, `schedule.continued`): cooldown 5.
    Resume,
    /// A Schedule is on the train and the hand isn't empty: deny sound and `schedule.remove_with_empty_hand`; cancelled, no cooldown.
    Refuse,
    /// The empty hand takes the Schedule back (`returnSchedule`, pickup sound, the removal message): cooldown 5. `auto`: an
    /// Auto-Schedule is thrown away (`returnSchedule` gives nothing back).
    Return { auto: bool },
}

/// `ScheduleItemEntityInteraction.interactWithConductor`.
pub fn entity_click(c: &EntityClick) -> Reaction {
    if c.spectator || !c.target.on_carriage || !c.living || c.on_cooldown {
        return Reaction::Ignore;
    }
    if c.held_schedule.is_some() {
        let result = hand_schedule_to(c.held_schedule, c.server, &c.target);
        if result.consumes() {
            return Reaction::Handed(result);
        }
    }
    if !c.main_hand {
        return Reaction::Ignore;
    }
    let Some(train) = c.target.train else { return Reaction::Ignore };
    if !train.has_schedule || !c.target.seated || !c.target.conductor_seat {
        return Reaction::Ignore;
    }
    if train.paused && !train.completed {
        return Reaction::Resume;
    }
    if !c.hand_empty {
        return Reaction::Refuse;
    }
    Reaction::Return { auto: train.auto }
}

/// What a click on the Blaze Burner of a carriage does (`ConductorBlockInteractionBehavior.handlePlayerInteraction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockClick {
    pub main_hand: bool,
    /// The burner's state is a valid conductor (`HEAT_LEVEL != NONE`).
    pub valid: bool,
    /// `carriageContraption.inControl(localPos, direction)` for either direction along the carriage.
    pub in_control: bool,
    pub train: Option<RuntimeView>,
    pub server: bool,
    pub hand_empty: bool,
    /// The held item is the Schedule item and its stops (`None`: not a Schedule item, or it holds none).
    pub held_schedule: Option<usize>,
    pub holds_schedule_item: bool,
}

/// The block's answer: whether the click was taken (`handlePlayerInteraction`'s boolean) and what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReaction {
    /// `false`: the click isn't taken.
    NotTaken,
    /// `true`, nothing else.
    Taken,
    /// `true`, with the deny sound and a message.
    Deny(Message),
    /// A paused train goes on (`schedule.continued`).
    Resume,
    /// The Schedule goes back into the hand (`setItemInHand(returnSchedule())`).
    Return { auto: bool },
    /// The train takes the held Schedule and the stack shrinks.
    Apply,
}

pub fn block_click(c: &BlockClick) -> BlockReaction {
    if !c.main_hand || !c.valid {
        return BlockReaction::NotTaken;
    }
    if !c.in_control {
        // The loop over the directions found no controls: `schedule.non_controlling_seat` (the client's click is
        // answered by the server's, as Java's is: the message is sent on both sides but the sound is the server's).
        return BlockReaction::Deny(Message::NonControllingSeat);
    }
    let Some(train) = c.train else { return BlockReaction::NotTaken };
    if !c.server {
        return BlockReaction::Taken;
    }
    if train.has_schedule {
        if train.paused && !train.completed {
            return BlockReaction::Resume;
        }
        if !c.hand_empty {
            return BlockReaction::Deny(Message::RemoveWithEmptyHand);
        }
        return BlockReaction::Return { auto: train.auto };
    }
    if !c.holds_schedule_item {
        return BlockReaction::Taken;
    }
    let Some(stops) = c.held_schedule else { return BlockReaction::NotTaken };
    if stops == 0 {
        return BlockReaction::Deny(Message::NoStops);
    }
    BlockReaction::Apply
}

/// The message after taking a Schedule back: the Auto-Schedule's is another line.
pub fn returned_message(auto: bool) -> Message {
    if auto { Message::AutoRemoved } else { Message::Removed }
}

// --- the Train Hat -----------------------------------------------------------------------------------------------

/// `EntityHats.shouldRenderTrainHat` (with `getHatFor`'s check of the head slot first): whether a mob or player draws the
/// Train Hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HatView {
    /// The head slot holds an item (`getHatFor` returns nothing then).
    pub head_item: bool,
    /// `getPersistentData().contains("TrainHat")`.
    pub tagged: bool,
    /// It rides a carriage (`getVehicle() instanceof CarriageContraptionEntity`).
    pub on_carriage: bool,
    /// `cce.hasSchedule()`.
    pub carriage_has_schedule: bool,
    pub player: bool,
    /// Its seat is in front of Train Controls (`conductorSeats.get(seatOf) != null`).
    pub conductor_seat: bool,
}

pub fn wears_train_hat(v: &HatView) -> bool {
    if v.head_item {
        return false;
    }
    if v.tagged {
        return true;
    }
    if !v.on_carriage || (!v.carriage_has_schedule && !v.player) {
        return false;
    }
    v.conductor_seat
}

/// `BlazeBurnerMovementBehaviour.shouldRenderHat`: the burner on a carriage, in front of Train Controls, while the train
/// has a schedule (`Conductor` is worked out once from `inControl` and kept in the actor's data).
pub fn burner_wears_hat(conducting: bool, carriage_has_schedule: bool) -> bool {
    conducting && carriage_has_schedule
}

/// Where `BlazeBurnerRenderer.renderShared` puts the hat: up the head's height, and for the small (inert) head also
/// scaled to 0.75 about the model's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatPlacement {
    pub lift: f32,
    pub scale: Option<f32>,
}

pub fn hat_placement(inert_head: bool) -> HatPlacement {
    if inert_head { HatPlacement { lift: 0.5, scale: Some(0.75) } } else { HatPlacement { lift: 0.75, scale: None } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target { on_carriage: true, train: Some(RuntimeView::default()), seated: true, conductor_seat: true }
    }

    #[test]
    fn a_schedule_is_handed_to_a_conductor_on_a_controlling_seat() {
        assert_eq!(hand_schedule_to(Some(2), true, &target()), Hand::Apply);
    }

    #[test]
    fn handing_over_refuses_in_java_s_order() {
        // No schedule in the item, or not on a carriage: the click passes.
        assert_eq!(hand_schedule_to(None, true, &target()), Hand::Pass);
        assert_eq!(hand_schedule_to(Some(1), true, &Target { on_carriage: false, ..target() }), Hand::Pass);
        // The client says success and leaves the rest to the server.
        assert_eq!(hand_schedule_to(Some(1), false, &target()), Hand::Nothing);
        assert_eq!(hand_schedule_to(Some(1), true, &Target { train: None, ..target() }), Hand::Nothing);
        assert_eq!(hand_schedule_to(Some(1), true, &Target { seated: false, ..target() }), Hand::Nothing);
        // A seat that isn't in front of controls is refused before the schedule is looked at.
        let t = Target { conductor_seat: false, train: Some(RuntimeView { has_schedule: true, ..Default::default() }), ..target() };
        assert_eq!(hand_schedule_to(Some(1), true, &t), Hand::Deny(Message::NonControllingSeat));
        let t = Target { train: Some(RuntimeView { has_schedule: true, ..Default::default() }), ..target() };
        assert_eq!(hand_schedule_to(Some(1), true, &t), Hand::Deny(Message::RemoveWithEmptyHand));
        assert_eq!(hand_schedule_to(Some(0), true, &target()), Hand::Deny(Message::NoStops));
    }

    fn click() -> EntityClick {
        EntityClick {
            spectator: false,
            living: true,
            on_cooldown: false,
            main_hand: true,
            hand_empty: true,
            server: true,
            held_schedule: None,
            target: Target { train: Some(RuntimeView { has_schedule: true, ..Default::default() }), ..target() },
        }
    }

    #[test]
    fn a_click_with_a_schedule_hands_it_over_and_is_taken() {
        let c = EntityClick {
            held_schedule: Some(3),
            hand_empty: false,
            target: target(),
            ..click()
        };
        assert_eq!(entity_click(&c), Reaction::Handed(Hand::Apply));
        // A refusal is taken too.
        let c = EntityClick { held_schedule: Some(0), ..c };
        assert_eq!(entity_click(&c), Reaction::Handed(Hand::Deny(Message::NoStops)));
    }

    #[test]
    fn spectators_cooldowns_and_other_vehicles_are_ignored() {
        assert_eq!(entity_click(&EntityClick { spectator: true, ..click() }), Reaction::Ignore);
        assert_eq!(entity_click(&EntityClick { on_cooldown: true, ..click() }), Reaction::Ignore);
        assert_eq!(entity_click(&EntityClick { living: false, ..click() }), Reaction::Ignore);
        let off = Target { on_carriage: false, ..click().target };
        assert_eq!(entity_click(&EntityClick { target: off, ..click() }), Reaction::Ignore);
    }

    #[test]
    fn an_empty_hand_takes_the_schedule_back() {
        assert_eq!(entity_click(&click()), Reaction::Return { auto: false });
        let mut c = click();
        c.target.train = Some(RuntimeView { has_schedule: true, auto: true, ..Default::default() });
        assert_eq!(entity_click(&c), Reaction::Return { auto: true });
        assert_eq!(returned_message(true), Message::AutoRemoved);
        assert_eq!(returned_message(false), Message::Removed);
    }

    #[test]
    fn a_paused_schedule_resumes_before_anything_is_taken_back() {
        let mut c = click();
        c.target.train = Some(RuntimeView { has_schedule: true, paused: true, ..Default::default() });
        assert_eq!(entity_click(&c), Reaction::Resume);
        // A finished schedule is paused too, and is taken back instead.
        c.target.train = Some(RuntimeView { has_schedule: true, paused: true, completed: true, auto: false });
        assert_eq!(entity_click(&c), Reaction::Return { auto: false });
    }

    #[test]
    fn a_full_hand_is_refused_not_cooled_down() {
        let c = EntityClick { hand_empty: false, ..click() };
        assert_eq!(entity_click(&c), Reaction::Refuse);
    }

    #[test]
    fn the_off_hand_and_a_train_without_a_schedule_do_nothing() {
        assert_eq!(entity_click(&EntityClick { main_hand: false, ..click() }), Reaction::Ignore);
        let mut c = click();
        c.target.train = Some(RuntimeView::default());
        assert_eq!(entity_click(&c), Reaction::Ignore);
        let mut c = click();
        c.target.conductor_seat = false;
        assert_eq!(entity_click(&c), Reaction::Ignore);
    }

    fn burner() -> BlockClick {
        BlockClick {
            main_hand: true,
            valid: true,
            in_control: true,
            train: Some(RuntimeView::default()),
            server: true,
            hand_empty: false,
            held_schedule: Some(2),
            holds_schedule_item: true,
        }
    }

    #[test]
    fn a_burner_takes_a_schedule() {
        assert_eq!(block_click(&burner()), BlockReaction::Apply);
        assert_eq!(block_click(&BlockClick { held_schedule: Some(0), ..burner() }), BlockReaction::Deny(Message::NoStops));
        // Another item: the click is taken and nothing happens.
        assert_eq!(block_click(&BlockClick { holds_schedule_item: false, held_schedule: None, ..burner() }), BlockReaction::Taken);
        // A Schedule item with no data isn't taken.
        assert_eq!(block_click(&BlockClick { held_schedule: None, ..burner() }), BlockReaction::NotTaken);
    }

    #[test]
    fn a_burner_gives_the_schedule_back_to_an_empty_hand() {
        let running = Some(RuntimeView { has_schedule: true, ..Default::default() });
        assert_eq!(block_click(&BlockClick { train: running, hand_empty: true, ..burner() }), BlockReaction::Return { auto: false });
        assert_eq!(block_click(&BlockClick { train: running, hand_empty: false, ..burner() }), BlockReaction::Deny(Message::RemoveWithEmptyHand));
        let paused = Some(RuntimeView { has_schedule: true, paused: true, ..Default::default() });
        assert_eq!(block_click(&BlockClick { train: paused, ..burner() }), BlockReaction::Resume);
    }

    #[test]
    fn a_burner_not_in_front_of_controls_says_so() {
        assert_eq!(block_click(&BlockClick { in_control: false, ..burner() }), BlockReaction::Deny(Message::NonControllingSeat));
        assert_eq!(block_click(&BlockClick { main_hand: false, ..burner() }), BlockReaction::NotTaken);
        assert_eq!(block_click(&BlockClick { valid: false, ..burner() }), BlockReaction::NotTaken);
        assert_eq!(block_click(&BlockClick { train: None, ..burner() }), BlockReaction::NotTaken);
    }

    #[test]
    fn the_hat_follows_the_seat_and_the_schedule() {
        let v = HatView { head_item: false, tagged: false, on_carriage: true, carriage_has_schedule: true, player: false, conductor_seat: true };
        assert!(wears_train_hat(&v));
        assert!(!wears_train_hat(&HatView { head_item: true, ..v }));
        // Without a schedule only a player wears it.
        assert!(!wears_train_hat(&HatView { carriage_has_schedule: false, ..v }));
        assert!(wears_train_hat(&HatView { carriage_has_schedule: false, player: true, ..v }));
        assert!(!wears_train_hat(&HatView { conductor_seat: false, ..v }));
        assert!(!wears_train_hat(&HatView { on_carriage: false, ..v }));
        // A tagged mob wears it anywhere (but not over a helmet).
        assert!(wears_train_hat(&HatView { tagged: true, on_carriage: false, ..v }));
        assert!(!wears_train_hat(&HatView { tagged: true, head_item: true, ..v }));
    }

    #[test]
    fn the_burner_hat_needs_a_schedule_and_controls() {
        assert!(burner_wears_hat(true, true));
        assert!(!burner_wears_hat(true, false));
        assert!(!burner_wears_hat(false, true));
        assert_eq!(hat_placement(true), HatPlacement { lift: 0.5, scale: Some(0.75) });
        assert_eq!(hat_placement(false), HatPlacement { lift: 0.75, scale: None });
    }

    #[test]
    fn the_messages_are_create_s() {
        assert_eq!(Message::Applied.key(), "create.schedule.applied_to_train");
        assert_eq!(Message::NoStops.text(), "This Schedule does not have any Stops yet");
    }
}

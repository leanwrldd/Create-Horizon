//! The Packager and Repackager's state machine (`PackagerBlockEntity`, `PackagerItemHandler`,
//! `PackagerBlock`, `RepackagerBlockEntity`): the held box, the tray's animation, the queue of
//! boxes waiting to leave, the redstone button cooldown, the address read from signs. No game
//! types: `B` is a box (a package stack), and times are game ticks, so the animation is
//! computed from the tick it started at and nothing is written between its start and end.
//! What packs a box is `rules::package`; the glue is `machines/packager.rs`.
//!
//! Known differences from Create, for the README:
//! - Create plays the tray's sounds on the client, from its animation counter; here the server
//!   plays them at the same moments of the animation.
//! - The Stock Link side (`recheckIfLinksPresent`, requests, promises, `LastSummary`) belongs to
//!   the stock network, which isn't ported: the Packager is never `linked`, and `attemptToSend`'s
//!   request mode is in `rules::package::pack` for it to use.
//! - No computer peripheral (`computerBehaviour`).

use super::package::is_blank;
use crate::Dir;

/// `PackagerBlockEntity.CYCLE`: the length of the tray's animation.
pub const CYCLE: i64 = 20;

/// `PackagerBlockEntity.activate`: a button signal's cooldown.
pub const BUTTON_COOLDOWN: i64 = 40;

/// `lazyTick`'s period (`SmartBlockEntity.lazyTick` runs every 10 ticks).
pub const LAZY_PERIOD: i64 = 10;

/// `PackagerBlockEntity.isTooBusyFor`, by who asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestType {
    Player,
    Redstone,
    Restock,
}

pub fn too_busy_for(queued: usize, kind: RequestType) -> bool {
    queued
        >= match kind {
            RequestType::Player => 50,
            RequestType::Redstone => 20,
            RequestType::Restock => 10,
        }
}

/// The sounds of the tray's animation, as the moment the client would play them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// `AllSoundEvents.PACKAGER`.
    Packager,
    /// `SoundEvents.IRON_TRAPDOOR_CLOSE`, at 0.25 volume and 0.75 pitch.
    Trapdoor,
}

/// What a tick did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tick {
    /// The state changed in a way a client must hear about.
    pub changed: bool,
    /// The animation ended this tick (`wakeTheFrogs`).
    pub finished: bool,
    pub cue: Option<Cue>,
}

/// The block entity's state. `B` is a box.
#[derive(Clone, Debug, PartialEq)]
pub struct State<B> {
    /// `heldBox`.
    pub held: Option<B>,
    /// `previouslyUnwrapped`: the box going in, drawn for the first half of the animation.
    pub unwrapped: Option<B>,
    /// `queuedExitingPackages`: `(box, how many)`.
    pub queued: Vec<(B, u32)>,
    /// The game time the animation started at; none when it isn't running (`animationTicks == 0`).
    pub start: Option<i64>,
    /// `animationInward`.
    pub inward: bool,
    /// `buttonCooldown`, as the game time it runs until.
    pub cooldown_until: i64,
    /// `redstonePowered`.
    pub powered: bool,
}

impl<B> Default for State<B> {
    fn default() -> Self {
        Self { held: None, unwrapped: None, queued: Vec::new(), start: None, inward: true, cooldown_until: 0, powered: false }
    }
}

impl<B: Clone> State<B> {
    /// `animationTicks`: the ticks of the animation left (0 when not running).
    pub fn animation_ticks(&self, now: i64) -> i64 {
        match self.start {
            Some(start) => (CYCLE - (now - start)).clamp(0, CYCLE),
            None => 0,
        }
    }

    pub fn animating(&self, now: i64) -> bool {
        self.start.is_some() && self.animation_ticks(now) > 0
    }

    /// `PackagerBlockEntity.tick`, on the server: ends the animation, and starts the next box of
    /// the queue out when the tray is free.
    pub fn tick(&mut self, now: i64) -> Tick {
        let mut tick = Tick::default();
        if let Some(start) = self.start {
            let elapsed = now - start;
            if elapsed >= CYCLE {
                self.start = None;
                tick.finished = true;
                tick.changed = true;
            } else {
                // `animationTicks == CYCLE - (inward ? 5 : 1)` plays the packager, and
                // `== (inward ? 1 : 5)` the trapdoor, evaluated before the tick's decrement.
                if elapsed == if self.inward { 6 } else { 2 } {
                    tick.cue = Some(Cue::Packager);
                } else if elapsed == if self.inward { 19 } else { 15 } {
                    tick.cue = Some(Cue::Trapdoor);
                }
                return tick;
            }
        }
        // `animationTicks == 0`: `previouslyUnwrapped = EMPTY`, and the next queued box comes out.
        if self.start.is_none() {
            if self.unwrapped.is_some() {
                self.unwrapped = None;
                tick.changed = true;
            }
            if self.held.is_none() && !self.queued.is_empty() {
                let entry = &mut self.queued[0];
                self.held = Some(entry.0.clone());
                entry.1 = entry.1.saturating_sub(1);
                if entry.1 == 0 {
                    self.queued.remove(0);
                }
                self.inward = false;
                self.start = Some(now);
                tick.changed = true;
            }
        }
        tick
    }

    /// `attemptToSend(null)`'s own guard: no box held, no animation, no button cooldown.
    pub fn can_send(&self, now: i64) -> bool {
        self.held.is_none() && !self.animating(now) && self.cooldown_until <= now
    }

    /// The end of `attemptToSend`: the new box is held and starts out, or waits in the queue when
    /// the tray is busy. Returns whether it went straight to the tray.
    pub fn send(&mut self, created: B, now: i64) -> bool {
        if self.held.is_some() || self.animating(now) {
            self.queued.push((created, 1));
            return false;
        }
        self.held = Some(created);
        self.inward = false;
        self.start = Some(now);
        true
    }

    /// `unwrapBox` after the items went in: the box is drawn going in.
    pub fn begin_unwrap(&mut self, boxed: B, now: i64) {
        self.unwrapped = Some(boxed);
        self.inward = true;
        self.start = Some(now);
    }

    /// `RepackagerBlockEntity.attemptToRepackage`: a package taken out of the inventory is held
    /// and slides out.
    pub fn begin_exit(&mut self, boxed: B, now: i64) {
        self.held = Some(boxed);
        self.inward = false;
        self.start = Some(now);
    }

    /// `PackagerItemHandler.getStackInSlot(0)`: the held box.
    pub fn slot(&self) -> Option<&B> {
        self.held.as_ref()
    }

    /// Whether `PackagerItemHandler.insertItem` takes a box now: nothing held or waiting.
    pub fn accepts(&self, now: i64) -> bool {
        self.held.is_none() && self.queued.is_empty() && !self.animating(now)
    }

    /// `PackagerItemHandler.extractItem`: the held box leaves once the animation is over.
    pub fn extract(&mut self, now: i64) -> Option<B> {
        if self.animating(now) {
            return None;
        }
        self.held.take()
    }

    /// `PackagerBlockEntity.activate`: a rising redstone signal.
    pub fn activate(&mut self, now: i64) {
        self.powered = true;
        // "dont send multiple packages when a button signal length is received".
        if self.cooldown_until <= now {
            self.cooldown_until = now + BUTTON_COOLDOWN;
        }
    }

    /// `PackagerBlock.getAnalogOutputSignal`: 15 with a box held or an animation running.
    pub fn analog_output(&self, now: i64) -> u8 {
        if self.held.is_none() && !self.animating(now) { 0 } else { 15 }
    }

    /// `getTrayOffset`: how far the tray is out, 0 to 1, `partial` ticks after `now`.
    pub fn tray_offset(&self, now: i64, partial: f32) -> f32 {
        let Some(start) = self.start else {
            return 0.0;
        };
        let remaining = CYCLE as f32 - ((now - start) as f32 + partial);
        let cycle = if self.inward { remaining } else { remaining - 5.0 };
        let progress = (cycle / (CYCLE as f32 - 5.0) * 2.0 - 1.0).clamp(-1.0, 1.0);
        let progress = 1.0 - progress * progress;
        progress * progress
    }

    /// `getRenderedBox`: the box drawn on the tray.
    pub fn rendered_box(&self, now: i64) -> Option<&B> {
        let ticks = self.animation_ticks(now);
        if self.inward {
            if ticks <= CYCLE / 2 { None } else { self.unwrapped.as_ref() }
        } else if ticks >= CYCLE / 2 {
            None
        } else {
            self.held.as_ref()
        }
    }

    /// `PackagerRenderer.isHatchOpen`.
    pub fn hatch_open(&self, now: i64) -> bool {
        let ticks = self.animation_ticks(now);
        ticks > if self.inward { 1 } else { 5 } && ticks < CYCLE - if self.inward { 5 } else { 1 }
    }
}

// --- the address on signs ------------------------------------------------------------------------

/// `PackagerBlockEntity.getSign`'s text of one side of a sign: its non-blank lines, trimmed and
/// joined with spaces; none when blank.
pub fn sign_side_text(lines: &[String]) -> Option<String> {
    let mut address = String::new();
    for line in lines {
        if !is_blank(line) {
            address.push_str(line.trim());
            address.push(' ');
        }
    }
    (!is_blank(&address)).then(|| address.trim().to_owned())
}

/// `getSign`: the front's text, else the back's.
pub fn sign_text(front: &[String], back: &[String]) -> Option<String> {
    sign_side_text(front).or_else(|| sign_side_text(back))
}

/// `updateSignAddress`: over the six sides in `Iterate.directions` order (down, up, north, south,
/// west, east), the last sign with text decides.
pub fn sign_address(signs: &[Option<String>]) -> String {
    let mut address = String::new();
    for sign in signs.iter().flatten() {
        if !is_blank(sign) {
            address = sign.clone();
        }
    }
    address
}

// --- placement ---------------------------------------------------------------------------------------

/// `Direction.orderedByNearest` / `BlockPlaceContext.getNearestLookingDirections`: the six
/// directions, the one the player looks towards first and the one behind them last.
pub fn nearest_looking(yaw: f32, pitch: f32) -> [Dir; 6] {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let mut order = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];
    order.sort_by(|a, b| {
        let dot = |d: &Dir| {
            let n = d.normal();
            f64::from(look[0]) * f64::from(n[0]) + f64::from(look[1]) * f64::from(n[1]) + f64::from(look[2]) * f64::from(n[2])
        };
        dot(b).total_cmp(&dot(a))
    });
    order
}

/// `PackagerBlock.getStateForPlacement`: the packager faces away from the first inventory it
/// finds in the order the player looks (`takes_inventory(direction)` says whether the block
/// there is one, other than a packager); with none, it faces the way they look, or back at
/// them when sneaking.
pub fn preferred_facing(order: [Dir; 6], takes_inventory: &dyn Fn(Dir) -> bool, sneaking: bool) -> Dir {
    for face in order {
        if takes_inventory(face) {
            return face.opposite();
        }
    }
    if sneaking { order[0] } else { order[0].opposite() }
}

// --- redstone ---------------------------------------------------------------------------------------

/// What a change of the neighbour signal does (`PackagerBlock.neighborChanged`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Nothing changed.
    None,
    /// The signal came on: `POWERED` is set and `activate` runs.
    Rising,
    /// The signal went off: `POWERED` is cleared.
    Falling,
}

pub fn edge(was_powered: bool, powered: bool) -> Edge {
    match (was_powered, powered) {
        (false, true) => Edge::Rising,
        (true, false) => Edge::Falling,
        _ => Edge::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sent_box_slides_out_and_is_taken_when_the_tray_is_back() {
        let mut state: State<&str> = State::default();
        assert!(state.can_send(0));
        assert!(state.send("box", 100));
        assert!(state.animating(100));
        // Not there yet: nothing to take, nothing to send, nothing goes in.
        assert_eq!(state.extract(110), None);
        assert!(!state.can_send(110));
        assert!(!state.accepts(110));
        // The animation runs for a cycle.
        let mut finished = None;
        for now in 101..=130 {
            let tick = state.tick(now);
            if tick.finished {
                finished = Some(now);
                break;
            }
        }
        assert_eq!(finished, Some(120));
        assert_eq!(state.extract(120), Some("box"));
        assert!(state.can_send(120));
    }

    #[test]
    fn boxes_sent_while_busy_queue_and_come_out_in_turn() {
        let mut state: State<&str> = State::default();
        assert!(state.send("a", 0));
        assert!(!state.send("b", 5));
        assert!(!state.send("b", 6));
        assert_eq!(state.queued, vec![("b", 1), ("b", 1)]);
        // Nothing moves out of the queue while the tray runs.
        for now in 1..20 {
            state.tick(now);
        }
        assert_eq!(state.held, Some("a"));
        state.tick(20);
        // Still held: the next box waits for someone to take the first.
        assert_eq!(state.held, Some("a"));
        assert_eq!(state.extract(20), Some("a"));
        let tick = state.tick(21);
        assert!(tick.changed);
        assert_eq!(state.held, Some("b"));
        assert_eq!(state.queued, vec![("b", 1)]);
        assert_eq!(state.start, Some(21));
        assert!(!state.inward);
    }

    #[test]
    fn a_queued_stack_counts_down() {
        let mut state: State<&str> = State::default();
        state.queued.push(("x", 2));
        state.tick(0);
        assert_eq!(state.held, Some("x"));
        assert_eq!(state.queued, vec![("x", 1)]);
        state.tick(20);
        state.extract(20);
        state.tick(21);
        assert!(state.queued.is_empty());
        assert_eq!(state.held, Some("x"));
    }

    #[test]
    fn a_box_going_in_is_drawn_for_the_first_half_only() {
        let mut state: State<&str> = State::default();
        state.begin_unwrap("in", 0);
        assert_eq!(state.rendered_box(0), Some(&"in"));
        assert_eq!(state.rendered_box(9), Some(&"in"));
        assert_eq!(state.rendered_box(10), None);
        // And it is forgotten once the tray is back.
        state.tick(20);
        state.tick(21);
        assert_eq!(state.unwrapped, None);
    }

    #[test]
    fn a_box_going_out_is_drawn_for_the_second_half() {
        let mut state: State<&str> = State::default();
        state.send("out", 0);
        assert_eq!(state.rendered_box(0), None);
        assert_eq!(state.rendered_box(9), None);
        assert_eq!(state.rendered_box(11), Some(&"out"));
    }

    #[test]
    fn the_tray_comes_out_and_goes_back() {
        let mut state: State<&str> = State::default();
        assert_eq!(state.tray_offset(0, 0.0), 0.0);
        state.send("box", 0);
        let start = state.tray_offset(0, 0.0);
        let middle = state.tray_offset(7, 0.5);
        let end = state.tray_offset(19, 0.5);
        assert!(start < 0.01, "starts in, got {start}");
        assert!(middle > 0.9, "out in the middle, got {middle}");
        assert!(end < 0.01, "back at the end, got {end}");
    }

    #[test]
    fn the_hatch_is_open_between_the_cues() {
        let mut state: State<&str> = State::default();
        state.send("box", 0);
        assert!(!state.hatch_open(0));
        assert!(state.hatch_open(10));
        assert!(!state.hatch_open(19));
    }

    #[test]
    fn a_button_signal_blocks_more_for_two_seconds() {
        let mut state: State<&str> = State::default();
        state.activate(100);
        assert!(state.powered);
        assert!(!state.can_send(120));
        assert!(!state.can_send(139));
        assert!(state.can_send(140));
        // A second pulse during the cooldown doesn't prolong it.
        state.activate(120);
        assert_eq!(state.cooldown_until, 140);
    }

    #[test]
    fn the_comparator_reads_fifteen_while_busy() {
        let mut state: State<&str> = State::default();
        assert_eq!(state.analog_output(0), 0);
        state.begin_unwrap("in", 0);
        assert_eq!(state.analog_output(5), 15);
        assert_eq!(state.analog_output(20), 0);
        state.begin_exit("out", 30);
        assert_eq!(state.analog_output(50), 15);
    }

    #[test]
    fn the_sign_address_is_the_text_of_the_last_sign() {
        let lines = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(sign_side_text(&lines(&["", "  ", ""])), None);
        assert_eq!(sign_side_text(&lines(&[" Farm ", "", "North"])), Some("Farm North".to_owned()));
        // The back is read when the front is blank.
        assert_eq!(sign_text(&lines(&["", "", "", ""]), &lines(&["Mill"])), Some("Mill".to_owned()));
        assert_eq!(sign_text(&lines(&["Mine"]), &lines(&["Mill"])), Some("Mine".to_owned()));
        // The last side wins.
        assert_eq!(sign_address(&[Some("Down".into()), None, Some("North".into())]), "North");
        assert_eq!(sign_address(&[None, None]), "");
    }

    #[test]
    fn busy_limits_by_who_asks() {
        assert!(!too_busy_for(49, RequestType::Player));
        assert!(too_busy_for(50, RequestType::Player));
        assert!(too_busy_for(20, RequestType::Redstone));
        assert!(too_busy_for(10, RequestType::Restock));
    }

    #[test]
    fn the_nearest_directions_start_with_the_look_and_end_with_its_back() {
        // Yaw 0 looks south, pitch 0 level.
        let order = nearest_looking(0.0, 0.0);
        assert_eq!(order[0], Dir::South);
        assert_eq!(order[5], Dir::North);
        // Looking straight down.
        let order = nearest_looking(0.0, 90.0);
        assert_eq!((order[0], order[5]), (Dir::Down, Dir::Up));
        // Yaw 90 looks west.
        assert_eq!(nearest_looking(90.0, 0.0)[0], Dir::West);
    }

    #[test]
    fn a_packager_turns_its_back_on_the_first_inventory_it_sees() {
        let order = nearest_looking(0.0, 0.0);
        // A chest to the east (the third nearest): the packager faces west.
        assert_eq!(preferred_facing(order, &|d| d == Dir::East, false), Dir::West);
        // Chests on two sides: the nearer wins.
        assert_eq!(preferred_facing(order, &|d| d == Dir::South || d == Dir::East, false), Dir::North);
        // Nothing around: it faces back at the player, or the way they look when they sneak.
        assert_eq!(preferred_facing(order, &|_| false, false), Dir::North);
        assert_eq!(preferred_facing(order, &|_| false, true), Dir::South);
    }

    #[test]
    fn redstone_edges() {
        assert_eq!(edge(false, true), Edge::Rising);
        assert_eq!(edge(true, false), Edge::Falling);
        assert_eq!(edge(true, true), Edge::None);
    }
}

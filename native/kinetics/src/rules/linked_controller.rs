//! The Linked Controller: `LinkedControllerClientHandler` (the client's modes and what it sends),
//! `LinkedControllerServerHandler` (the entries a player's pressed keys hold open in the link
//! network) and the messages between them, as plain rules. The game types (keys, items, links)
//! stay in `machines/linked_controller.rs`.
//!
//! Six keys are bound to six frequency pairs: forward, back, left, right, jump and sneak
//! (`ControlsUtil.getControls`).

use std::collections::BTreeSet;

/// The keys, in `ControlsUtil.getControls` order, by their vanilla names (the default bindings:
/// Horizon gives a mod no way to read the player's own).
pub const KEYS: [&str; 6] = [
    "key.keyboard.w",
    "key.keyboard.s",
    "key.keyboard.a",
    "key.keyboard.d",
    "key.keyboard.space",
    "key.keyboard.left.shift",
];
/// What the bind-mode hint calls them.
pub const KEY_LABELS: [&str; 6] = ["W", "S", "A", "D", "Space", "Shift"];
/// `LinkedControllerClientHandler.PACKET_RATE`: ticks between keepalives of a held key.
pub const PACKET_RATE: u32 = 5;
/// `LinkedControllerServerHandler.TIMEOUT`: ticks an entry lives without a keepalive.
pub const TIMEOUT: i32 = 30;
/// `ItemStackHandler(12)`: two frequency items for each key.
pub const FREQUENCY_SLOTS: usize = 12;
/// What a transmitting entry gives (`ManualFrequencyEntry.getTransmittedStrength`).
pub const STRENGTH: i32 = 15;

pub const CONTROLLER: &str = "create:linked_controller";
pub const COMPONENT: &str = "create:linked_controller_items";

pub fn key_slot(name: &str) -> Option<usize> {
    KEYS.iter().position(|k| *k == name)
}

/// Which frequency item slot holds a key's first (`true`) or second frequency
/// (`LinkedControllerItem.toFrequency`, `LinkedControllerBindPacket`).
pub fn frequency_slot(key: usize, first: bool) -> usize {
    key * 2 + usize::from(!first)
}

// ---- the messages (`LinkedControllerInputPacket`, `LinkedControllerBindPacket`) -----------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Keys pressed (a keepalive repeats it) or released.
    Input { keys: Vec<usize>, pressed: bool },
    /// Bind a key to the Redstone Link at the position.
    Bind { key: usize, pos: [i32; 3] },
    /// `LinkedControllerInputPacket` with a lectern position: the keys of the controller on that lectern.
    LecternInput { lectern: [i32; 3], keys: Vec<usize>, pressed: bool },
    /// `LinkedControllerStopLecternPacket`: the player is done with the Lectern Controller.
    StopLectern { lectern: [i32; 3] },
}

pub const CHANNEL: &str = "create:linked_controller";

pub fn encode(message: &Message) -> Vec<u8> {
    match message {
        Message::Input { keys, pressed } => {
            let mask = keys.iter().filter(|k| **k < 6).fold(0u8, |m, k| m | 1 << k);
            vec![0, u8::from(*pressed), mask]
        }
        Message::Bind { key, pos } => {
            let mut out = vec![1, *key as u8];
            for v in pos {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
        Message::LecternInput { lectern, keys, pressed } => {
            let mask = keys.iter().filter(|k| **k < 6).fold(0u8, |m, k| m | 1 << k);
            let mut out = vec![2, u8::from(*pressed), mask];
            for v in lectern {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
        Message::StopLectern { lectern } => {
            let mut out = vec![3];
            for v in lectern {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
    }
}

fn position(bytes: &[u8]) -> [i32; 3] {
    let int = |i: usize| i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    [int(0), int(4), int(8)]
}

pub fn decode(data: &[u8]) -> Option<Message> {
    match data {
        [0, pressed, mask] if *pressed <= 1 && *mask < 64 => Some(Message::Input {
            keys: (0..6).filter(|k| mask & (1 << k) != 0).collect(),
            pressed: *pressed == 1,
        }),
        [1, key, rest @ ..] if *key < 6 && rest.len() == 12 => {
            Some(Message::Bind { key: usize::from(*key), pos: position(rest) })
        }
        [2, pressed, mask, rest @ ..] if *pressed <= 1 && *mask < 64 && rest.len() == 12 => {
            Some(Message::LecternInput {
                lectern: position(rest),
                keys: (0..6).filter(|k| mask & (1 << k) != 0).collect(),
                pressed: *pressed == 1,
            })
        }
        [3, rest @ ..] if rest.len() == 12 => Some(Message::StopLectern { lectern: position(rest) }),
        _ => None,
    }
}

// ---- the client (`LinkedControllerClientHandler`) ---------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Idle,
    Active,
    /// Waiting for a key to bind to the link at this position.
    Bind([i32; 3]),
    /// Using the controller on the lectern at this position (`LinkedControllerClientHandler.lecternPos`): active
    /// without the controller being in a hand.
    Lectern([i32; 3]),
}

#[derive(Clone, Debug, Default)]
pub struct Client {
    pub mode: Mode,
    /// Keys held down now.
    pub down: BTreeSet<usize>,
    /// What the server was last told is pressed (`currentlyPressed`).
    pub sent: BTreeSet<usize>,
    pub cooldown: u32,
}

impl Client {
    /// The reset of `onReset`: release what was pressed.
    fn reset(&mut self) -> Vec<Message> {
        let lectern = match self.mode {
            Mode::Lectern(pos) => Some(pos),
            _ => None,
        };
        self.mode = Mode::Idle;
        self.cooldown = 0;
        let released: Vec<usize> = std::mem::take(&mut self.sent).into_iter().collect();
        let mut out = Vec::new();
        if !released.is_empty() {
            // Create sends the release without the lectern's position; here it keeps it, so the lectern's own
            // controller (and not whatever is in the hand) is the one that lets go.
            out.push(match lectern {
                Some(lectern) => Message::LecternInput { lectern, keys: released, pressed: false },
                None => Message::Input { keys: released, pressed: false },
            });
        }
        if let Some(lectern) = lectern {
            out.push(Message::StopLectern { lectern });
        }
        out
    }

    /// `activateInLectern`: the server says this player now uses the lectern at `pos`.
    pub fn activate_in_lectern(&mut self, pos: [i32; 3]) {
        if self.mode == Mode::Idle {
            self.mode = Mode::Lectern(pos);
        }
    }

    /// `deactivateInLectern`: the server says the player no longer uses their lectern.
    pub fn deactivate_in_lectern(&mut self) -> Vec<Message> {
        if matches!(self.mode, Mode::Lectern(_)) { self.reset() } else { Vec::new() }
    }

    /// `inLectern`.
    pub fn in_lectern(&self) -> bool {
        matches!(self.mode, Mode::Lectern(_))
    }

    /// `toggle`: using the controller in the air or on a block.
    pub fn toggle(&mut self) -> Vec<Message> {
        if self.mode == Mode::Idle {
            self.mode = Mode::Active;
            Vec::new()
        } else {
            self.reset()
        }
    }

    /// `toggleBindMode`: using it on a Redstone Link.
    pub fn toggle_bind(&mut self, link: [i32; 3]) -> Vec<Message> {
        if self.mode == Mode::Idle {
            self.mode = Mode::Bind(link);
            Vec::new()
        } else {
            self.reset()
        }
    }

    /// A control key went down or up.
    pub fn key(&mut self, key: usize, pressed: bool) {
        if pressed {
            self.down.insert(key);
        } else {
            self.down.remove(&key);
        }
    }

    /// Whether the keys are the controller's now (the movement keys are zeroed).
    pub fn captures_keys(&self) -> bool {
        self.mode != Mode::Idle
    }

    /// `tick`: `holding` says a controller is in a hand, `blocked` that a screen is open or the
    /// player is a spectator. What to send.
    pub fn tick(&mut self, holding: bool, blocked: bool) -> Vec<Message> {
        if self.mode == Mode::Idle {
            return Vec::new();
        }
        self.cooldown = self.cooldown.saturating_sub(1);
        // On a lectern the controller needn't be in a hand (`!inLectern() && !isIn(heldItem)`).
        if (!holding && !self.in_lectern()) || blocked {
            return self.reset();
        }
        let pressed = self.down.clone();
        let new: Vec<usize> = pressed.difference(&self.sent).copied().collect();
        let released: Vec<usize> = self.sent.difference(&pressed).copied().collect();
        let mut out = Vec::new();
        match self.mode {
            Mode::Active | Mode::Lectern(_) => {
                let lectern = match self.mode {
                    Mode::Lectern(pos) => Some(pos),
                    _ => None,
                };
                let input = |keys: Vec<usize>, pressed: bool| match lectern {
                    Some(lectern) => Message::LecternInput { lectern, keys, pressed },
                    None => Message::Input { keys, pressed },
                };
                if !released.is_empty() {
                    out.push(input(released, false));
                }
                if !new.is_empty() {
                    out.push(input(new, true));
                    self.cooldown = PACKET_RATE;
                }
                if self.cooldown == 0 && !pressed.is_empty() {
                    out.push(input(pressed.iter().copied().collect(), true));
                    self.cooldown = PACKET_RATE;
                }
            }
            Mode::Bind(link) => {
                if let Some(key) = new.first() {
                    out.push(Message::Bind { key: *key, pos: link });
                    self.mode = Mode::Idle;
                }
            }
            Mode::Idle => {}
        }
        self.sent = pressed;
        out
    }
}

// ---- the server (`LinkedControllerServerHandler`) ---------------------------------------------------

/// What the link network is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect<K> {
    /// A new transmitter of 15 at the position.
    Add { key: K, pos: [i32; 3] },
    /// The player moved: the transmitter is at a new position.
    Move { key: K, from: [i32; 3], to: [i32; 3] },
    /// The transmitter is gone.
    Remove { key: K, pos: [i32; 3] },
}

#[derive(Clone, Debug)]
struct Entry<K> {
    key: K,
    slot: usize,
    pos: [i32; 3],
    /// `IntAttached.first`: ticks left.
    life: i32,
}

/// One player's open transmitters.
#[derive(Clone, Debug)]
pub struct Server<K> {
    entries: Vec<Entry<K>>,
}

impl<K> Default for Server<K> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<K: Clone + PartialEq> Server<K> {
    /// `receivePressed`: for each key, its frequency (`keys`, by slot) and whether it was pressed.
    pub fn receive(&mut self, pos: [i32; 3], keys: &[(usize, K)], pressed: bool) -> Vec<Effect<K>> {
        let mut out = Vec::new();
        for (slot, key) in keys {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.key == *key) {
                if !pressed {
                    entry.life = 0;
                } else {
                    // `updatePosition`
                    if entry.pos != pos {
                        out.push(Effect::Move { key: key.clone(), from: entry.pos, to: pos });
                        entry.pos = pos;
                    }
                    entry.life = TIMEOUT;
                }
                continue;
            }
            if !pressed {
                continue;
            }
            self.entries.push(Entry { key: key.clone(), slot: *slot, pos, life: TIMEOUT });
            out.push(Effect::Add { key: key.clone(), pos });
        }
        out
    }

    /// `tick`: entries age; those that ran out are removed.
    pub fn tick(&mut self) -> Vec<Effect<K>> {
        let mut out = Vec::new();
        for entry in &mut self.entries {
            entry.life -= 1;
        }
        self.entries.retain(|entry| {
            let alive = entry.life > 0;
            if !alive {
                out.push(Effect::Remove { key: entry.key.clone(), pos: entry.pos });
            }
            alive
        });
        out
    }

    /// The transmitters still open: they report to the network each tick.
    pub fn alive(&self) -> impl Iterator<Item = (&K, [i32; 3])> {
        self.entries.iter().map(|e| (&e.key, e.pos))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Everything goes (the player left, or put the controller away).
    pub fn clear(&mut self) -> Vec<Effect<K>> {
        self.entries.drain(..).map(|e| Effect::Remove { key: e.key, pos: e.pos }).collect()
    }

    /// The key slots that are open (for tests).
    pub fn slots(&self) -> Vec<usize> {
        self.entries.iter().map(|e| e.slot).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_slots_in_create_order() {
        assert_eq!(key_slot("key.keyboard.w"), Some(0));
        assert_eq!(key_slot("key.keyboard.s"), Some(1));
        assert_eq!(key_slot("key.keyboard.left.shift"), Some(5));
        assert_eq!(key_slot("key.keyboard.q"), None);
        assert_eq!(frequency_slot(0, true), 0);
        assert_eq!(frequency_slot(0, false), 1);
        assert_eq!(frequency_slot(5, false), 11);
        assert_eq!(FREQUENCY_SLOTS, frequency_slot(5, false) + 1);
    }

    #[test]
    fn messages_round_trip() {
        let m = Message::Input { keys: vec![0, 2, 5], pressed: true };
        assert_eq!(decode(&encode(&m)), Some(m));
        let m = Message::Input { keys: vec![], pressed: false };
        assert_eq!(decode(&encode(&m)), Some(m));
        let m = Message::Bind { key: 3, pos: [-12, 64, 9000] };
        assert_eq!(decode(&encode(&m)), Some(m));
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[0, 2, 1]), None);
        assert_eq!(decode(&[0, 1, 200]), None);
        assert_eq!(decode(&[1, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(decode(&[1, 1, 0]), None);
    }

    #[test]
    fn a_use_toggles_active_and_a_second_one_releases() {
        let mut c = Client::default();
        assert!(c.toggle().is_empty());
        assert_eq!(c.mode, Mode::Active);
        c.key(0, true);
        let sent = c.tick(true, false);
        assert_eq!(sent, vec![Message::Input { keys: vec![0], pressed: true }]);
        // Using it again puts it away and releases what is pressed.
        let sent = c.toggle();
        assert_eq!(sent, vec![Message::Input { keys: vec![0], pressed: false }]);
        assert_eq!(c.mode, Mode::Idle);
    }

    #[test]
    fn a_held_key_is_repeated_every_five_ticks_and_a_released_one_is_sent_once() {
        let mut c = Client::default();
        c.toggle();
        c.key(1, true);
        assert_eq!(c.tick(true, false).len(), 1); // pressed
        let mut keepalives = 0;
        for _ in 0..10 {
            keepalives += c.tick(true, false).len();
        }
        assert_eq!(keepalives, 2);
        c.key(1, false);
        assert_eq!(c.tick(true, false), vec![Message::Input { keys: vec![1], pressed: false }]);
        assert!(c.tick(true, false).is_empty());
    }

    #[test]
    fn putting_the_controller_away_or_opening_a_screen_resets() {
        let mut c = Client::default();
        c.toggle();
        c.key(4, true);
        c.tick(true, false);
        let sent = c.tick(false, false);
        assert_eq!(sent, vec![Message::Input { keys: vec![4], pressed: false }]);
        assert_eq!(c.mode, Mode::Idle);
        c.toggle();
        c.key(4, true);
        c.tick(true, false);
        assert_eq!(c.tick(true, true).len(), 1);
        assert!(!c.captures_keys());
    }

    #[test]
    fn lectern_messages_round_trip() {
        let m = Message::LecternInput { lectern: [-4, 70, 12], keys: vec![1, 5], pressed: true };
        assert_eq!(decode(&encode(&m)), Some(m));
        let m = Message::StopLectern { lectern: [100, -3, 0] };
        assert_eq!(decode(&encode(&m)), Some(m));
        assert_eq!(decode(&[3, 0, 0]), None);
        assert_eq!(decode(&[2, 1, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn a_lectern_session_sends_the_lecterns_position_and_needs_no_controller_in_hand() {
        let mut c = Client::default();
        c.activate_in_lectern([5, 64, 5]);
        assert_eq!(c.mode, Mode::Lectern([5, 64, 5]));
        assert!(c.captures_keys() && c.in_lectern());
        c.key(2, true);
        // Nothing in a hand, still active.
        let sent = c.tick(false, false);
        assert_eq!(sent, vec![Message::LecternInput { lectern: [5, 64, 5], keys: vec![2], pressed: true }]);
        // The server lets go: the keys are released on the lectern, and the stop follows.
        let sent = c.deactivate_in_lectern();
        assert_eq!(
            sent,
            vec![
                Message::LecternInput { lectern: [5, 64, 5], keys: vec![2], pressed: false },
                Message::StopLectern { lectern: [5, 64, 5] }
            ]
        );
        assert_eq!(c.mode, Mode::Idle);
    }

    #[test]
    fn a_screen_ends_a_lectern_session_and_an_active_controller_ignores_a_lectern_stop() {
        let mut c = Client::default();
        c.activate_in_lectern([1, 2, 3]);
        assert_eq!(c.tick(false, true), vec![Message::StopLectern { lectern: [1, 2, 3] }]);
        // Not on a lectern: the server's "stop using" changes nothing.
        let mut c = Client::default();
        c.toggle();
        assert!(c.deactivate_in_lectern().is_empty());
        assert_eq!(c.mode, Mode::Active);
        // Already active: a lectern can't be taken over.
        c.activate_in_lectern([1, 2, 3]);
        assert_eq!(c.mode, Mode::Active);
    }

    #[test]
    fn bind_mode_binds_the_first_new_key_and_ends() {
        let mut c = Client::default();
        assert!(c.toggle_bind([1, 2, 3]).is_empty());
        assert!(c.captures_keys());
        assert!(c.tick(true, false).is_empty());
        c.key(3, true);
        assert_eq!(c.tick(true, false), vec![Message::Bind { key: 3, pos: [1, 2, 3] }]);
        assert_eq!(c.mode, Mode::Idle);
        // A second use while in bind mode cancels it.
        c.toggle_bind([1, 2, 3]);
        let _ = c.toggle_bind([4, 5, 6]);
        assert_eq!(c.mode, Mode::Idle);
    }

    #[test]
    fn the_server_opens_an_entry_per_key_and_keeps_it_alive_with_keepalives() {
        let mut s: Server<&str> = Server::default();
        let at = [0, 64, 0];
        let fx = s.receive(at, &[(0, "a"), (1, "b")], true);
        assert_eq!(fx, vec![Effect::Add { key: "a", pos: at }, Effect::Add { key: "b", pos: at }]);
        // Without keepalives the entry runs out after the timeout.
        let mut removed = 0;
        for tick in 0..TIMEOUT {
            if tick == 20 {
                assert!(s.receive(at, &[(0, "a")], true).is_empty());
            }
            removed += s.tick().len();
        }
        // `b` was not refreshed: it is gone; `a` was refreshed at tick 20.
        assert_eq!(removed, 1);
        assert_eq!(s.slots(), vec![0]);
        for _ in 0..TIMEOUT {
            s.tick();
        }
        assert!(s.is_empty());
    }

    #[test]
    fn a_release_removes_an_entry_on_the_next_tick() {
        let mut s: Server<u8> = Server::default();
        s.receive([0; 3], &[(0, 7)], true);
        assert!(s.receive([0; 3], &[(0, 7)], false).is_empty());
        assert_eq!(s.tick(), vec![Effect::Remove { key: 7, pos: [0; 3] }]);
        assert!(s.is_empty());
        // A release for a key never pressed does nothing.
        assert!(s.receive([0; 3], &[(1, 9)], false).is_empty());
    }

    #[test]
    fn a_moving_player_moves_the_transmitter() {
        let mut s: Server<u8> = Server::default();
        s.receive([0, 0, 0], &[(0, 1)], true);
        let fx = s.receive([5, 0, 0], &[(0, 1)], true);
        assert_eq!(fx, vec![Effect::Move { key: 1, from: [0, 0, 0], to: [5, 0, 0] }]);
        assert_eq!(s.alive().next(), Some((&1, [5, 0, 0])));
    }

    #[test]
    fn clearing_removes_everything() {
        let mut s: Server<u8> = Server::default();
        s.receive([0; 3], &[(0, 1), (1, 2)], true);
        assert_eq!(s.clear().len(), 2);
        assert!(s.is_empty());
    }
}

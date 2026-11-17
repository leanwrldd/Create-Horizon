//! The Lectern Controller (`LecternControllerBlock`, `LecternControllerBlockEntity`): a Linked Controller placed on
//! a lectern, which a player standing close to it uses until they walk away. The session rules (who uses it, when
//! it stops), the range and the drop's place are here; the controller's own keys, frequencies and messages are
//! `linked_controller`.
//!
//! Known differences from Create, for the README:
//! - the block's `powered` state is copied but never read (the lectern's page-turn pulse needs a book): it gives no
//!   redstone signal;
//! - the controller's frequency items are saved under `ControllerData` as the stack's own
//!   `create:linked_controller_items` layout (`Items`, each with `Slot`), not as an `ItemContainerContents`;
//! - the player's "is using a lectern" mark (`IsUsingLecternController` in their persistent data) is kept in memory,
//!   so a restart ends a session as a reload does in Create (`tick` finds no such player);
//! - the block's drop of the controller comes from the break itself (`destroy_block`), and from the loot table's
//!   drops for what is not a player breaking it; the buttons don't animate under the held keys, as the item's don't.

/// `AllBlocks.LECTERN_CONTROLLER`.
pub const BLOCK: &str = "create:lectern_controller";
/// Vanilla's lectern.
pub const LECTERN: &str = "minecraft:lectern";

/// `Attributes.BLOCK_INTERACTION_RANGE`'s default.
pub const BLOCK_INTERACTION_RANGE: f64 = 4.5;

/// `LecternControllerBlockEntity.playerInRange`: the player's eye within 0.4 times their reach of the block's centre.
pub fn in_range(eye: [f64; 3], pos: [i32; 3]) -> bool {
    let reach = 0.4 * BLOCK_INTERACTION_RANGE;
    let d: [f64; 3] = std::array::from_fn(|i| eye[i] - (f64::from(pos[i]) + 0.5));
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < reach * reach
}

/// What the block entity's tick knows of its user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum User {
    /// No such player is in the world (`getEntity(user)` isn't a player).
    Gone,
    /// A player: in range and still marked as using a lectern or not.
    Present { in_range: bool, using: bool },
}

/// `LecternControllerBlockEntity`'s `user` and `deactivatedThisTick`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub user: Option<u128>,
    pub deactivated_this_tick: bool,
}

impl Session {
    pub fn has_user(&self) -> bool {
        self.user.is_some()
    }

    pub fn is_used_by(&self, player: u128) -> bool {
        self.user == Some(player)
    }

    /// `tryStartUsing`: a free lectern, not stopped this very tick, for a player who is near and uses no other
    /// lectern. True when they now use it.
    pub fn try_start(&mut self, player: u128, in_range: bool, using_a_lectern: bool) -> bool {
        if !self.deactivated_this_tick && self.user.is_none() && !using_a_lectern && in_range {
            self.user = Some(player);
            return true;
        }
        false
    }

    /// `tryStopUsing`. True when the player was the user.
    pub fn try_stop(&mut self, player: u128) -> bool {
        if self.is_used_by(player) {
            self.stop();
            return true;
        }
        false
    }

    /// `stopUsing`.
    pub fn stop(&mut self) {
        self.user = None;
        self.deactivated_this_tick = true;
    }

    /// The server's `tick`: the flag clears, and a user who is gone, out of range or no longer marked is let go.
    /// Returns the user who was stopped.
    pub fn tick(&mut self, user: User) -> Option<u128> {
        self.deactivated_this_tick = false;
        let who = self.user?;
        let stop = match user {
            User::Gone => true,
            User::Present { in_range, using } => !in_range || !using,
        };
        if stop {
            self.stop();
            return Some(who);
        }
        None
    }
}

/// `LecternControllerBlockEntity.dropController`: where the item appears (0.25 towards the front, on top).
pub fn drop_at(pos: [i32; 3], facing: &str) -> [f64; 3] {
    let (dx, dz) = match facing {
        "north" => (0.0, -1.0),
        "south" => (0.0, 1.0),
        "west" => (-1.0, 0.0),
        "east" => (1.0, 0.0),
        _ => (0.0, 0.0),
    };
    [f64::from(pos[0]) + 0.5 + 0.25 * dx, f64::from(pos[1]) + 1.0, f64::from(pos[2]) + 0.5 + 0.25 * dz]
}

/// `LinkedControllerItem.onItemUseFirst`: a controller goes on a lectern that has no book.
pub fn takes_controller(block: &str, has_book: bool) -> bool {
    block == LECTERN && !has_book
}

/// `LecternControllerBlock.replaceLectern`: the controller block's state, with the lectern's `facing` and `powered`
/// (`has_book` stays at its default, true).
pub fn controller_state(facing: &str, powered: bool) -> String {
    format!("{BLOCK}[facing={facing},has_book=true,powered={powered}]")
}

/// `LecternControllerBlock.replaceWithLectern`: the lectern's state, with the controller's `facing` and `powered`.
pub fn lectern_state(facing: &str, powered: bool) -> String {
    format!("{LECTERN}[facing={facing},powered={powered}]")
}

/// `LecternControllerBlock.getAnalogOutputSignal`.
pub const ANALOG_OUTPUT: i32 = 15;

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u128 = 7;
    const YOU: u128 = 9;

    #[test]
    fn the_range_is_four_tenths_of_the_reach() {
        // 0.4 * 4.5 = 1.8 blocks from the centre.
        assert!(in_range([0.5, 1.5, 0.5], [0, 0, 0]));
        assert!(in_range([2.2, 0.5, 0.5], [0, 0, 0]));
        assert!(!in_range([2.4, 0.5, 0.5], [0, 0, 0]));
        assert!(!in_range([0.5, 3.0, 0.5], [0, 0, 0]));
    }

    #[test]
    fn a_free_lectern_starts_for_a_near_player_only() {
        let mut s = Session::default();
        assert!(!s.try_start(ME, false, false));
        assert!(!s.try_start(ME, true, true), "a player using another lectern can't start this one");
        assert!(s.try_start(ME, true, false));
        assert!(s.is_used_by(ME) && s.has_user());
        assert!(!s.try_start(YOU, true, false), "it has a user");
    }

    #[test]
    fn stopping_takes_the_session_and_blocks_a_restart_this_tick() {
        let mut s = Session::default();
        s.try_start(ME, true, false);
        assert!(!s.try_stop(YOU));
        assert!(s.try_stop(ME));
        assert!(!s.has_user());
        // The same tick: no one can start it again.
        assert!(!s.try_start(ME, true, false));
        // The next tick's start clears the flag.
        assert_eq!(s.tick(User::Gone), None);
        assert!(s.try_start(ME, true, false));
    }

    #[test]
    fn a_tick_lets_a_user_go_who_left() {
        let mut s = Session::default();
        s.try_start(ME, true, false);
        assert_eq!(s.tick(User::Present { in_range: true, using: true }), None);
        assert!(s.has_user());
        assert_eq!(s.tick(User::Present { in_range: false, using: true }), Some(ME));
        assert!(!s.has_user());
        s.tick(User::Gone);
        s.try_start(ME, true, false);
        assert_eq!(s.tick(User::Present { in_range: true, using: false }), Some(ME));
        s.tick(User::Gone);
        s.try_start(ME, true, false);
        assert_eq!(s.tick(User::Gone), Some(ME));
    }

    #[test]
    fn the_item_appears_in_front_of_the_lectern() {
        assert_eq!(drop_at([10, 64, 10], "north"), [10.5, 65.0, 10.25]);
        assert_eq!(drop_at([10, 64, 10], "east"), [10.75, 65.0, 10.5]);
        assert_eq!(drop_at([10, 64, 10], "west"), [10.25, 65.0, 10.5]);
    }

    #[test]
    fn a_controller_goes_on_an_empty_lectern_only() {
        assert!(takes_controller("minecraft:lectern", false));
        assert!(!takes_controller("minecraft:lectern", true));
        assert!(!takes_controller("create:lectern_controller", false));
        assert!(!takes_controller("minecraft:chest", false));
    }

    #[test]
    fn the_states_carry_facing_and_powered() {
        assert_eq!(
            controller_state("east", false),
            "create:lectern_controller[facing=east,has_book=true,powered=false]"
        );
        assert_eq!(lectern_state("west", true), "minecraft:lectern[facing=west,powered=true]");
    }
}

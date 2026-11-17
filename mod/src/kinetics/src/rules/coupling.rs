//! Minecart Coupling's rules (`MinecartCouplingItem`, `CouplingHandler`, `CouplingPhysics`): which
//! carts may be coupled, and how a coupled pair pulls each other along. No game types: the game
//! glue is `machines/coupling.rs`.
//!
//! Known differences from Create, for the README:
//! - Create keeps a `MinecartController` per cart (an attachment on the cart) and runs
//!   `MinecartSim2020`, which moves a cart along its rail shape. A mod can't keep data on a vanilla
//!   minecart or move one along its rail, so the couplings are a list saved with the level, by
//!   entity id and uuid, and each tick only the soft step is done: both carts' velocities are
//!   corrected so the gap closes (`softCollisionStep`, without the rail shape: the game's own rail
//!   movement keeps a cart on its track). Create's hard step, which moves a cart's position when the
//!   soft step wasn't enough, is left out, so a long stretched train eases in rather than snapping.
//! - A coupling does not survive a restart of the game: minecart entity ids change, so a saved
//!   coupling whose ids no longer name its carts is dropped (the item is not returned).
//! - A coupling is drawn as a line between the two carts, not Create's chain model.
//! - Carts can't be coupled through a contraption (`CouplingHandler.tryToCoupleCarts` with no
//!   player), and a coupled pair doesn't block riders from mounting.

/// `CKinetics.maxCartCouplingLength`: the default of the config (32 blocks).
pub const MAX_LENGTH: i32 = 32;
/// `tryToCoupleCarts`: a closer pair is coupled at 2 blocks.
pub const MIN_LENGTH: i32 = 2;
/// A coupling's carts may each have two couplings at most (`isFullyCoupled`).
pub const MAX_PER_CART: usize = 2;
/// `AbstractMinecart.getMaxCartSpeedOnRail` for a vanilla cart (blocks a tick).
pub const MAX_SPEED: f64 = 0.4;
/// `CouplingPhysics.softCollisionStep`: motions are clamped to a block a tick before the pull.
const MAX_MOTION: f64 = 1.0;

/// One coupling between two carts, by entity id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub a: u32,
    pub b: u32,
    /// The length it holds the carts at, in blocks.
    pub length: i32,
}

/// Why a pair wasn't coupled (`CouplingHandler.status`, the keys of `minecart_coupling.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// "Minecarts are too far apart".
    TooFar,
    /// "Minecarts cannot have more than two couplings each".
    TwoCouplingsMax,
    /// "Couplings cannot form a loop".
    NoLoops,
    /// The two are coupled already: nothing happens and no message shows.
    Already,
    /// The same cart twice.
    Same,
}

impl Refusal {
    /// The line `CouplingHandler.status` shows, none for the quiet refusals.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Self::TooFar => Some("Minecarts are too far apart"),
            Self::TwoCouplingsMax => Some("Minecarts cannot have more than two couplings each"),
            Self::NoLoops => Some("Couplings cannot form a loop"),
            Self::Already | Self::Same => None,
        }
    }
}

/// Removes the links that touch `cart`; how many there were.
pub fn decouple(links: &mut Vec<Link>, cart: u32) -> usize {
    let before = links.len();
    links.retain(|link| link.a != cart && link.b != cart);
    before - links.len()
}

fn degree(links: &[Link], cart: u32) -> usize {
    links.iter().filter(|l| l.a == cart || l.b == cart).count()
}

/// Whether `from` and `to` are joined by a chain of links.
fn connected(links: &[Link], from: u32, to: u32) -> bool {
    let mut seen = vec![from];
    let mut frontier = vec![from];
    while let Some(cart) = frontier.pop() {
        for link in links {
            let next = if link.a == cart {
                link.b
            } else if link.b == cart {
                link.a
            } else {
                continue;
            };
            if next == to {
                return true;
            }
            if !seen.contains(&next) {
                seen.push(next);
                frontier.push(next);
            }
        }
    }
    false
}

/// `CouplingHandler.tryToCoupleCarts` for a player: the link to add for two carts `distance`
/// blocks apart, or why not. `max_length` is the config's.
pub fn try_couple(
    links: &[Link],
    a: u32,
    b: u32,
    distance: f64,
    max_length: i32,
) -> Result<Link, Refusal> {
    if a == b {
        return Err(Refusal::Same);
    }
    // `(int) distanceTo`, at least 2.
    let length = (distance as i32).max(MIN_LENGTH);
    if length > max_length {
        return Err(Refusal::TooFar);
    }
    if degree(links, a) >= MAX_PER_CART || degree(links, b) >= MAX_PER_CART {
        return Err(Refusal::TwoCouplingsMax);
    }
    if links.iter().any(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)) {
        return Err(Refusal::Already);
    }
    if connected(links, a, b) {
        return Err(Refusal::NoLoops);
    }
    Ok(Link { a, b, length })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `VecHelper.clamp`: the vector with its length at most `limit`.
fn clamp(v: [f64; 3], limit: f64) -> [f64; 3] {
    let len = length(v);
    if len > limit && len > 0.0 {
        let scale = limit / len;
        [v[0] * scale, v[1] * scale, v[2] * scale]
    } else {
        v
    }
}

/// `CouplingPhysics.softCollisionStep` for one coupled pair: from their positions and velocities,
/// the velocities that pull them towards `length` apart, as `[first, second]`. Carts that are
/// already the right distance apart in the next tick keep theirs.
pub fn soft_step(
    positions: [[f64; 3]; 2],
    velocities: [[f64; 3]; 2],
    length_wanted: f64,
) -> [[f64; 3]; 2] {
    let motions = [clamp(velocities[0], MAX_MOTION), clamp(velocities[1], MAX_MOTION)];
    // `MinecartSim2020.predictNextPositionOf`: where each is a tick on.
    let next = [add(positions[0], motions[0]), add(positions[1], motions[1])];
    let stress = length_wanted - length(sub(next[0], next[1]));
    if stress.abs() < 1e-6 {
        return motions;
    }
    let mut out = motions;
    for current in 0..2 {
        let link = sub(next[1 - current], next[current]);
        let size = length(link);
        if size == 0.0 {
            continue;
        }
        let magnitude = -stress / 2.0;
        let correction = clamp(
            [link[0] / size * magnitude, link[1] / size * magnitude, link[2] / size * magnitude],
            MAX_SPEED,
        );
        out[current] = add(out[current], correction);
    }
    [clamp(out[0], MAX_SPEED), clamp(out[1], MAX_SPEED)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_free_carts_couple_at_their_distance_and_at_least_two() {
        assert_eq!(
            try_couple(&[], 1, 2, 5.7, MAX_LENGTH),
            Ok(Link { a: 1, b: 2, length: 5 })
        );
        assert_eq!(try_couple(&[], 1, 2, 0.4, MAX_LENGTH).unwrap().length, 2);
        assert_eq!(try_couple(&[], 1, 1, 3.0, MAX_LENGTH), Err(Refusal::Same));
    }

    #[test]
    fn a_pair_too_far_apart_is_refused() {
        assert_eq!(try_couple(&[], 1, 2, 32.0, MAX_LENGTH).unwrap().length, 32);
        assert_eq!(try_couple(&[], 1, 2, 33.0, MAX_LENGTH), Err(Refusal::TooFar));
        assert_eq!(Refusal::TooFar.message(), Some("Minecarts are too far apart"));
    }

    #[test]
    fn a_cart_has_two_couplings_at_most() {
        let links = [Link { a: 1, b: 2, length: 3 }, Link { a: 2, b: 3, length: 3 }];
        assert_eq!(try_couple(&links, 2, 4, 3.0, MAX_LENGTH), Err(Refusal::TwoCouplingsMax));
        assert!(try_couple(&links, 3, 4, 3.0, MAX_LENGTH).is_ok());
    }

    #[test]
    fn a_chain_never_closes_into_a_loop() {
        let links = [Link { a: 1, b: 2, length: 3 }, Link { a: 2, b: 3, length: 3 }];
        // 1 and 3 each have one coupling: joining them would close the loop.
        assert_eq!(try_couple(&links, 1, 3, 3.0, MAX_LENGTH), Err(Refusal::NoLoops));
        // The same pair twice is quietly refused.
        assert_eq!(try_couple(&links, 1, 2, 3.0, MAX_LENGTH), Err(Refusal::TwoCouplingsMax));
        let one = [Link { a: 1, b: 2, length: 3 }];
        assert_eq!(try_couple(&one, 2, 1, 3.0, MAX_LENGTH), Err(Refusal::Already));
    }

    #[test]
    fn decoupling_removes_the_carts_links() {
        let mut links = vec![Link { a: 1, b: 2, length: 3 }, Link { a: 2, b: 3, length: 3 }];
        assert_eq!(decouple(&mut links, 2), 2);
        assert!(links.is_empty());
        assert_eq!(decouple(&mut links, 2), 0);
    }

    #[test]
    fn a_stretched_pair_is_pulled_together_and_a_squeezed_one_apart() {
        let at = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0]];
        let still = [[0.0; 3]; 2];
        let pulled = soft_step(at, still, 4.0);
        // Too far: the first goes towards +x, the second towards -x.
        assert!(pulled[0][0] > 0.0 && pulled[1][0] < 0.0);
        assert!((pulled[0][0] + pulled[1][0]).abs() < 1e-9);
        let squeezed = soft_step(at, still, 8.0);
        assert!(squeezed[0][0] < 0.0 && squeezed[1][0] > 0.0);
        // Exactly the right distance: nothing changes.
        assert_eq!(soft_step(at, still, 6.0), still);
    }

    #[test]
    fn the_pull_is_limited_to_a_carts_top_speed() {
        let at = [[0.0, 0.0, 0.0], [30.0, 0.0, 0.0]];
        let out = soft_step(at, [[0.0; 3]; 2], 2.0);
        assert!(length(out[0]) <= MAX_SPEED + 1e-9);
        assert!(length(out[1]) <= MAX_SPEED + 1e-9);
    }
}

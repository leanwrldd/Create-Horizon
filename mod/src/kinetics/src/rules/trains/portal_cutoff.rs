//! What a carriage in a portal hides, and who has to get off: the cutoff half of `CarriageContraption` (`portalCutoffMin`,
//! `portalCutoffMax`, `isHiddenInPortal`, `atSeam`, `withinVisible`, `notInPortal`), `CarriageContraptionEntity.isLocalCoordWithin`
//! and `updateRenderedPortalCutoff`, and the passenger half of `Carriage.DimensionalCarriageEntity` (`updatePassengerLoadout`,
//! `dismountPlayer`). No game types. The cutoff itself (`cutoff`, `minAllowedLocalCoord`, `maxAllowedLocalCoord`,
//! `updateCutoff`, `discardPivot`) is `carriage.rs`.
//!
//! A carriage that straddles a portal is two entities, one per dimension, each holding the part of the carriage that is on its
//! side: `min` and `max` are the local coordinates (along the carriage, counted from the front) that its side shows, the block at
//! either bound is the seam, drawn as the portal's slice.
//!
//! Nothing here is wired: trains here don't cross dimensions (`machines/trains.rs` keeps one set of graphs and one set of carriage
//! anchors per dimension; see the README for what portals need).
//!
//! `Axis.choose(x, y, z)` and `Direction.getAxisDirection().getStep()` are Minecraft's.

use std::collections::BTreeMap;

use crate::{Axis, Dir, Pos};

/// `localPos` as a position along the carriage: `axis.choose(z, y, x) * -facing.getAxisDirection().getStep()` for the axis of
/// `facing.getClockWise()` (`facing` is the carriage's `assemblyDirection`).
pub fn local_coord(assembly_direction: Dir, local: Pos) -> i32 {
    let axis = assembly_direction.clockwise_about(Axis::Y).axis();
    let along = match axis {
        Axis::X => local[2],
        Axis::Y => local[1],
        Axis::Z => local[0],
    };
    along * -assembly_direction.sign()
}

/// `CarriageContraption.notInPortal`: neither bound is set.
pub fn not_in_portal(min: i32, max: i32) -> bool {
    min == i32::MIN && max == i32::MAX
}

/// `atSeam(int)`.
pub fn at_seam(coord: i32, min: i32, max: i32) -> bool {
    coord == min || coord == max
}

/// `withinVisible(int)`: strictly between the bounds.
pub fn within_visible(coord: i32, min: i32, max: i32) -> bool {
    coord > min && coord < max
}

/// `isHiddenInPortal(int)`: false when the carriage is not in a portal; else outside the visible part or at the seam.
pub fn hidden_in_portal(coord: i32, min: i32, max: i32) -> bool {
    if not_in_portal(min, max) {
        return false;
    }
    !within_visible(coord, min, max) || at_seam(coord, min, max)
}

/// `isHiddenInPortal(BlockPos)` for a block of the carriage.
pub fn block_hidden(assembly_direction: Dir, local: Pos, min: i32, max: i32) -> bool {
    hidden_in_portal(local_coord(assembly_direction, local), min, max)
}

/// `CarriageContraptionEntity.isLocalCoordWithin`: a seat counts for this side when its coordinate is within `min..=max`.
pub fn is_local_coord_within(assembly_direction: Dir, local: Pos, min: i32, max: i32) -> bool {
    let c = local_coord(assembly_direction, local);
    c >= min && c <= max
}

/// `CarriageContraption.getSimplifiedEntityColliders` is `null` while in a portal: the carriage's collision is the blocks'.
pub fn uses_simplified_colliders(min: i32, max: i32) -> bool {
    not_in_portal(min, max)
}

/// `CarriageContraptionEntity.updateRenderedPortalCutoff`: the blocks at the seam as the particle slice
/// (`(forward position * step, y, 0)`) and the mean `y` (integer division), for the portal's particles.
pub fn seam_slice(assembly_direction: Dir, blocks: impl IntoIterator<Item = Pos>, min: i32, max: i32) -> (Vec<Pos>, i32) {
    let forward = assembly_direction.clockwise_about(Axis::Y);
    let along_x = forward.axis() == Axis::X;
    let mut slice = Vec::new();
    let mut total = 0;
    for pos in blocks {
        if !at_seam(local_coord(assembly_direction, pos), min, max) {
            continue;
        }
        let p = if along_x { pos[0] } else { pos[2] } * forward.sign();
        let cell = [p, pos[1], 0];
        total += cell[1];
        slice.push(cell);
    }
    let avg = if slice.is_empty() { 0 } else { total / slice.len() as i32 };
    (slice, avg)
}

// --- the passengers ---------------------------------------------------------------------------------------------------------

/// A passenger saved while its seat was out of this side (`serialisedPassengers`), keyed by the seat's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    /// A player (`PlayerPassenger`: found by uuid) rather than a mob's data.
    pub player: bool,
    /// For a player: whether they are in this level (`passenger.level().dimension() == sLevel.dimension()`).
    pub in_this_dimension: bool,
}

/// Someone riding the carriage in this dimension now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rider {
    pub id: u128,
    pub player: bool,
    /// Where their seat is (`getSeatOf`).
    pub seat: Pos,
    /// `getSeatMapping().get(uuid)`: the seat's index, if the carriage knows it.
    pub seat_index: Option<i32>,
}

/// What `updatePassengerLoadout` decided.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loadout {
    /// Saved passengers that sit down again (`addSittingPassenger`). A saved player in another dimension isn't one.
    pub sit: Vec<i32>,
    /// Every saved passenger whose seat came back into view leaves `serialisedPassengers` (`loadedPassengers`), also the ones
    /// that couldn't be seated.
    pub forget: Vec<i32>,
    /// Riders whose seat is no longer within this side: `(rider id, seat index)`. A player is dismounted and saved
    /// (`dismountPlayer(.., capture = true)`), a mob is saved and discarded.
    pub put_off: Vec<(u128, i32, bool)>,
}

/// `DimensionalCarriageEntity.updatePassengerLoadout`. `seats` is `getContraption().getSeats()` in order.
pub fn passenger_loadout(
    assembly_direction: Dir,
    min: i32,
    max: i32,
    seats: &[Pos],
    saved: &BTreeMap<i32, Saved>,
    riders: &[Rider],
) -> Loadout {
    let mut out = Loadout::default();
    for (&seat_id, passenger) in saved {
        let Some(&local) = usize::try_from(seat_id).ok().and_then(|i| seats.get(i)) else { continue };
        if !is_local_coord_within(assembly_direction, local, min, max) {
            continue;
        }
        if !(passenger.player && !passenger.in_this_dimension) {
            out.sit.push(seat_id);
        }
        out.forget.push(seat_id);
    }
    for rider in riders {
        if is_local_coord_within(assembly_direction, rider.seat, min, max) {
            continue;
        }
        let Some(seat) = rider.seat_index else { continue };
        out.put_off.push((rider.id, seat, rider.player));
    }
    out
}

/// One dimension's entity of the carriage, as `dismountPlayer` looks at the others.
#[derive(Clone, Debug, PartialEq)]
pub struct OtherSide<'a> {
    pub dimension: &'a str,
    /// `otherDce.pivot.getLocation()`.
    pub pivot: Option<[f64; 3]>,
    /// `otherDce.positionAnchor`.
    pub position_anchor: Option<[f64; 3]>,
}

/// `dismountPlayer(.., capture = true)`'s teleports: for each other side of the carriage that the player is not in, the player
/// goes to its pivot (else its position anchor); a side with neither is skipped. The player is sent through them in order, so
/// the last is where they end up (and each sets the portal cooldown and awards `TRAIN_PORTAL`).
pub fn dismount_teleports<'a>(player_dimension: &str, this_dimension: &str, others: &[OtherSide<'a>]) -> Vec<(&'a str, [f64; 3])> {
    others
        .iter()
        .filter(|o| o.dimension != this_dimension && o.dimension != player_dimension)
        .filter_map(|o| o.pivot.or(o.position_anchor).map(|loc| (o.dimension, loc)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_coordinate_runs_along_the_carriage_from_its_front() {
        // A carriage assembled facing east is laid out along z (the clockwise axis), and the front is toward negative z * -1.
        assert_eq!(local_coord(Dir::East, [2, 5, 3]), -2);
        assert_eq!(local_coord(Dir::West, [2, 5, 3]), 2);
        // Facing south the carriage lies along x, which is `choose(z, y, x)`'s z-argument, the local z.
        assert_eq!(local_coord(Dir::South, [2, 5, 3]), -3);
        assert_eq!(local_coord(Dir::North, [2, 5, 3]), 3);
    }

    #[test]
    fn a_carriage_outside_portals_hides_nothing() {
        assert!(not_in_portal(i32::MIN, i32::MAX));
        assert!(!hidden_in_portal(0, i32::MIN, i32::MAX));
        assert!(!hidden_in_portal(1000, i32::MIN, i32::MAX));
        assert!(uses_simplified_colliders(i32::MIN, i32::MAX));
    }

    #[test]
    fn the_front_part_of_a_carriage_in_a_portal_shows_between_the_bounds_and_not_at_the_seam() {
        // `cutoff > 0`: the front (low coordinates) is gone, `min` is a real bound and `max` is open.
        let (min, max) = (-2, i32::MAX);
        assert!(hidden_in_portal(-3, min, max));
        // The seam itself is hidden, and the block past it shows.
        assert!(hidden_in_portal(-2, min, max));
        assert!(!hidden_in_portal(-1, min, max));
        assert!(!hidden_in_portal(4, min, max));
        assert!(!uses_simplified_colliders(min, max));
        assert!(within_visible(0, min, max) && !within_visible(-2, min, max));
        assert!(at_seam(-2, min, max));
    }

    #[test]
    fn the_far_side_hides_the_rest() {
        let (min, max) = (i32::MIN, 1);
        assert!(!hidden_in_portal(0, min, max));
        assert!(hidden_in_portal(1, min, max));
        assert!(hidden_in_portal(2, min, max));
    }

    #[test]
    fn a_block_is_hidden_by_its_place_along_the_carriage() {
        // East: coordinate = -local.x.
        assert!(block_hidden(Dir::East, [2, 0, 0], -1, i32::MAX));
        assert!(!block_hidden(Dir::East, [0, 0, 0], -1, i32::MAX));
        assert!(block_hidden(Dir::East, [1, 0, 0], -1, i32::MAX) == (local_coord(Dir::East, [1, 0, 0]) <= -1));
    }

    #[test]
    fn a_seat_counts_for_the_side_that_shows_its_coordinate() {
        assert!(is_local_coord_within(Dir::East, [0, 0, 0], -1, 3));
        assert!(is_local_coord_within(Dir::East, [-3, 0, 0], -1, 3));
        assert!(!is_local_coord_within(Dir::East, [-4, 0, 0], -1, 3));
        assert!(!is_local_coord_within(Dir::East, [2, 0, 0], -1, 3));
    }

    #[test]
    fn the_seam_slice_is_the_blocks_on_the_bound() {
        let blocks = [[0, 3, 0], [1, 5, 2], [-2, 7, 1], [-2, 9, 4]];
        // East: forward is south (clockwise), a carriage along z? local_coord east = -x, so the seam at coordinate 2 is x == -2.
        let (slice, avg) = seam_slice(Dir::East, blocks, 2, i32::MAX);
        assert_eq!(slice.len(), 2);
        assert_eq!(avg, 8);
        assert!(slice.iter().all(|c| c[2] == 0));
        // No seam, no slice.
        assert_eq!(seam_slice(Dir::East, blocks, i32::MIN, i32::MAX), (Vec::new(), 0));
    }

    fn seats() -> Vec<Pos> {
        // East: coordinate = -x: seat 0 at coordinate 0, seat 1 at -3 (x = 3), seat 2 at 2 (x = -2).
        vec![[0, 1, 0], [3, 1, 0], [-2, 1, 0]]
    }

    #[test]
    fn a_saved_passenger_sits_down_when_its_seat_comes_into_view() {
        let saved = BTreeMap::from([
            (0, Saved { player: false, in_this_dimension: false }),
            (1, Saved { player: false, in_this_dimension: false }),
            (2, Saved { player: true, in_this_dimension: false }),
            (7, Saved { player: false, in_this_dimension: false }),
        ]);
        // Showing coordinates 0 and below: seats 0 and 1 (coordinates 0 and -3) are in, seat 2 (2) is out.
        let out = passenger_loadout(Dir::East, i32::MIN, 1, &seats(), &saved, &[]);
        assert_eq!(out.sit, vec![0, 1]);
        assert_eq!(out.forget, vec![0, 1]);
        // Showing everything: the player is saved for another dimension, so nobody sits for them, but they are forgotten.
        let out = passenger_loadout(Dir::East, i32::MIN, i32::MAX, &seats(), &saved, &[]);
        assert_eq!(out.sit, vec![0, 1]);
        assert_eq!(out.forget, vec![0, 1, 2]);
        // A player saved here does sit.
        let here = BTreeMap::from([(2, Saved { player: true, in_this_dimension: true })]);
        assert_eq!(passenger_loadout(Dir::East, i32::MIN, i32::MAX, &seats(), &here, &[]).sit, vec![2]);
    }

    #[test]
    fn a_rider_whose_seat_leaves_the_visible_part_is_put_off() {
        let riders = [
            Rider { id: 1, player: true, seat: [3, 1, 0], seat_index: Some(1) },
            Rider { id: 2, player: false, seat: [0, 1, 0], seat_index: Some(0) },
            Rider { id: 3, player: false, seat: [3, 1, 0], seat_index: None },
        ];
        // Showing coordinates -1 and over: seat 1 (-3) is out; seat 0 (0) and seat 2 (2) are in.
        let out = passenger_loadout(Dir::East, -1, i32::MAX, &seats(), &BTreeMap::new(), &riders);
        assert_eq!(out.put_off, vec![(1, 1, true)]);
        // A rider the carriage has no seat index for stays (`!mapping.containsKey`).
        let out = passenger_loadout(Dir::East, -1, i32::MAX, &seats(), &BTreeMap::new(), &riders[2..]);
        assert!(out.put_off.is_empty());
    }

    #[test]
    fn a_player_put_off_goes_to_the_other_dimension_s_pivot_or_anchor() {
        let others = [
            OtherSide { dimension: "minecraft:the_nether", pivot: Some([1.0, 2.0, 3.0]), position_anchor: Some([9.0, 9.0, 9.0]) },
            OtherSide { dimension: "minecraft:overworld", pivot: None, position_anchor: Some([4.0, 5.0, 6.0]) },
            OtherSide { dimension: "minecraft:the_end", pivot: None, position_anchor: None },
        ];
        // The carriage's own side (the overworld) is skipped, and so is a side with nowhere to put the player.
        let tp = dismount_teleports("minecraft:overworld", "minecraft:overworld", &others);
        assert_eq!(tp, vec![("minecraft:the_nether", [1.0, 2.0, 3.0])]);
        // A player already in the nether isn't sent there, but is sent to the others.
        let tp = dismount_teleports("minecraft:the_nether", "minecraft:overworld", &others);
        assert!(tp.is_empty());
    }
}

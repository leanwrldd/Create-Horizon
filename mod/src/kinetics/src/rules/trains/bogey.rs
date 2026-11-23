//! Bogeys: `BogeySizes` and the numbers of `AbstractBogeyBlock` / `StandardBogeyBlock`
//! (content/trains/bogey/) the train rules use: wheel point spacing, wheel radius, the track type a
//! bogey runs on, whether it can hang upside down, the coupling anchor.
//!
//! A bogey is its [`BogeyType`] here; the block, its style and its block entity are the glue's.
//! Left out: styles (`BogeyStyle`, only render and cycling), the wrench cycling and block state code.

use super::navigation::TRACK_TYPE_STANDARD;
use super::vec::Vec3;

/// `BogeySizes.BogeySize`: an id and the wheel radius (blocks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BogeySize {
    pub id: &'static str,
    pub wheel_radius: f32,
}

/// `BogeySizes.SMALL`.
pub const SMALL: BogeySize = BogeySize { id: "create:small", wheel_radius: 6.5 / 16.0 };
/// `BogeySizes.LARGE`.
pub const LARGE: BogeySize = BogeySize { id: "create:large", wheel_radius: 12.5 / 16.0 };

/// `BogeySizes.allSortedIncreasing()` with the two Create registers.
pub const SIZES_INCREASING: [BogeySize; 2] = [SMALL, LARGE];

impl BogeySize {
    /// `nextBySize()`: the next larger size, wrapping round.
    pub fn next_by_size(&self) -> BogeySize {
        let i = SIZES_INCREASING.iter().position(|s| s.id == self.id).unwrap_or(0);
        SIZES_INCREASING[(i + 1) % SIZES_INCREASING.len()]
    }
}

/// An `AbstractBogeyBlock`'s numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct BogeyType {
    /// The block's registry id.
    pub id: String,
    pub size: BogeySize,
    /// `getWheelPointSpacing()`.
    pub wheel_point_spacing: f64,
    /// `getWheelRadius()`.
    pub wheel_radius: f64,
    /// `getTrackType(style).id`.
    pub track_type: String,
    /// `canBeUpsideDown()`.
    pub can_be_upside_down: bool,
    /// `allowsSingleBogeyCarriage()`.
    pub allows_single_bogey_carriage: bool,
    /// `getConnectorAnchorOffset()` (not upside down).
    pub connector_anchor_offset: Vec3,
}

impl BogeyType {
    /// `StandardBogeyBlock` of a size: spacing 2, radius `(12.5 or 6.5) / 16`, anchor `(0, 7/32f, 1)`.
    pub fn standard(id: &str, size: BogeySize) -> BogeyType {
        BogeyType {
            id: id.to_string(),
            size,
            wheel_point_spacing: 2.0,
            wheel_radius: (if size.id == LARGE.id { 12.5 } else { 6.5 }) / 16.0,
            track_type: TRACK_TYPE_STANDARD.to_string(),
            can_be_upside_down: false,
            allows_single_bogey_carriage: true,
            connector_anchor_offset: [0.0, f64::from(7.0_f32 / 32.0), 1.0],
        }
    }

    /// `create:small_bogey`.
    pub fn small() -> BogeyType {
        BogeyType::standard("create:small_bogey", SMALL)
    }

    /// `create:large_bogey`.
    pub fn large() -> BogeyType {
        BogeyType::standard("create:large_bogey", LARGE)
    }

    /// `getValidPathfindingTypes(style)`: only the bogey's own track type.
    pub fn valid_pathfinding_types(&self) -> Vec<String> {
        vec![self.track_type.clone()]
    }

    /// `isOnIncompatibleTrack`: the edge's track type is not the bogey's. `None` is no edge.
    pub fn is_on_incompatible_track(&self, edge_track_type: Option<&str>) -> bool {
        edge_track_type.is_some_and(|t| t != self.track_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_bogeys() {
        let s = BogeyType::small();
        let l = BogeyType::large();
        assert_eq!(s.wheel_point_spacing, 2.0);
        assert_eq!(s.wheel_radius, 6.5 / 16.0);
        assert_eq!(l.wheel_radius, 12.5 / 16.0);
        assert_eq!(s.connector_anchor_offset[2], 1.0);
        assert_eq!(s.valid_pathfinding_types(), vec!["create:standard".to_string()]);
    }

    #[test]
    fn sizes_cycle() {
        assert_eq!(SMALL.next_by_size(), LARGE);
        assert_eq!(LARGE.next_by_size(), SMALL);
    }

    #[test]
    fn incompatible_track() {
        let s = BogeyType::small();
        assert!(!s.is_on_incompatible_track(None));
        assert!(!s.is_on_incompatible_track(Some("create:standard")));
        assert!(s.is_on_incompatible_track(Some("other:track")));
    }
}

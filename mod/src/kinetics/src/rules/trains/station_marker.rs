//! Station markers on filled maps: `StationMarker` (what a map remembers of a station), `StationMapData` /
//! `MapItemSavedDataMixin` (toggling one with a click, putting it at its place on the map, pruning it when the station
//! changes, the 256 tracked-decoration limit) and `TrackTargetingBehaviour.getPositionForMapMarker`.
//!
//! Ported from `content/trains/station/{StationMarker,StationMapData}.java`, `foundation/mixin/MapItemSavedDataMixin.java` and
//! `AllMapDecorationTypes` (the station type tracks its count and shows in item frames). The map's own state lives in the
//! caller: [`StationMarkers`] holds what the mixin adds to `MapItemSavedData`, and the decorations the markers make.
//!
//! Differences from Create, for the README:
//! - The name is plain text, not a `Component` JSON (`Component.literal(name)` is all `fromWorld` ever makes).
//! - A decoration's `name` is kept, but Horizon's `map_decorations` hook can't draw text (see `docs/trains.md`, glue part 6b).

use std::collections::BTreeMap;

/// `MapItemSavedData.isTrackedCountOverLimit(256)`'s limit, asked by `toggleStation`.
pub const TRACKED_LIMIT: i32 = 256;
/// `if (localX < -63.0F || localX > 63.0F ...)`: a marker must be inside this many pixels of the map's centre.
pub const WINDOW: f64 = 63.0;

/// `StationMarker`: the station's block, the block the marker is shown at, and the name.
#[derive(Clone, Debug)]
pub struct StationMarker {
    pub source: [i32; 3],
    pub target: [i32; 3],
    pub name: String,
}

/// `StationMarker.equals`: only the target and the name count (the source doesn't).
impl PartialEq for StationMarker {
    fn eq(&self, other: &StationMarker) -> bool {
        self.target == other.target && self.name == other.name
    }
}

impl Eq for StationMarker {}

impl StationMarker {
    pub fn new(source: [i32; 3], target: [i32; 3], name: &str) -> StationMarker {
        StationMarker { source, target, name: name.to_owned() }
    }

    /// `getId()`: `"create:station-" + x + "," + y + "," + z` of the target.
    pub fn id(&self) -> String {
        format!("create:station-{},{},{}", self.target[0], self.target[1], self.target[2])
    }

    /// `save`, as text: `sx,sy,sz;tx,ty,tz;name` (the name last, it may hold anything).
    pub fn save(&self) -> String {
        let p = |v: [i32; 3]| format!("{},{},{}", v[0], v[1], v[2]);
        format!("{};{};{}", p(self.source), p(self.target), self.name)
    }

    /// `load`.
    pub fn load(text: &str) -> Option<StationMarker> {
        let mut parts = text.splitn(3, ';');
        let pos = |s: &str| -> Option<[i32; 3]> {
            let mut it = s.split(',');
            Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?])
        };
        let source = pos(parts.next()?)?;
        let target = pos(parts.next()?)?;
        Some(StationMarker { source, target, name: parts.next()?.to_owned() })
    }
}

/// `TrackTargetingBehaviour.getPositionForMapMarker` for a straight track: the targeted block.
pub fn marker_position_straight(target_track: [i32; 3]) -> [i32; 3] {
    target_track
}

/// `getPositionForMapMarker` for a curve: `BlockPos.containing(bc.getPosition(t))` with `t = (segment + 1) / floor(length * 2)`.
/// `position_at` is the connection's `getPosition`; `None` (no connection at the track) falls back to the targeted block.
pub fn marker_position_curve(
    target_track: [i32; 3],
    curve_length: f64,
    segment: i32,
    position_at: Option<&dyn Fn(f64) -> [f64; 3]>,
) -> [i32; 3] {
    let Some(position_at) = position_at else { return target_track };
    let length = (curve_length * 2.0).floor();
    let t = f64::from(segment + 1) / length;
    let p = position_at(t);
    [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
}

/// `MapDecoration` of the station type: a place on the map in half pixels, and the name shown under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoration {
    pub x: i8,
    pub y: i8,
    pub name: String,
}

/// `Math` of `(byte) (int) (local * 2.0F + 0.5F)`: Java's float to int truncates towards zero and a byte keeps the low 8 bits.
fn half_pixels(local: f32) -> i8 {
    ((local * 2.0 + 0.5) as i32) as i8
}

/// What `MapItemSavedDataMixin` adds to a map: its station markers, and the decorations they make.
#[derive(Clone, Debug, Default)]
pub struct StationMarkers {
    /// `centerX` and `centerZ`, and `scale`.
    pub center: [i32; 2],
    pub scale: u8,
    markers: BTreeMap<String, StationMarker>,
    decorations: BTreeMap<String, Decoration>,
    /// `trackedDecorationCount`, of the decorations this map has (vanilla's included, as given to [`StationMarkers::new`]).
    tracked: i32,
    /// `setDecorationsDirty`.
    pub dirty: bool,
}

impl StationMarkers {
    /// A map at `center` with `scale` and `tracked` decorations already counted (a banner is one, a player's pointer isn't).
    pub fn new(center: [i32; 2], scale: u8, tracked: i32) -> StationMarkers {
        StationMarkers { center, scale, tracked, ..StationMarkers::default() }
    }

    /// The markers in id order.
    pub fn markers(&self) -> impl Iterator<Item = &StationMarker> {
        self.markers.values()
    }

    /// The decorations the markers make, by id.
    pub fn decorations(&self) -> &BTreeMap<String, Decoration> {
        &self.decorations
    }

    pub fn tracked(&self) -> i32 {
        self.tracked
    }

    /// `1 << scale`.
    fn multiplier(&self) -> i32 {
        1 << self.scale
    }

    /// `isTrackedCountOverLimit(256)`.
    pub fn over_limit(&self) -> bool {
        self.tracked >= TRACKED_LIMIT
    }

    fn remove_decoration(&mut self, id: &str) {
        if self.decorations.remove(id).is_some() {
            self.tracked -= 1;
            self.dirty = true;
        }
    }

    /// `addStationMarker`: remember the marker, and show it unless it lies outside the map (then its decoration goes).
    pub fn add(&mut self, marker: StationMarker) {
        let id = marker.id();
        let m = self.multiplier() as f32;
        let local_x = (marker.target[0] - self.center[0]) as f32 / m;
        let local_z = (marker.target[2] - self.center[1]) as f32 / m;
        let outside = local_x < -63.0 || local_x > 63.0 || local_z < -63.0 || local_z > 63.0;
        let name = marker.name.clone();
        self.markers.insert(id.clone(), marker);
        if outside {
            self.remove_decoration(&id);
            return;
        }
        let decoration = Decoration { x: half_pixels(local_x), y: half_pixels(local_z), name };
        let old = self.decorations.insert(id, decoration.clone());
        if old.as_ref() != Some(&decoration) {
            // The station type tracks its count: the old one stops counting, the new one starts.
            if old.is_none() {
                self.tracked += 1;
            }
            self.dirty = true;
        }
    }

    /// `toggleStation`: false when the block is off the map or isn't a station (`marker` is `StationMarker.fromWorld` of it); a
    /// marker that is there is taken away (true), else one is added unless the map tracks too many decorations.
    pub fn toggle(&mut self, block: [i32; 3], marker: Option<StationMarker>) -> bool {
        let m = f64::from(self.multiplier());
        let local_x = (f64::from(block[0]) + 0.5 - f64::from(self.center[0])) / m;
        let local_z = (f64::from(block[2]) + 0.5 - f64::from(self.center[1])) / m;
        if !(-WINDOW..=WINDOW).contains(&local_x) || !(-WINDOW..=WINDOW).contains(&local_z) {
            return false;
        }
        let Some(marker) = marker else { return false };
        let id = marker.id();
        // `Map.remove(key, value)`: only when the saved marker equals this one.
        if self.markers.get(&id) == Some(&marker) {
            self.markers.remove(&id);
            self.remove_decoration(&id);
            return true;
        }
        if !self.over_limit() {
            self.add(marker);
            return true;
        }
        false
    }

    /// `create$checkStations(blockGetter, x, z)`: a marker at that column whose station has changed is dropped, and put back
    /// from the station when it still targets the same block. `lookup` is `StationMarker.fromWorld` at a source.
    pub fn check_stations(&mut self, x: i32, z: i32, lookup: &dyn Fn([i32; 3]) -> Option<StationMarker>) {
        let mut renewed = Vec::new();
        let ids: Vec<String> = self.markers.keys().cloned().collect();
        for id in ids {
            let marker = self.markers[&id].clone();
            if marker.target[0] != x || marker.target[2] != z {
                continue;
            }
            let other = lookup(marker.source);
            if other.as_ref() != Some(&marker) {
                self.markers.remove(&id);
                self.remove_decoration(&id);
                if let Some(other) = other
                    && other.target == marker.target
                {
                    renewed.push(other);
                }
            }
        }
        for marker in renewed {
            self.add(marker);
        }
    }

    /// The markers as saved (`create:stations`), each as [`StationMarker::save`].
    pub fn save(&self) -> Vec<String> {
        self.markers.values().map(StationMarker::save).collect()
    }

    /// `create$onLoad`: each saved marker is added.
    pub fn load(&mut self, saved: &[String]) {
        for text in saved {
            if let Some(marker) = StationMarker::load(text) {
                self.add(marker);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(x: i32, z: i32, name: &str) -> StationMarker {
        StationMarker::new([x, 64, z], [x, 64, z], name)
    }

    #[test]
    fn the_id_names_the_target() {
        assert_eq!(StationMarker::new([0, 0, 0], [1, -2, 3], "A").id(), "create:station-1,-2,3");
    }

    #[test]
    fn markers_are_equal_by_target_and_name() {
        let a = StationMarker::new([0, 0, 0], [5, 5, 5], "Central");
        assert_eq!(a, StationMarker::new([9, 9, 9], [5, 5, 5], "Central"));
        assert_ne!(a, StationMarker::new([0, 0, 0], [5, 5, 5], "Other"));
        assert_ne!(a, StationMarker::new([0, 0, 0], [5, 5, 6], "Central"));
    }

    #[test]
    fn a_marker_is_saved_and_loaded() {
        let m = StationMarker::new([1, 2, 3], [4, 5, 6], "East; Gate");
        let back = StationMarker::load(&m.save()).expect("load");
        assert_eq!((back.source, back.target, back.name.as_str()), ([1, 2, 3], [4, 5, 6], "East; Gate"));
        assert!(StationMarker::load("nope").is_none());
        assert!(StationMarker::load("1,2;3,4,5;x").is_none());
    }

    #[test]
    fn a_curve_marker_sits_one_segment_further_along() {
        let line = |t: f64| [t * 40.0, 70.5, 3.2];
        // length 10 -> 20 segments; segment 4 -> t = 5 / 20.
        assert_eq!(marker_position_curve([0, 0, 0], 10.0, 4, Some(&line as &dyn Fn(f64) -> [f64; 3])), [10, 70, 3]);
        assert_eq!(marker_position_curve([1, 2, 3], 10.0, 4, None), [1, 2, 3]);
        assert_eq!(marker_position_straight([7, 8, 9]), [7, 8, 9]);
    }

    #[test]
    fn a_marker_is_put_at_its_place_in_half_pixels() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        map.add(marker(10, -4, "A"));
        let d = &map.decorations()["create:station-10,64,-4"];
        // (10 / 1) * 2 + 0.5 = 20.5 -> 20; (-4) * 2 + 0.5 = -7.5 -> -7 (truncated towards zero).
        assert_eq!((d.x, d.y), (20, -7));
        assert_eq!(map.tracked(), 1);
        assert!(map.dirty);
    }

    #[test]
    fn the_scale_shrinks_the_distance() {
        let mut map = StationMarkers::new([100, 100], 2, 0);
        map.add(marker(140, 60, "A"));
        let d = &map.decorations()["create:station-140,64,60"];
        // (40 / 4) * 2 + 0.5 = 20.5, (-40 / 4) * 2 + 0.5 = -19.5.
        assert_eq!((d.x, d.y), (20, -19));
    }

    #[test]
    fn a_marker_off_the_map_is_remembered_but_not_shown() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        map.add(marker(64, 0, "Far"));
        assert!(map.decorations().is_empty());
        assert_eq!(map.markers().count(), 1);
        assert_eq!(map.tracked(), 0);
        // The edge is still on the map.
        map.add(marker(63, 0, "Edge"));
        assert_eq!(map.decorations().len(), 1);
        assert_eq!(map.tracked(), 1);
    }

    #[test]
    fn adding_the_same_marker_again_changes_nothing() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        map.add(marker(3, 3, "A"));
        map.dirty = false;
        map.add(marker(3, 3, "A"));
        assert!(!map.dirty);
        assert_eq!(map.tracked(), 1);
        map.add(marker(3, 3, "Renamed"));
        assert!(map.dirty);
        assert_eq!(map.tracked(), 1);
        assert_eq!(map.decorations()["create:station-3,64,3"].name, "Renamed");
    }

    #[test]
    fn a_click_adds_and_the_next_click_removes() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        assert!(map.toggle([5, 64, 5], Some(marker(5, 5, "A"))));
        assert_eq!(map.markers().count(), 1);
        assert!(map.toggle([5, 64, 5], Some(marker(5, 5, "A"))));
        assert_eq!(map.markers().count(), 0);
        assert_eq!(map.tracked(), 0);
        // A renamed station is a different marker: the click replaces the old one.
        map.toggle([5, 64, 5], Some(marker(5, 5, "A")));
        assert!(map.toggle([5, 64, 5], Some(marker(5, 5, "B"))));
        assert_eq!(map.markers().next().map(|m| m.name.as_str()), Some("B"));
    }

    #[test]
    fn a_click_fails_off_the_map_or_without_a_station() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        assert!(!map.toggle([200, 64, 0], Some(marker(200, 0, "A"))));
        assert!(!map.toggle([5, 64, 5], None));
        // The block's centre (+0.5) is what is measured: 62 + 0.5 is inside, 63 + 0.5 is not.
        assert!(map.toggle([62, 64, 0], Some(marker(62, 0, "A"))));
        assert!(!map.toggle([63, 64, 0], Some(marker(63, 0, "B"))));
    }

    #[test]
    fn a_map_with_too_many_decorations_takes_no_more() {
        let mut map = StationMarkers::new([0, 0], 0, 256);
        assert!(map.over_limit());
        assert!(!map.toggle([5, 64, 5], Some(marker(5, 5, "A"))));
        let mut map = StationMarkers::new([0, 0], 0, 255);
        assert!(map.toggle([5, 64, 5], Some(marker(5, 5, "A"))));
        assert!(map.over_limit());
    }

    #[test]
    fn a_changed_station_is_taken_off_or_renewed() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        map.add(StationMarker::new([1, 60, 1], [5, 64, 5], "A"));
        // Another column: nothing is asked.
        map.check_stations(6, 6, &|_| panic!("not this column"));
        assert_eq!(map.markers().count(), 1);
        // The station was renamed: the marker is made again with the new name.
        map.check_stations(5, 5, &|source| Some(StationMarker::new(source, [5, 64, 5], "B")));
        assert_eq!(map.markers().next().map(|m| m.name.as_str()), Some("B"));
        assert_eq!(map.decorations()["create:station-5,64,5"].name, "B");
        // The station now targets another block: the marker goes.
        map.check_stations(5, 5, &|source| Some(StationMarker::new(source, [9, 64, 9], "B")));
        assert_eq!(map.markers().count(), 0);
        assert_eq!(map.tracked(), 0);
        // The station is gone.
        map.add(StationMarker::new([1, 60, 1], [5, 64, 5], "A"));
        map.check_stations(5, 5, &|_| None);
        assert_eq!(map.markers().count(), 0);
    }

    #[test]
    fn markers_survive_a_save() {
        let mut map = StationMarkers::new([0, 0], 0, 0);
        map.add(marker(5, 5, "A"));
        map.add(marker(-8, 12, "B"));
        let saved = map.save();
        let mut loaded = StationMarkers::new([0, 0], 0, 0);
        loaded.load(&saved);
        assert_eq!(loaded.decorations(), map.decorations());
        assert_eq!(loaded.tracked(), 2);
    }
}

//! `Carriage`, `CarriageBogey` and the cargo counters of `TrainCargoManager`
//! (content/trains/entity/Carriage.java, CarriageBogey.java, content/contraptions/minecart/TrainCargoManager.java).
//!
//! A carriage is one or two bogeys (each a pair of [`TravellingPoint`]s) and the spacing between
//! them. [`Carriage::travel`] moves its wheels along the graph by a distance, the leading wheel first
//! (or the trailing one first when going backwards), each wheel following the one in front of it
//! (or, at the end of the train, the navigation), pulling the bogeys together or apart by the stress
//! between them. [`Carriage::update_contraption_anchors`] then places the carriage entity's anchors,
//! per dimension.
//!
//! What the signal listeners and the navigation selector do is the train's: `travel` is handed them
//! in a [`TravelCtx`].
//!
//! Left out (glue): the entities, passengers and their NBT (`DimensionalCarriageEntity`'s entity
//! handling, `manageEntities` beyond reporting the dimensions the points traverse,
//! `updatePassengerLoadout`, `dismountPlayer`, `alignEntity`), `CarriageBogey.updateAngles` and
//! `updateCouplingAnchor` (rendering), the bogey's style and its NBT (`bogey_data` is an opaque
//! payload the glue keeps), `Carriage.write`/`read`. `Create.RANDOM` (the derail angle) and
//! `netIdGenerator` are arguments.

use super::bogey::BogeyType;
use super::graph::{NodeId, TrackNodeLocation};
use super::network::TrackGraph;
use super::travelling_point::{Candidate, EdgePointHit, Listeners, TrackSelector, TravellingPoint};
use super::vec::{Vec3, add, distance_to, lerp, normalize, scale, sub};
use std::collections::BTreeMap;

/// `Carriage.FIRST` and friends: where the carriage is in the train.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarriageKind {
    First,
    Middle,
    Last,
    Both,
}

impl CarriageKind {
    /// The kind of carriage `i` of `count`.
    pub fn of(i: usize, count: usize) -> CarriageKind {
        let first = i == 0;
        let last = i + 1 == count;
        match (first, last) {
            (true, true) => CarriageKind::Both,
            (true, false) => CarriageKind::First,
            (false, true) => CarriageKind::Last,
            (false, false) => CarriageKind::Middle,
        }
    }
}

/// `TrainCargoManager`'s counters and the contents the wait conditions look at. The glue keeps the
/// slots current (and calls [`CargoState::change_detected`] when something moves).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CargoState {
    /// `ticksSinceLastExchange`.
    pub ticks_since_last_exchange: i32,
    /// `version`.
    pub version: i32,
    /// Every item slot: the item's id, count, max stack size and, for a package, its address.
    pub items: Vec<CargoItem>,
    /// Every tank: the fluid's id and amount (mB).
    pub fluids: Vec<(String, i32)>,
}

/// An item slot of a carriage's storage.
#[derive(Clone, Debug, PartialEq)]
pub struct CargoItem {
    pub id: String,
    pub count: i32,
    pub max_stack: i32,
    /// A package's address.
    pub package_address: Option<String>,
}

impl CargoState {
    /// `resetIdleCargoTracker()`.
    pub fn reset_idle_cargo_tracker(&mut self) {
        self.ticks_since_last_exchange = 0;
    }

    /// `tickIdleCargoTracker()`.
    pub fn tick_idle_cargo_tracker(&mut self) {
        self.ticks_since_last_exchange += 1;
    }

    /// `changeDetected()`: something was put in or taken out.
    pub fn change_detected(&mut self) {
        self.version += 1;
        self.reset_idle_cargo_tracker();
    }
}

/// `CarriageBogey`.
#[derive(Clone, Debug, PartialEq)]
pub struct CarriageBogey {
    pub ty: BogeyType,
    upside_down: bool,
    /// `points`: the leading wheel first.
    pub points: [TravellingPoint; 2],
    /// `bogeyData`, opaque: the glue's NBT.
    pub bogey_data: Vec<u8>,
    /// `derailAngle`.
    pub derail_angle: i32,
}

impl CarriageBogey {
    /// `new CarriageBogey(type, upsideDown, data, point, point2)`.
    pub fn new(ty: BogeyType, upside_down: bool, bogey_data: Vec<u8>, mut p1: TravellingPoint, mut p2: TravellingPoint, derail_angle: i32) -> CarriageBogey {
        let upside_down = ty.can_be_upside_down && upside_down;
        p1.upside_down = upside_down;
        p2.upside_down = upside_down;
        CarriageBogey { ty, upside_down, points: [p1, p2], bogey_data, derail_angle }
    }

    /// `isUpsideDown()`.
    pub fn is_upside_down(&self) -> bool {
        self.ty.can_be_upside_down && self.upside_down
    }

    /// `leading()`.
    pub fn leading(&self) -> &TravellingPoint {
        &self.points[0]
    }

    /// `trailing()`.
    pub fn trailing(&self) -> &TravellingPoint {
        &self.points[1]
    }

    /// `getDimension()`: the dimension both wheels are in, none when an edge is missing or between
    /// dimensions or they differ.
    pub fn dimension(&self, graph: &TrackGraph) -> Option<u32> {
        let l = self.leading().edge(graph)?;
        let t = self.trailing().edge(graph)?;
        if l.is_inter_dimensional() || t.is_inter_dimensional() {
            return None;
        }
        let d1 = node_dimension(graph, self.leading().node1)?;
        let d2 = node_dimension(graph, self.trailing().node1)?;
        if d1 == d2 { Some(d1) } else { None }
    }

    /// `getStress()`: how far the wheels are from their spacing.
    pub fn stress(&self, graph: &TrackGraph, derailed: bool) -> f64 {
        if self.dimension(graph).is_none() || derailed {
            return 0.0;
        }
        let (Some(a), Some(b)) = (self.leading().get_position(Some(graph), false), self.trailing().get_position(Some(graph), false)) else {
            return 0.0;
        };
        self.ty.wheel_point_spacing - distance_to(a, b)
    }

    /// `getAnchorPosition(flipUpsideDown)`: halfway between the wheels, none when off the track.
    pub fn anchor_position(&self, graph: &TrackGraph, flip_upside_down: bool) -> Option<Vec3> {
        self.leading().edge(graph)?;
        let a = self.points[0].get_position(Some(graph), flip_upside_down)?;
        let b = self.points[1].get_position(Some(graph), flip_upside_down)?;
        Some(scale(add(a, b), 0.5))
    }
}

fn node_dimension(graph: &TrackGraph, node: Option<NodeId>) -> Option<u32> {
    Some(graph.get_node(node?)?.location.dimension)
}

/// `DimensionalCarriageEntity` without the entity: where the carriage is in one dimension.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DimensionalCarriage {
    pub position_anchor: Option<Vec3>,
    pub rotation_anchors: [Option<Vec3>; 2],
    pub pivot: Option<TrackNodeLocation>,
    /// 0 whole, 0 to 1 fading out, -1 to 0 fading in.
    pub cutoff: f32,
    pub discard_ticks: i32,
    /// Set when the cutoff changed: the glue refreshes the rendered cutoff and the passengers
    /// (`updateRenderedCutoff`, `updatePassengerLoadout`), then clears it.
    pub cutoff_changed: bool,
}

/// What `updateCutoff` reads of the carriage.
#[derive(Clone, Copy, Debug)]
pub struct CarriageShape {
    pub leading_half_spacing: f64,
    pub trailing_half_spacing: f64,
    pub bogey_spacing: i32,
    pub on_two_bogeys: bool,
}

impl DimensionalCarriage {
    /// `leadingAnchor()`.
    pub fn leading_anchor(&self, on_two_bogeys: bool) -> Option<Vec3> {
        if on_two_bogeys { self.rotation_anchors[0] } else { self.position_anchor }
    }

    /// `trailingAnchor()`.
    pub fn trailing_anchor(&self, on_two_bogeys: bool) -> Option<Vec3> {
        if on_two_bogeys { self.rotation_anchors[1] } else { self.position_anchor }
    }

    /// `minAllowedLocalCoord()`.
    pub fn min_allowed_local_coord(&self, bogey_spacing: i32) -> i32 {
        if self.cutoff <= 0.0 {
            return i32::MIN;
        }
        if self.cutoff >= 1.0 {
            return i32::MAX;
        }
        let b = f64::from(bogey_spacing);
        (-b + -1.0 + (2.0 + b) * f64::from(self.cutoff)).floor() as i32
    }

    /// `maxAllowedLocalCoord()`.
    pub fn max_allowed_local_coord(&self, bogey_spacing: i32) -> i32 {
        if self.cutoff >= 0.0 {
            return i32::MAX;
        }
        if self.cutoff <= -1.0 {
            return i32::MIN;
        }
        let b = f64::from(bogey_spacing);
        (-b + -1.0 + (2.0 + b) * f64::from(self.cutoff + 1.0)).ceil() as i32
    }

    /// `discardPivot()`.
    pub fn discard_pivot(&mut self, bogey_spacing: i32) {
        let (pmin, pmax) = (self.min_allowed_local_coord(bogey_spacing), self.max_allowed_local_coord(bogey_spacing));
        self.cutoff = 0.0;
        self.pivot = None;
        if pmin != self.min_allowed_local_coord(bogey_spacing) || pmax != self.max_allowed_local_coord(bogey_spacing) {
            self.cutoff_changed = true;
        }
    }

    /// `updateCutoff(leadingIsCurrent)`.
    pub fn update_cutoff(&mut self, leading_is_current: bool, shape: &CarriageShape) {
        let (Some(mut leading_anchor), Some(mut trailing_anchor)) = (self.rotation_anchors[0], self.rotation_anchors[1]) else {
            return;
        };
        let Some(pivot) = self.pivot else {
            self.cutoff = 0.0;
            return;
        };
        let pivot_loc = add(pivot.location(), [0.0, 1.0, 0.0]);
        let leading_spacing = shape.leading_half_spacing;
        let trailing_spacing = shape.trailing_half_spacing;
        let anchor_spacing = leading_spacing + f64::from(shape.bogey_spacing) + trailing_spacing;
        if shape.on_two_bogeys {
            let diff = normalize(sub(trailing_anchor, leading_anchor));
            trailing_anchor = add(trailing_anchor, scale(diff, trailing_spacing));
            leading_anchor = add(leading_anchor, scale(diff, -leading_spacing));
        }
        let mut leading_diff = distance_to(leading_anchor, pivot_loc);
        let mut trailing_diff = distance_to(trailing_anchor, pivot_loc);
        leading_diff /= anchor_spacing;
        trailing_diff /= anchor_spacing;

        self.cutoff = if leading_is_current && leading_diff > trailing_diff && leading_diff > 1.0 {
            0.0
        } else if leading_is_current && leading_diff < trailing_diff && trailing_diff > 1.0 {
            1.0
        } else if !leading_is_current && leading_diff > trailing_diff && leading_diff > 1.0 {
            -1.0
        } else if !leading_is_current && leading_diff < trailing_diff && trailing_diff > 1.0 {
            0.0
        } else {
            let d = if leading_is_current { leading_diff } else { trailing_diff };
            ((1.0 - d).clamp(0.0, 1.0) * if leading_is_current { 1.0 } else { -1.0 }) as f32
        };
    }
}

/// What a carriage's travel needs of the train: its way at forks, its edge point listeners.
pub struct TravelCtx<'a> {
    pub derailed: bool,
    /// `navigation.control(point)`: picks the way at a fork for a wheel that follows nothing.
    pub control: &'a mut dyn FnMut(&TrackGraph, &TravellingPoint, bool, &[Candidate]) -> usize,
    /// `train.frontSignalListener()`.
    pub front: &'a mut dyn FnMut(f64, &EdgePointHit) -> bool,
    /// `train.backSignalListener()`.
    pub back: &'a mut dyn FnMut(f64, &EdgePointHit) -> bool,
    /// `train.status.doublePortal()`.
    pub double_portal: &'a mut dyn FnMut(),
}

/// How a wheel picks its way at a fork.
enum Pick {
    Control,
    Follow(TravellingPoint),
}

/// `Carriage`.
#[derive(Clone, Debug, PartialEq)]
pub struct Carriage {
    pub id: i32,
    pub blocked: bool,
    pub stalled: bool,
    /// `presentConductors`: a forward driver, a backward driver.
    pub present_conductors: [bool; 2],
    pub bogey_spacing: i32,
    /// `bogeys`: the leading bogey, and the trailing one when the carriage has two.
    pub bogeys: (CarriageBogey, Option<CarriageBogey>),
    pub storage: CargoState,
    /// `entities`' positioning, per dimension.
    pub dimensions: BTreeMap<u32, DimensionalCarriage>,
    /// The dimensions the wheels traverse after the last travel (`currentlyTraversedDimensions`).
    pub traversed_dimensions: Vec<u32>,
}

impl Carriage {
    /// `new Carriage(bogey1, bogey2, bogeySpacing)`.
    pub fn new(id: i32, bogey1: CarriageBogey, bogey2: Option<CarriageBogey>, bogey_spacing: i32) -> Carriage {
        Carriage {
            id,
            blocked: false,
            stalled: false,
            present_conductors: [false, false],
            bogey_spacing,
            bogeys: (bogey1, bogey2),
            storage: CargoState::default(),
            dimensions: BTreeMap::new(),
            traversed_dimensions: Vec::new(),
        }
    }

    pub fn is_on_two_bogeys(&self) -> bool {
        self.bogeys.1.is_some()
    }

    /// `leadingBogey()`.
    pub fn leading_bogey(&self) -> &CarriageBogey {
        &self.bogeys.0
    }

    /// `trailingBogey()`.
    pub fn trailing_bogey(&self) -> &CarriageBogey {
        self.bogeys.1.as_ref().unwrap_or(&self.bogeys.0)
    }

    /// `getLeadingPoint()`.
    pub fn leading_point(&self) -> &TravellingPoint {
        self.leading_bogey().leading()
    }

    /// `getTrailingPoint()`.
    pub fn trailing_point(&self) -> &TravellingPoint {
        self.trailing_bogey().trailing()
    }

    /// `presentInMultipleDimensions()`: `entities.size() > 1`.
    pub fn present_in_multiple_dimensions(&self) -> bool {
        self.dimensions.len() > 1
    }

    /// `getDimensional(dimension)`.
    pub fn dimensional(&mut self, dimension: u32) -> &mut DimensionalCarriage {
        self.dimensions.entry(dimension).or_default()
    }

    /// `getDimensionalIfPresent(dimension)`.
    pub fn dimensional_if_present(&self, dimension: u32) -> Option<&DimensionalCarriage> {
        self.dimensions.get(&dimension)
    }

    fn shape(&self) -> CarriageShape {
        CarriageShape {
            leading_half_spacing: self.leading_bogey().ty.wheel_point_spacing / 2.0,
            trailing_half_spacing: self.trailing_bogey().ty.wheel_point_spacing / 2.0,
            bogey_spacing: self.bogey_spacing,
            on_two_bogeys: self.is_on_two_bogeys(),
        }
    }

    /// `isOnIncompatibleTrack()`: either end sits on an edge of a track type its bogey doesn't run on.
    pub fn is_on_incompatible_track(&self, graph: &TrackGraph, track_type_of: &dyn Fn(Option<&str>) -> String) -> bool {
        let check = |b: &CarriageBogey, p: &TravellingPoint| {
            let Some(edge) = p.edge(graph) else { return false };
            b.ty.is_on_incompatible_track(Some(&track_type_of(edge.material.as_deref())))
        };
        check(self.leading_bogey(), self.leading_point()) || check(self.trailing_bogey(), self.trailing_point())
    }

    /// `getAnchorDiff()`: how far apart the entity's anchors are, the spacing when unknown.
    pub fn anchor_diff(&self, graph: &TrackGraph) -> f64 {
        let spacing = f64::from(self.bogey_spacing);
        let two = self.is_on_two_bogeys();
        if let (Some(a), Some(b)) = (node_dimension(graph, self.leading_point().node1), node_dimension(graph, self.trailing_point().node1)) {
            if a != b {
                return spacing;
            }
        }
        let mut diff = 0.0;
        let mut entries = 0;
        for dce in self.dimensions.values() {
            if let (Some(l), Some(t)) = (dce.leading_anchor(two), dce.trailing_anchor(two)) {
                entries += 1;
                diff += distance_to(l, t);
            }
        }
        if entries == 0 { spacing } else { diff / f64::from(entries) }
    }

    /// `travel(level, graph, distance, toFollowForward, toFollowBackward, type)`: moves every wheel
    /// by `distance` and returns how far the last one got. The wheels of a carriage follow each
    /// other; at the ends they follow `to_follow_*` (the next carriage's wheel) or the navigation.
    pub fn travel(
        &mut self,
        graph: &TrackGraph,
        distance: f64,
        to_follow_forward: Option<TravellingPoint>,
        to_follow_backward: Option<TravellingPoint>,
        kind: CarriageKind,
        ctx: &mut TravelCtx,
    ) -> f64 {
        let on_two = self.is_on_two_bogeys();
        let stress = if ctx.derailed || !on_two { 0.0 } else { f64::from(self.bogey_spacing) - self.anchor_diff(graph) };
        self.blocked = false;

        let mut distance_moved = distance;
        let iterate_from_back = distance < 0.0;

        for first_bogey in [true, false] {
            if !first_bogey && !on_two {
                continue;
            }
            let actually_first_bogey = !on_two || (first_bogey ^ iterate_from_back);
            let bogey_correction = stress * if actually_first_bogey { 0.5 } else { -0.5 };
            let bogey_stress = {
                let b = if actually_first_bogey { &self.bogeys.0 } else { self.bogeys.1.as_ref().unwrap() };
                b.stress(graph, ctx.derailed)
            };

            for first_wheel in [true, false] {
                let actually_first_wheel = first_wheel ^ iterate_from_back;
                let wheel = if actually_first_wheel { 0 } else { 1 };

                // The wheels to follow, as they are now.
                let prev_point: Option<TravellingPoint> = if !actually_first_wheel {
                    Some(self.pick_bogey(actually_first_bogey).points[0].clone())
                } else if !actually_first_bogey && on_two {
                    Some(self.bogeys.0.points[1].clone())
                } else {
                    None
                };
                let next_point: Option<TravellingPoint> = if actually_first_wheel {
                    Some(self.pick_bogey(actually_first_bogey).points[1].clone())
                } else if actually_first_bogey && on_two {
                    Some(self.bogeys.1.as_ref().unwrap().points[0].clone())
                } else {
                    None
                };

                let correction = bogey_stress * if actually_first_wheel { 0.5 } else { -0.5 };
                let mut to_move = distance_moved;

                let front_pick = match prev_point {
                    None => match &to_follow_forward {
                        None => Pick::Control,
                        Some(f) => Pick::Follow(f.clone()),
                    },
                    Some(p) => Pick::Follow(p),
                };
                let back_pick = match next_point {
                    None => match &to_follow_backward {
                        None => Pick::Control,
                        Some(f) => Pick::Follow(f.clone()),
                    },
                    Some(p) => Pick::Follow(p),
                };

                let at_front = matches!(kind, CarriageKind::First | CarriageKind::Both) && actually_first_wheel && actually_first_bogey;
                let at_back = matches!(kind, CarriageKind::Last | CarriageKind::Both) && !actually_first_wheel && (!actually_first_bogey || !on_two);

                to_move += correction + bogey_correction;

                let pick = if to_move > 0.0 { front_pick } else { back_pick };
                // Which listener: forward moves report the front at the front wheel, the back at the
                // back wheel; backward moves the other way round. None otherwise (`ignoreEdgePoints`).
                #[derive(Clone, Copy)]
                enum Which {
                    Front,
                    Back,
                    Passive,
                }
                let which = if to_move > 0.0 {
                    if at_front { Which::Front } else if at_back { Which::Back } else { Which::Passive }
                } else if at_front {
                    Which::Back
                } else if at_back {
                    Which::Front
                } else {
                    Which::Passive
                };

                let Carriage { bogeys, dimensions, .. } = self;
                let bogey = if actually_first_bogey { &mut bogeys.0 } else { bogeys.1.as_mut().unwrap() };
                let point = &mut bogey.points[wheel];

                let TravelCtx { control, front, back, double_portal, .. } = &mut *ctx;
                let mut selector = match pick {
                    Pick::Control => TrackSelector::Custom(&mut **control),
                    Pick::Follow(other) => TrackSelector::follow(&other),
                };

                let moved;
                {
                    let mut portal = |c: [TrackNodeLocation; 2]| -> bool {
                        for dce in dimensions.values() {
                            if let Some(pivot) = dce.pivot {
                                if c.iter().any(|n| n.equals_ignore_dim(&pivot)) {
                                    return false;
                                }
                            }
                        }
                        if dimensions.len() > 1 {
                            (*double_portal)();
                            return true;
                        }
                        false
                    };
                    let mut passive = |_: f64, _: &EdgePointHit| false;
                    let mut listeners = Listeners {
                        edge_point: Some(match which {
                            Which::Front => &mut **front,
                            Which::Back => &mut **back,
                            Which::Passive => &mut passive,
                        }),
                        turn: None,
                        portal: Some(&mut portal),
                    };
                    moved = point.travel(graph, to_move, &mut selector, &mut listeners);
                }

                self.blocked |= self.pick_bogey(actually_first_bogey).points[wheel].blocked;
                distance_moved = moved;
            }
        }

        self.update_contraption_anchors(graph);
        self.update_traversed_dimensions(graph);
        distance_moved
    }

    fn pick_bogey(&self, first: bool) -> &CarriageBogey {
        if first { &self.bogeys.0 } else { self.bogeys.1.as_ref().unwrap_or(&self.bogeys.0) }
    }

    /// The start of `manageEntities`: the dimensions the wheels are in. What the glue does with
    /// them (create, align and discard entities) is its own.
    pub fn update_traversed_dimensions(&mut self, graph: &TrackGraph) {
        let mut dims: Vec<u32> = Vec::new();
        for b in [Some(&self.bogeys.0), self.bogeys.1.as_ref()].into_iter().flatten() {
            for p in &b.points {
                if let Some(d) = node_dimension(graph, p.node1) {
                    if !dims.contains(&d) {
                        dims.push(d);
                    }
                }
            }
        }
        self.traversed_dimensions = dims;
    }

    /// `updateContraptionAnchors()`.
    pub fn update_contraption_anchors(&mut self, graph: &TrackGraph) {
        let leading_bogey = self.bogeys.0.clone();
        if leading_bogey.points.iter().any(|t| t.edge(graph).is_none()) {
            return;
        }
        let trailing_bogey = self.trailing_bogey().clone();
        if trailing_bogey.points.iter().any(|t| t.edge(graph).is_none()) {
            return;
        }
        let two = self.is_on_two_bogeys();
        let leading_dim = leading_bogey.dimension(graph);
        let trailing_dim = trailing_bogey.dimension(graph);
        let leading_wheel_spacing = leading_bogey.ty.wheel_point_spacing;
        let trailing_wheel_spacing = trailing_bogey.ty.wheel_point_spacing;
        let leading_ud = leading_bogey.is_upside_down();
        let trailing_ud = trailing_bogey.is_upside_down();
        let bogey_spacing = self.bogey_spacing;
        let spacing = f64::from(bogey_spacing);
        let shape = self.shape();
        let leading_point = self.leading_point().clone();
        let trailing_point = self.trailing_point().clone();

        for leading in [true, false] {
            let (point, other) = if leading { (&leading_point, &trailing_point) } else { (&trailing_point, &leading_point) };
            let Some(dimension) = node_dimension(graph, point.node1) else { continue };
            let Some(other_dimension) = node_dimension(graph, other.node1) else { continue };

            if dimension == other_dimension && leading {
                self.dimensional(dimension).discard_pivot(bogey_spacing);
                continue;
            }

            let mut dce = self.dimensions.remove(&dimension).unwrap_or_default();

            let pivoted = |dce: &mut DimensionalCarriage, offset: f64| -> Option<Vec3> {
                pivoted(graph, dce, dimension, point, leading, &leading_point, &trailing_point, spacing, offset, leading_ud, trailing_ud)
            };

            let leading_offset = if leading { leading_wheel_spacing / 2.0 } else { spacing + trailing_wheel_spacing / 2.0 };
            dce.position_anchor = if Some(dimension) == leading_dim {
                leading_bogey.anchor_position(graph, false)
            } else {
                pivoted(&mut dce, leading_offset)
            };

            let back_anchor_flip = trailing_ud ^ leading_ud;

            if two {
                dce.rotation_anchors[0] = if Some(dimension) == leading_dim {
                    leading_bogey.anchor_position(graph, false)
                } else {
                    pivoted(&mut dce, leading_offset)
                };
                dce.rotation_anchors[1] = if Some(dimension) == trailing_dim {
                    trailing_bogey.anchor_position(graph, back_anchor_flip)
                } else {
                    pivoted(&mut dce, if leading { leading_wheel_spacing / 2.0 + spacing } else { trailing_wheel_spacing / 2.0 })
                };
            } else if dimension == other_dimension {
                dce.rotation_anchors = [
                    leading_bogey.points[0].get_position(Some(graph), false),
                    leading_bogey.points[1].get_position(Some(graph), false),
                ];
            } else {
                dce.rotation_anchors[0] = if leading { point.get_position(Some(graph), false) } else { pivoted(&mut dce, leading_wheel_spacing) };
                dce.rotation_anchors[1] = if !leading { point.get_position(Some(graph), false) } else { pivoted(&mut dce, leading_wheel_spacing) };
            }

            let prevmin = dce.min_allowed_local_coord(bogey_spacing);
            let prevmax = dce.max_allowed_local_coord(bogey_spacing);
            dce.update_cutoff(leading, &shape);
            if prevmin != dce.min_allowed_local_coord(bogey_spacing) || prevmax != dce.max_allowed_local_coord(bogey_spacing) {
                dce.cutoff_changed = true;
            }
            self.dimensions.insert(dimension, dce);
        }
    }
}

/// `pivoted(dce, dimension, start, offset, leadingUpsideDown, trailingUpsideDown)` with `findPivot`.
#[allow(clippy::too_many_arguments)]
fn pivoted(
    graph: &TrackGraph,
    dce: &mut DimensionalCarriage,
    dimension: u32,
    start: &TravellingPoint,
    start_is_leading: bool,
    leading_point: &TravellingPoint,
    trailing_point: &TravellingPoint,
    bogey_spacing: f64,
    offset: f64,
    leading_upside_down: bool,
    trailing_upside_down: bool,
) -> Option<Vec3> {
    let pivot = match dce.pivot {
        Some(p) => Some(p),
        None => {
            let end = if start_is_leading { trailing_point } else { leading_point };
            let found = find_pivot(graph, start, end, bogey_spacing, start_is_leading, dimension);
            dce.pivot = found;
            found
        }
    }?;
    let flipped = !start_is_leading && (leading_upside_down != trailing_upside_down);
    let start_vec = start.get_position(Some(graph), flipped)?;
    let portal_vec = add(pivot.location(), [0.0, if leading_upside_down { -1.0 } else { 1.0 }, 0.0]);
    Some(lerp((offset / distance_to(start_vec, portal_vec)) as f32, start_vec, portal_vec))
}

/// `findPivot(dimension, leading)`: scouts from `start`, following `end`, `bogeySpacing + 10` blocks
/// the other way, for the portal into `dimension`.
pub fn find_pivot(
    graph: &TrackGraph,
    start: &TravellingPoint,
    end: &TravellingPoint,
    bogey_spacing: f64,
    leading: bool,
    dimension: u32,
) -> Option<TrackNodeLocation> {
    let mut scout = TravellingPoint { node1: start.node1, node2: start.node2, position: start.position, blocked: false, upside_down: false };
    let mut selector = TrackSelector::follow(end);
    let distance = bogey_spacing + 10.0;
    let direction = if leading { -1.0 } else { 1.0 };
    let mut pivot: Option<TrackNodeLocation> = None;
    {
        let mut portal = |nodes: [TrackNodeLocation; 2]| -> bool {
            for n in nodes {
                if n.dimension == dimension {
                    pivot = Some(n);
                }
            }
            true
        };
        let mut listeners = Listeners { edge_point: None, turn: None, portal: Some(&mut portal) };
        scout.travel(graph, direction * distance, &mut selector, &mut listeners);
    }
    pivot
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::network::RailNetworks;
    use super::super::network::tests::line;
    use super::super::travelling_point::SteerDirection;
    use super::super::travelling_point::steer_select;

    fn carriage(nets: &RailNetworks, g: u128, p: [f64; 4], spacing: i32, two: bool) -> Carriage {
        let ids: Vec<NodeId> = nets.graphs[&g].nodes().map(|n| n.net_id).collect();
        let (a, b) = (ids[0], ids[1]);
        // Points sit on the first edge, measured from node `a`.
        let pt = |x: f64| TravellingPoint::new(a, b, x, false);
        let b1 = CarriageBogey::new(BogeyType::small(), false, vec![], pt(p[0]), pt(p[1]), 0);
        let b2 = if two { Some(CarriageBogey::new(BogeyType::small(), false, vec![], pt(p[2]), pt(p[3]), 0)) } else { None };
        Carriage::new(1, b1, b2, spacing)
    }

    fn run(c: &mut Carriage, graph: &TrackGraph, d: f64, kind: CarriageKind) -> f64 {
        let mut control = |g: &TrackGraph, tp: &TravellingPoint, _f: bool, cands: &[Candidate]| steer_select(g, tp, SteerDirection::None, [0.0, 1.0, 0.0], cands);
        let mut front = |_: f64, _: &EdgePointHit| false;
        let mut back = |_: f64, _: &EdgePointHit| false;
        let mut dp = || {};
        let mut ctx = TravelCtx { derailed: false, control: &mut control, front: &mut front, back: &mut back, double_portal: &mut dp };
        c.travel(graph, d, None, None, kind, &mut ctx)
    }

    #[test]
    fn a_one_bogey_carriage_moves_its_wheels_together() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 40.0]);
        let mut c = carriage(&nets, g, [11.0, 9.0, 0.0, 0.0], 0, false);
        let moved = run(&mut c, &nets.graphs[&g], 3.0, CarriageKind::Both);
        assert!((moved - 3.0).abs() < 1e-4);
        assert!((c.leading_point().position - 14.0).abs() < 1e-4, "lead {} trail {}", c.leading_point().position, c.trailing_point().position);
        assert!((c.trailing_point().position - 12.0).abs() < 1e-4);
        assert!(!c.blocked);
        assert!(c.dimensions.contains_key(&0));
    }

    #[test]
    fn reaching_the_end_of_the_track_blocks() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 40.0]);
        let mut c = carriage(&nets, g, [38.0, 36.0, 0.0, 0.0], 0, false);
        let moved = run(&mut c, &nets.graphs[&g], 10.0, CarriageKind::Both);
        assert!(c.blocked);
        assert!(moved < 10.0);
    }

    #[test]
    fn two_bogeys_keep_their_spacing_going_either_way() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 100.0]);
        // The bogeys' centres are 6 apart.
        let mut c = carriage(&nets, g, [21.0, 19.0, 15.0, 13.0], 6, true);
        let graph = nets.graphs[&g].clone();
        let d = run(&mut c, &graph, 5.0, CarriageKind::Both);
        assert!((d - 5.0).abs() < 1e-4);
        let lead = c.bogeys.0.points[0].position;
        let trail = c.bogeys.1.as_ref().unwrap().points[1].position;
        assert!((lead - 26.0).abs() < 1e-4 && (trail - 18.0).abs() < 1e-4);
        let d = run(&mut c, &graph, -3.0, CarriageKind::Both);
        assert!((d + 3.0).abs() < 1e-4);
    }

    #[test]
    fn cutoff_limits() {
        let mut d = DimensionalCarriage::default();
        assert_eq!(d.min_allowed_local_coord(3), i32::MIN);
        assert_eq!(d.max_allowed_local_coord(3), i32::MAX);
        d.cutoff = 1.0;
        assert_eq!(d.min_allowed_local_coord(3), i32::MAX);
        d.cutoff = 0.5;
        assert_eq!(d.min_allowed_local_coord(3), (-3.0f64 - 1.0 + 5.0 * 0.5).floor() as i32);
        d.cutoff = -0.5;
        assert_eq!(d.max_allowed_local_coord(3), (-3.0f64 - 1.0 + 5.0 * 0.5).ceil() as i32);
        d.cutoff = -1.0;
        assert_eq!(d.max_allowed_local_coord(3), i32::MIN);
    }

    #[test]
    fn kinds_and_cargo() {
        assert_eq!(CarriageKind::of(0, 1), CarriageKind::Both);
        assert_eq!(CarriageKind::of(0, 3), CarriageKind::First);
        assert_eq!(CarriageKind::of(1, 3), CarriageKind::Middle);
        assert_eq!(CarriageKind::of(2, 3), CarriageKind::Last);
        let mut s = CargoState::default();
        s.tick_idle_cargo_tracker();
        assert_eq!(s.ticks_since_last_exchange, 1);
        s.change_detected();
        assert_eq!((s.version, s.ticks_since_last_exchange), (1, 0));
    }

    #[test]
    fn stress_and_anchor() {
        let mut nets = RailNetworks::new();
        let g = line(&mut nets, &[0.0, 40.0]);
        let c = carriage(&nets, g, [11.0, 9.0, 0.0, 0.0], 0, false);
        let graph = &nets.graphs[&g];
        assert!(c.leading_bogey().stress(graph, false).abs() < 1e-4);
        assert!(c.leading_bogey().anchor_position(graph, false).is_some());
        assert_eq!(c.leading_bogey().dimension(graph), Some(0));
    }
}

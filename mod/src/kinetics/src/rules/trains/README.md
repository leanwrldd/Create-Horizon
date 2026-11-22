# Trains, parts 1 and 2: the rules foundation, stations, navigation, schedules

Pure rules (no game types) for Create 6.0.10's train system, ported from
`create-java-src/src/main/java/com/simibubi/create/content/trains/`. Tests sit beside each file.
Part 1 is the track side; part 2 (the last three rows) is stations, navigation and schedules.
Registered through `tools/gen_modules.py`, which now also lists a rules folder with a `mod.rs`.

## Ported

| File | Java source | What |
|---|---|---|
| `vec.rs` | `Vec3`, `Mth`, Catnip `VecHelper`, `AngleHelper.deg` | vector maths, `bezier`, `bezierDerivative`, `intersect` (by plane), `slerp`, `intersectRanged` |
| `track_shape.rs` | `track/TrackShape.java` | shapes, axes, normals, mirror, rotate, portals |
| `bezier.rs` | `track/BezierConnection.java` | handles, length, step table, `getPosition`, `incrementT`, `getNormal`, bounds, segment iterator, `rasterise`, `secondary`, item costs |
| `placement.rs` | `track/TrackPlacement.java` (`tryConnect`) | curve validity: min radius, angles, S curves, slopes, extents, every error key |
| `graph.rs` | `graph/TrackNodeLocation`, `TrackNode`, `TrackEdge`, `EdgeData`, `TrackEdgeIntersection`, `EdgePointType`, `EdgePointStorage`, `signal/TrackEdgePoint` | node rounding, edge length/direction/`canTravelTo`/position/crossings, edge point lists |
| `network.rs` | `graph/TrackGraph`, `graph/EdgePointManager`, the track half of `GlobalRailwayManager` | add/remove nodes and edges, crossings, join (`transferAll`) and split (`findDisconnectedGraphs`), points |
| `propagator.rs` | `track/TrackPropagator`, `ITrackBlock` (discovery) | placing/breaking track updates the graphs; node validity |
| `travelling_point.rs` | `entity/TravellingPoint` | stepping along edges, junction selectors (random, follow, steer), edge point/turn/portal listeners |
| `signal.rs` | `signal/SignalBoundary`, `SignalEdgeGroup`, `EdgeGroupColor`, `SignalPropagator` | signal blocks (group propagation), occupancy, colours, signal state ticks incl. cross signals |
| `station.rs` | `station/GlobalStation`, `station/StationBlockEntity` (assembly), `infrastructure/config/CTrains` | the station edge point (name, assembling, nearest train, `reserveFor`, `getPresentTrain`, `getImminentTrain`, `canApproachFrom`); assembly scan (`refreshAssemblyInfo`), `isValidBogeyOffset`, assembly direction, wheel point offsets and placement on the graph, every `train_assembly.*` error key |
| `navigation.rs` | `entity/Navigation`, `graph/DiscoveredPath`, `Train.Penalties`/`getNavigationPenalty` | Dijkstra `search` with Create's penalties (red signals, stations, other trains) and octile estimate, `findPathTo`, `findNearestApproachable`, `startNavigation`, `cancelNavigation`, `navigateOptions`, and `tick` (signal scouting with cross signal chains, reservation, braking and curve speeds) |
| `schedule.rs` | `schedule/Schedule`, `ScheduleEntry`, `ScheduleRuntime`, `condition/*`, `destination/*` | the 5 instructions and 9 wait conditions as data with their evaluation and `start`, `ScheduleRuntime` (state machine, cooldown, predictions) |

Design: Java object references become ids (`NodeId`, edge = `(from, to)`, point = `(type, uuid)`).
`RailNetworks` is `GlobalRailwayManager`'s graphs + signal edge groups; whatever Java does to trains
and to the sync layer is pushed as a `RailEvent` for later code to drain. Ordered maps replace Java's
hash maps, so "any" becomes "lowest"; UUIDs come from a counter in `RailNetworks`.

Part 2 design: the train is not in the crate. `station.rs` takes a `TrainStates` (does the train exist, its
station, whether it navigates, its distance), `navigation.rs` a `NavTrain` (carriage points, conductors,
speed, status) plus `OtherTrain`s for the penalties, `schedule.rs` a `ScheduleTrain` and a `ConditionWorld`.
The later `Train` implements them. A station is `PointData::Station` in `graph.rs`.
`Navigation::tick` takes the graph out of `RailNetworks` for the tick (so signal groups can change while the
scouting point reads it) and puts it back. `JavaPriorityQueue` reproduces `java.util.PriorityQueue` so ties in
the frontier break as in Java.

## Not in the Java tree / unverified

- Catnip (`VecHelper`) is not in `create-java-src`. `bezier`, `bezierDerivative`, `intersect`, `slerp`
  follow the existing `create-horizon-ola` port and the calls Create makes. `intersect_ranged` is
  reconstructed from how `TrackEdge.getIntersection` uses it; check it when Catnip is available.
- `Mth.atan2` is a lookup-table approximation in Minecraft; `f64::atan2` is used.
- Java quirk kept: `SignalPropagator.onSignalRemoved` discards the result of `sidesToUpdate.map`.
- Part 2: station and address globs use `rules/package.rs`'s `Glob`, itself reconstructed from Catnip's `Glob`.
- Java quirk kept: in `search`, a station found while expanding an edge is reported twice, and the second
  report (when the edge is polled) passes `penalty` alone as the cost, so `DiscoveredPath.cost` excludes the
  distance. `findPathTo` overwrites its result with the second.
- Where Java walks a `HashMap` (a node's connections, other trains, chained groups) the order is by node/group id.
- `ScheduleRuntime.destinationReached` indexes `predictionTicks` without a bounds check in Java; here an index
  out of range is skipped.
- The `JavaPriorityQueue` tie order was written from `java.util.PriorityQueue`'s `siftUp`/`siftDown`; the tests
  check the order of keys, not of ties.

## Left out of part 1

NBT/packet reading and writing, `DimensionPalette`, `TrackGraphBounds`, `TrackGraphVisualizer`,
`TrackGraphSync`/`TrackGraphHelper`, render transforms (`SegmentAngles`, `GirderAngles`), tilt
(`TrackBlockEntityTilt`, `yOffsetAt`), the placement inventory/pavement bookkeeping
(`placeTracks`, `paveTracks`), `TrackBlock`/`ITrackBlock` block-state specifics (the glue implements
`TrackWorld`), `AllPortalTracks`.

## Left out of part 2

NBT/packet reading and writing everywhere (`GlobalStation`, `Navigation`, `Schedule*`, `StationEditPacket`),
`GlobalStation.runMailTransfer` and the package ports' offline buffers (items), the contraption half of
`StationBlockEntity.assemble` (carriages, `CarriageContraption`, `Train`), `TrackTargetingBehaviour`,
`TrackObserver`, schedule screens/titles/summaries/icons/waiting status texts, `ScheduleItem`,
`DestinationSuggestions`, hats.

## Next

1. Carriages and trains: `Train` (implements `TrainStates`, `NavTrain`, `ScheduleTrain`), `Carriage`, `CarriageBogey`, `CarriageContraption`, couplings, `TrainStatus`, `GlobalRailwayManager.tick` (trains' signal occupancy: `group.trains`, `reserved`; `Train.tick` around `Navigation.tick`), the rest of `StationBlockEntity.assemble`.
2. Bogeys: `AbstractBogeyBlock`, `BogeyStyle`, sizes (`getValidPathfindingTypes`, `getWheelPointSpacing`).
3. `TrackTargetingBehaviour` (placing edge points on the graph) and observers (`TrackObserver`).
4. Glue: `TrackWorld` implementation from real track blocks, drain `RailNetworks::events`, content (blocks, items, recipes), saved data, `ScheduleItem`.

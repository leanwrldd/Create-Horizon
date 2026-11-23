# Trains, parts 1 to 3: the rules foundation, stations, navigation, schedules, the train itself

Pure rules (no game types) for Create 6.0.10's train system, ported from
`create-java-src/src/main/java/com/simibubi/create/content/trains/`. Tests sit beside each file.
Part 1 is the track side; part 2 (the next three rows) is stations, navigation and schedules; part 3 (the last
rows) is the train: carriages, ticking, the manager's train half, assembly, targeting and observers.
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
| `bogey.rs` | `bogey/BogeySizes`, `AbstractBogeyBlock`, `StandardBogeyBlock` | sizes (small 6.5/16, large 12.5/16), wheel point spacing 2, radius, track type, upside down, single-bogey rule |
| `carriage.rs` | `entity/Carriage`, `CarriageBogey`, `TrainCargoManager` (counters) | `travel` (wheels follow each other, stress between bogeys, front/back listeners), anchors per dimension (`updateContraptionAnchors`, pivots, cutoff), `getAnchorDiff`, bogey stress/dimension, cargo version and idle ticks, fuel slots |
| `train.rs` | `entity/Train`, `TrainStatus`, `TrainMigration` | `Train` implementing `NavTrain` and (via `ScheduleCtx`) `ScheduleTrain`; `tick` (passive slowdown, derailed slowdown, stalls, carriage stress and speed, blocked / high stress / derail, `updateNavigationTarget`), `earlyTick`, signal listeners (`occupy`, reserve, observers, arrival at a station), `collectInitiallyOccupiedSignalBlocks`, collisions (`findCollidingTrain`, `crash`), detach/reattach (`TrainMigration`), `canDisassemble`, disassembly placement, `burnFuel`, `maxSpeed`/`acceleration` from the config, `TrainStatus` messages |
| `railways.rs` | `GlobalRailwayManager` (train half), `TrackGraph` train hooks, `SignalPropagator.notifyTrains` | trains list, moving/waiting lists and tick order, group occupancy reset each tick, invalid trains dropped, graph events applied to trains (node removed, node/graph transferred, signal blocks stale), `disassemble_train` |
| `assembly.rs` | `StationBlockEntity.assemble` (second half), `enterAssemblyMode`, `tick` | carriages and `Train` from scanned bogeys and the glue's contraption results (spacing, two bogeys per carriage, `single_bogey_carriage`, `not_connected_in_order`, `no_controls`, `nothing_attached`), `add_assembled_train`, re-navigation when a station enters assembly, a station's train presence |
| `targeting.rs` | `track/TrackTargetingBehaviour` | `createEdgePoint` from a graph location: rotated-block direction flip, coexistence, merging a second signal, point placement for signals/stations/observers, `destroy` |
| `observer.rs` | `observer/TrackObserver`, `SingleBlockEntityEdgePoint` | activation (8 on keep-alive, counted down in both tick phases), filter, current train; `PointData::Observer` in `graph.rs` |

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

Part 3 design: `Train` owns its `Navigation` and `ScheduleRuntime` but takes them out of itself for the length of
a call into them (`with_navigation`, `with_runtime`); whatever they ask of the train that Java answers from the other
one (`runtime.paused`, `transitInterrupted`) is a copy or a queued flag, applied when the call returns. `ScheduleCtx` is
the `ScheduleTrain` (train + navigation + networks + the other trains + a `TrainWorld`). `TrainWorld` is what the
train asks of the game (day time, redstone, loaded dimensions, package ports, passengers, conductors, item filter
matching); `TrainEvent` is what it tells the game (explosions, crashes, mail transfers, entity management, fuel items
used, removed trains). While the wheels move the graph is out of the networks so the signal groups can change under
the listeners; the arrival at a station is finished (`runtime.destinationReached`, mail transfer) right after the
carriage that arrived, and a collision is applied (both trains crash) after the graph is back, ending the tick as
Java's `if (graph == null) return` does. The first carriage ticked in speed order is the one checked for collisions,
as in Java.

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

- Part 3: `VecHelper.intersectSphere` (Catnip, used by `TrainMigration` and the collision test's fallback) is
  reconstructed (far intersection of the line with the sphere); check it when Catnip is available.
- Part 3: `Carriage.travel`'s portal listener only has the per-dimension pivots to compare with; portals between
  dimensions (`findPivot`, `pivoted`, cutoffs) are ported but not exercised by tests.
- Java quirk kept: `Train.forEachTravellingPointBackwards` reuses `trailingBogey.leading()` for "inside 2nd bogey";
  `TrackObserver.tick` ignores `preTrains` and so counts down twice a game tick (kept alive: 4 ticks).
- Part 3: deviations are noted in `train.rs`'s header; the order of Java's `HashMap` walks (trains, graphs for
  migration) is by id here.
- Part 3: `TrackGraph.removeNode`'s train detach needs the removed edges (the networks' `TrainsOnNodeRemoved`
  event now carries them); `TrainMigration` is built from those.
- Part 3: the cargo (items, fluids, fuel, packages) is plain data the glue keeps current in `CargoState`; filters are
  matched by `TrainWorld::item_matches`, `FilterItemStack`'s real matching is the glue's.
- Part 3 tests run trains on straight lines; curves, junction selection by the navigation and signals on the track
  are covered by the earlier parts' tests, not repeated with a whole train.

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

## Left out of part 3

NBT/packets for trains and carriages, `Carriage` entity handling (`manageEntities` beyond reporting the dimensions,
passengers, `alignEntity`, `createEntity`), `CarriageBogey.updateAngles`/`updateCouplingAnchor` (rendering), honking and
steam, advancements except the red-signal event, `CarriageContraption` (blocks, actors, storage, seats), bogey styles,
the station block's states and `StationEditPacket`, `TrackGraphHelper.getGraphLocationAt` (reads track blocks),
`TrackTargetingBehaviour`'s rendering and `transform`'s structure transform, `RailwaySavedData`, `TrainRelocator`,
`GlobalTrainDisplayData`.

## Next

1. Glue: `TrackWorld` implementation from real track blocks, `TrackGraphHelper` (graph location from a block and
   axis), track blocks and placement, drain `RailNetworks::events` into `Railways::apply_rail_events` and the sync layer.
2. Glue: station and signal blocks (block entities calling `TargetingState::create_edge_point`), observer block,
   `StationBlockEntity` (assembly: `scan_assembly`, `place_travelling_points`, `assemble_train`, `add_assembled_train`;
   disassembly: `Railways::disassemble_train`), schedule item and screens.
3. Glue: the train entity (`CarriageContraptionEntity`) drawn through block entities, driven by `Carriage` anchors and
   `TrainEvent::ManageEntities`; controls (`manual_tick`, `manual_steer`, `speed`), conductors via
   `TrainWorld::check_conductors`, passengers.
4. Content: blocks, items, recipes, bogey styles, saved data for `Railways`.

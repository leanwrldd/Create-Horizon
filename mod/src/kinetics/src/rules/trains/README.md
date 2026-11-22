# Trains, part 1: the rules foundation

Pure rules (no game types) for Create 6.0.10's train system, ported from
`create-java-src/src/main/java/com/simibubi/create/content/trains/`. Tests sit beside each file.
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

Design: Java object references become ids (`NodeId`, edge = `(from, to)`, point = `(type, uuid)`).
`RailNetworks` is `GlobalRailwayManager`'s graphs + signal edge groups; whatever Java does to trains
and to the sync layer is pushed as a `RailEvent` for later code to drain. Ordered maps replace Java's
hash maps, so "any" becomes "lowest"; UUIDs come from a counter in `RailNetworks`.

## Not in the Java tree / unverified

- Catnip (`VecHelper`) is not in `create-java-src`. `bezier`, `bezierDerivative`, `intersect`, `slerp`
  follow the existing `create-horizon-ola` port and the calls Create makes. `intersect_ranged` is
  reconstructed from how `TrackEdge.getIntersection` uses it; check it when Catnip is available.
- `Mth.atan2` is a lookup-table approximation in Minecraft; `f64::atan2` is used.
- Java quirk kept: `SignalPropagator.onSignalRemoved` discards the result of `sidesToUpdate.map`.

## Left out of part 1

NBT/packet reading and writing, `DimensionPalette`, `TrackGraphBounds`, `TrackGraphVisualizer`,
`TrackGraphSync`/`TrackGraphHelper`, render transforms (`SegmentAngles`, `GirderAngles`), tilt
(`TrackBlockEntityTilt`, `yOffsetAt`), the placement inventory/pavement bookkeeping
(`placeTracks`, `paveTracks`), `TrackBlock`/`ITrackBlock` block-state specifics (the glue implements
`TrackWorld`), `AllPortalTracks`.

## Next

1. Stations: `GlobalStation`, `StationBlockEntity` rules (assembly, `StationEditPacket`), `TrackTargetingBehaviour` (placing edge points on the graph), observers (`TrackObserver`).
2. Navigation / pathfinding: `Navigation` (Dijkstra over the graph, signal waiting), `DiscoveredPath`.
3. Schedules: `Schedule`, `ScheduleRuntime`, conditions and instructions.
4. Carriages and trains: `Train`, `Carriage`, `CarriageBogey`, `CarriageContraption`, couplings, `TrainStatus`, `GlobalRailwayManager.tick`, train collision with signals' occupancy (`group.trains`, `reserved`).
5. Bogeys: `AbstractBogeyBlock`, `BogeyStyle`, sizes.
6. Glue: `TrackWorld` implementation from real track blocks, drain `RailNetworks::events`, content (blocks, items, recipes), saved data.

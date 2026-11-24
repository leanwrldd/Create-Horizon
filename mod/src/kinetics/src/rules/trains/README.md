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
| `graph_helper.rs` | `graph/TrackGraphHelper`, `track/TrackTargetingBlockItem.withGraphLocation`, `OverlapResult` | the graph location of a track block and a direction (centre on a node; between two nodes, walking to them), of a point on a curve, and the overlap check of a new edge point |
| `edge_blocks.rs` | `signal/SignalBlock`, `SignalBlockEntity`, `observer/TrackObserverBlock(Entity)`, `station/StationBlock(Entity)`, `StationEditPacket`, `track/TrackTargetingBlockItem`, `AbstractBogeyBlock` | signal lights (`enterState`, the 15-tick hold off red, the power report, the type follow, the 4-tick power release), observer power, station edits and `/station`, the train presence picture, the assembly area, the bogey click (`trackClicked`), the targeting item's reach, the English of the messages |
| `schedule_nbt.rs` | `schedule/Schedule`, `ScheduleEntry`, `ScheduleInstruction`, `ScheduleWaitCondition` (`write`, `fromTag`, each one's `data` keys) | a schedule as an NBT tree (`create:train_schedule`) |
| `schedule_edit.rs` | `schedule/ScheduleScreen` (its edits), `ScheduleItem.appendHoverText` | `/schedule`: the stops and conditions as commands, a new stop's defaults, summaries and the tooltip |
| `carriage_pose.rs` | `entity/Carriage.java` (`DimensionalCarriageEntity.alignEntity`), `CarriageContraptionEntityRenderer`, `OrientedContraptionEntity.applyLocalTransforms`, `CarriageBogey.updateAngles`, `bogey/StandardBogeyRenderer` | a carriage's yaw and pitch from its rotation anchors, the draw operations of the carriage and of each bogey, the bogey angles and wheel turn, the standard bogeys' pieces, and the pose news a client runs on (`Motion`) |
| `carriage_contraption.rs` | `entity/CarriageContraption.java`, `AbstractBogeyBlock.getStickySurfaces` (in `contraption::search`) | what a carriage's blocks are: bogeys, the second bogey, Train Controls facing, conductor seats, `assemble`'s verdict and errors |
| `train_nbt.rs` | `Train`, `Carriage`, `CarriageBogey`, `TravellingPoint`, `Navigation`, `ScheduleRuntime` (`write` / `read`), `TrackNodeLocation.write` | a train as an NBT tree and back against the rebuilt graph |
| `train_controls.rs` | `CarriageContraptionEntity.startControlling` / `control`, `ControlsServerHandler`, `ControlsHandler`, `ControlsUtil`, `TrainStatus` | driving by hand: the held keys with their 30-tick keepalive, `control` (target speed, steering, approaching a station, the prompts), the train status lines |

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

## Glue part 1: the track (merged with this folder's rules; not played)

Content `create:track` (`tools/content/track.py`: `shape` and `turn` as Create's blockstate names them, shapes clipped to the
cell, the sequenced assembly recipe, the tags), the rules `track_block.rs` (`TrackBlock`, `ITrackBlock` defaults,
`TrackBlockItem.select`, `TrackPlacement.placeTracks`, messages) and `track_render.rs` (`TrackRenderer.getModelAngles`,
`BezierConnection.SegmentAngles`), and the glue in `mod/src/machines/`: `track.rs` (the block, `Connections` in the block
entity as Create saves them, `TrackWorld` over the host, one `RailNetworks` per dimension in a static, queued
add/remove at the level tick, the break hook, a sweep for vanished blocks, the level attachment `create:track_network`
that keeps a world token and the track positions: the graphs are rebuilt from those on load), `track_item.rs` (the two-click
curve flow, the selection as the component `create:track_connecting_from` plus the glint, written at the inventory tick)
and `track_draw.rs` (the curve's ties and rails with the `tie`, `segment_left` and `segment_right` partial models, kept).

Not ported in this part: `TrackBlockEntityTilt` (slope smoothing, `yOffsetAt` is 0), portal tracks, the girder on a curve,
pavement, the sprint-maximised turn, the placement preview and overlay, `FakeTrackBlock` (the map marker), wrench pick-up
of a curve's items, `getRequiredItems`, `CurvedTrackInteraction`/`TrackBlockOutline` (selecting a curve with the cursor),
saving the graphs themselves (only the positions are saved), `TrackGraphSync` to clients, `getBogeyAnchor`.
A block put down by `setblock` or a structure is not in the graph until the sweep or a neighbouring change finds it; there
is no hook for `onPlace`.

## Glue part 2: stations, signals, observers, bogeys, the schedule item (not played)

Content (`tools/content/`): `track_station.py`, `track_signal.py`, `track_observer.py`, `bogey.py` (the two bogeys, with the
Train Casing and its item application) and `schedule.py`; Create's datagen recipes, loot and lang. Art comes from the player's
jar (blockstates, `block_entry_signal`..., the indicator partials). The glue, in `mod/src/machines/`: `train_net.rs` (the track
questions the rules ask, `TrackTargetingBehaviour`'s data and its edge point, the sweep that removes the point of a lost block
entity, the tick of the graphs' points), `track_target.rs` (the item: pick a spot on the track, place the block), `track_station.rs`,
`track_signal.rs`, `track_observer.rs` (the cargo filter is a value box: `Slot::TrackObserver` and its row) and `schedule.rs`.
Each block entity ensures its point in `RailNetworks` every tick and keeps the saved form (`Id`, `TargetTrack`, `TargetDirection`,
the station's `StationName`), so the points come back after a load although the graphs aren't saved.

Differences from Create, and what's missing (the module headers have the details):
- No station, assembly or schedule screens: `/station` and `/schedule` stand in, and a right click prints the state.
- **Assembly stopped at the track in part 2**: the scan, the bogey clicks, the errors and the wheel points on the graph; part 3 (below)
  gathers the carriages and makes the train, with disassembly, the station's train presence and the auto-schedule slot.
- A point on a curve can't be selected (client curve picking); the signal and observer overlays on the track and the station
  flag/assembly outlines aren't drawn; the bogeys' wheels aren't drawn; no door controls, display sources, map markers,
  computer behaviour, package ports.
- The graphs' points ticked in `train_net.rs` in part 2; `Railways::tick` owns them since part 3 (`trains.rs`).
- The track-clicks of an assembly area are `use_item_on` on the track block (`TrackBlock.useItemOn`).

## Glue part 3: trains exist and run (not played)

The glue, in `mod/src/machines/`: `trains.rs` (the railways of each dimension ticked every level tick: `Railways::tick`, its
`TrainEvent`s applied, each carriage put where its wheels are; the trains saved in the level attachment `create:trains`
and read back once their track is back; `/train`), `train_assemble.rs` (the second half of `StationBlockEntity.assemble`
and `Train.disassemble`), `train_draw.rs` (the carriage anchor block entity and its drawing), `train_controls.rs` (the keys
of a driving player) and the pieces in `track.rs` (the `Railways` kept beside the graphs, `dimension_name`),
`track_station.rs` (assembling, the train presence's comparator output, the auto-schedule slot), `schedule.rs`,
`structure_click.rs` (a click on a carriage's Train Controls) and `solid/` (a carriage is a solid structure).
Content (`tools/content/train_controls.py`): the Train Controls (`create:controls`) and the carriage anchor block.

- **Assembling** (`/station assemble`, or the station's assemble step): each carriage is what `Contraption.searchMovedStructure`
  finds from its bogey (`rules::contraption::search`, the bogey's sticky sides added); `assemble_train` checks it; the blocks leave
  the world as a pistons' structure does (`capture`, `remove_from_world`) and are kept in the carriage's runtime; the `Train` joins
  the dimension's `Railways` standing at the station (`add_assembled_train`). A failed assembly changes nothing.
- **Disassembling** (`/station disassemble`): `Railways::disassemble_train` (every carriage level and square to the track), each carriage set
  down by `place_in_world` turned by `-yaw + initialYaw`; the station remembers the name for the next assembly; the schedule goes back
  to the player or the station (`dropSchedule`).
- **Running**: `Railways::tick` from `level_tick_post`; `train_net.rs` no longer ticks the points. `updateContraptionAnchors` and the
  pose (`alignEntity`) are worked out each tick from the wheels. A crash's explosion is a flash and a sound (Horizon has no `explode`).
  Status lines (`TrainStatus`) go to the owner in chat.
- **Drawing** (why a block entity in an invisible block): Horizon has no entity a mod can draw (a general entity type, declared with
  `declare_general_entity`, has a record, a payload and a tick, and no renderer), so each carriage has an anchor
  (`create:carriage_anchor`: no shape, no item, an empty model) that `trains.rs` keeps in an air cell within six blocks of the carriage
  and moves when the carriage gets far from it. The anchor's block entity holds the structure and the pose; its draw hook turns and moves
  what it draws each frame (`translate` and `rotate_about`, never `keep()`, as a moving piston is drawn) and widens its render box. The
  other routes were worse: a block entity at a fixed place (the station, the track under it) can't be drawn once the train is further
  than the view of block entities, and a display entity per block is per viewer, unsaved and a packet per block. The pose goes out as
  news (position, velocity, yaw and pitch and their rates, the game time) every other tick while the carriage moves and once when it
  stops, and the client runs it on for at most six ticks. The update tag is the whole block entity data, structure included (a mod can't tell
  a chunk being sent from a block change), about a kilobyte per carriage per news.
- **Bogeys** are drawn from their wheel angle with `StandardBogeyRenderer`'s pieces (two shafts, the frame and wheels; the large one with its drive,
  belt, piston and pin), under `translateBogey`'s operations; the bogey blocks themselves are part of the structure.
- **Solid**: a carriage's anchor is a controller of `solid/` (`Sample::Oriented`, `Rotation::Oriented` of the collider): entities are pushed,
  carried and held up by it, and the local player's mixin collides with it. Clicks on a carriage's blocks work through the structure click
  (`structure_click.rs`: levers, doors, deployers, storage): a click on the Train Controls starts a drive. A click on a seat seats the
  player: the carriage's tick moves the rider's marker with the seat (`trains::carry_riders`, the same `StructurePose` the solid module
  uses; seats keyed by `trains::seat_key`, not the moving anchor), and disassembly puts riders back on the seat blocks it places.
- **Controls**: a click on the Train Controls of a carriage, or `/train drive` near it, starts driving; the client takes the movement keys and
  sends the six keys (`create:train_controls`, a keepalive every 5 ticks), the server holds each key 30 ticks (`HeldKeys`) and runs
  `train_controls::control` each tick; sneaking stops it. Prompts (`Arrived at`, `Departing from`, the approach bar) are the action bar.
- **Schedules**: `/train schedule` gives the held Schedule to the nearest train, a Schedule right-clicked into the station (empty hand while
  sneaking takes it out) is the auto-schedule slot and starts `ScheduleRuntime` on a train that arrives (`applyAutoSchedule`);
  `/train conductor` says a driver is aboard (Create's seated villager or blaze burner: no mob can sit on a seat here).

Differences and gaps (what is *not* Create):
- No carriage entities: seats on a carriage hold no passenger the rules know, no mob conductors (`/train conductor` stands in), no
  `countPlayerPassengers`, no sounds, steam, honking or particles, no couplings between carriages drawn.
- The cargo is not kept (`TrainCargoManager`): fuel isn't burnt from chests, cargo conditions see nothing, no mail transfer.
- The anchors' light is the anchor cell's; the bogeys' belt texture doesn't scroll; the Controls' lever and cover are still.
- A train in a chunk nobody loads still runs (Create stalls a train a player rides into unloaded chunks); trains don't cross dimensions.
- After a load a train waits (60 ticks, then every second) for the graph its wheels name and for the stations' and signals' points; a train
  whose track never comes back stays as the saved anchors draw it. The graphs are still rebuilt from the track blocks.
- The first full run of trains in game has not been done: see HANDOFF.md for what to check.

## Next

1. Play-test glue part 3 (HANDOFF.md lists what to watch).
2. Train doors (`TrainDoorBlock`, `DoorControlBehaviour`), the station's door controls, display sources (`StationSummary`, `TrainStatus`
   sources, `TrainDisplay`), map markers (`StationMarker`, `StationMapData`), portals (`AllPortalTracks`, the carriage cutoffs), the
   station screens, `TrainRelocator`, `GlobalTrainDisplayData`.
3. The track overlays of edge points, the station's flag and assembly outlines, and edge points on curved track (picking a point on a
   curve with the cursor), the girder and tilt of a curve.
4. Cargo: `TrainCargoManager` over the carriages' chests, barrels and vaults (fuel, cargo conditions), passengers and seated conductors,
   `ScheduleItemEntityInteraction`, the Blaze Burner conductor hat.
5. One-shot update fields for the pose (`set_client_update_fields`) so only the pose travels, once the compat layer exposes them.

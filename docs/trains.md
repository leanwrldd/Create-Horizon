# Trains, parts 1 to 5: the rules foundation, stations, navigation, schedules, the train itself, the display data, the relocator, cargo, passengers, doors and what the client draws

Pure rules (no game types) for Create 6.0.10's train system, ported from
`create-java-src/src/main/java/com/simibubi/create/content/trains/`. Tests sit beside each file.
Part 1 is the track side; part 2 (the next three rows) is stations, navigation and schedules; part 3 (the last
rows) is the train: carriages, ticking, the manager's train half, assembly, targeting and observers. Part 4 (the last
four rows) is what rides in it: cargo, seated passengers and conductors, handing over a Schedule, and the doors.
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
| `carriage_contraption.rs` | `entity/CarriageContraption.java`, `AbstractBogeyBlock.getStickySurfaces` (in `contraption::search`) | what a carriage's blocks are: bogeys, the second bogey, Train Controls facing, conductor seats, the Blaze Burners that conduct (`blockConductors`), `assemble`'s verdict and errors |
| `train_nbt.rs` | `Train`, `Carriage`, `CarriageBogey`, `TravellingPoint`, `Navigation`, `ScheduleRuntime` (`write` / `read`), `TrackNodeLocation.write` | a train as an NBT tree and back against the rebuilt graph |
| `train_controls.rs` | `CarriageContraptionEntity.startControlling` / `control`, `ControlsServerHandler`, `ControlsHandler`, `ControlsUtil`, `TrainStatus` | driving by hand: the held keys with their 30-tick keepalive, `control` (target speed, steering, approaching a station, the prompts), the train status lines |
| `display.rs` | `display/GlobalTrainDisplayData`, `displayLink/source/{StationSummary,TrainStatus}DisplaySource`, `ObservedTrainNameSource`, `ScheduleRuntime.getWaitingStatus` and each condition's `getWaitingStatus` | the departure board's table (`refresh` from every train's `submitPredictions`, `prepare` by glob filter and sort), the station summary's rows and flap layout, the train status line (only while the train waits at the station), the observed train's name, the English of every waiting status |
| `relocator.rs` | `entity/TrainRelocator`, `TrainRelocationPacket` | `relocate` (the probe walking the wheels back from the clicked spot, blocked by the end of the track, a portal or another train; then the train takes its places, forgets its signals and destination, and stops), the session's checks and messages |
| `train_cargo.rs` | `TrainCargoManager`, `MountedStorageManager` (`initialize`, `canUseForFuel`, `isExposed`), `Train.burnFuel` (the container item) | what a carriage's chests, barrels, shulker boxes, vaults and tanks hold, sorted into the items, fluids and fuel slots the schedule conditions and `burnFuel` read; the version / idle ticker change detection; taking one fuel item out (and putting its bucket back) |
| `train_passengers.rs` | `CarriageContraptionEntity.checkConductors`, `Train.countPlayerPassengers`, `SeatBlock.canBePickedUp`, `SeatInteractionBehaviour.handleEntityCollision`, `AbstractContraptionEntity.addSittingPassenger` | the conductor sides from the burners and the seated mobs, the players seated, who a seat picks up, who gives way to whom, whether a mob touches a seat |
| `schedule_hand.rs` | `ScheduleItem.handScheduleTo`, `ScheduleItemEntityInteraction`, `ConductorBlockInteractionBehavior`, `EntityHats.shouldRenderTrainHat`, `BlazeBurnerMovementBehaviour.shouldRenderHat`, `BlazeBurnerRenderer.renderShared` (the hat) | the decisions of handing a Schedule to a seated conductor or a Blaze Burner (apply, deny, resume, take back), their messages, and when the Train Hat is worn and where it sits on a burner |
| `mail.rs` | `GlobalStation.runMailTransfer`, `GlobalPackagePort`, `StationBlockEntity.attachPackagePort`, `PackagePortPlacementPacket`, `PackagePortTargetSelectionHandler.validateDiff` | packages between carriages and a station's Postboxes (import what isn't for the port, export what matches, offline buffer and `primed`, a Creative Crate voiding), the range checks |
| `portal_track.rs` | `AllPortalTracks`, `PortalTrackProvider`, `TrackBlock.connectToPortal` / `updateShape` / `getConnected` (bound half), `BlockUtil.getLargestRectangleAround`, `PortalShape.getRelativePosition` | portal integrations, the exit of a portal track (vanilla destination rebuilt from Horizon's port of it), the link plan with its failures and English, the survival check, the bound ends that join two dimensions |
| `portal_cutoff.rs` | `CarriageContraption` (`isHiddenInPortal`, `atSeam`, `withinVisible`), `CarriageContraptionEntity.isLocalCoordWithin`, `DimensionalCarriageEntity.updatePassengerLoadout` / `dismountPlayer` | what of a carriage a portal hides, the seam slice, which passengers sit down or are put off, where a player goes |
| `train_doors.rs` | `DoorControl`, `DoorControlBehaviour`, `SlidingDoorMovementBehaviour`, `DoorMovingInteraction`, `SlidingDoorBlock.isDoubleDoor`, `StationEditPacket` (door control) | the six door modes (matching, saved name, English), the five Train Doors, `getDoorFacing`, `shouldOpen` / `shouldUpdate` / `tickOpen` / `toggleDoor`, the click on a door in a carriage; `edge_blocks.rs` carries the station edit and `/station doors` |
| `track_overlay.rs` | `TrackBlock.prepareTrackOverlay` / `prepareAssemblyOverlay`, `TrackTargetingBehaviour.render`, `TrackTargetingClient.render`, `StationRenderer` (the flag, the assembly spots), `StationBlockEntity.flag`, `TrackBlockOutline.pickCurves` / `drawCurveSelection`, `TrainRelocator.relocateClient` (the preview lines), `CurvedTrackSelectionPacket` | a pose stack as a chain of operations (`Chain`, `Affine`), the edge point marker's pose on a straight block or a curve segment, the assembly spots and their colours, the flag's lerp, model choice, wiggle and angle, the cursor's ray against a curve's segments and the outline of the one it hits, the preview's lines, and the wire formats of the client's hover, the server's preview and a click on a curve |
| `train_hat.rs` | `CreateHatArmorLayer.render`, `TrainHatInfo`, `EntityHats` | the operations that lay the Train Hat on a head (`translateAndRotate` to the head, the head cube's top, the hat's scale, flip, sink and tilt), for the humanoid, villager and illager heads |

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

## Left out of part 4

- **Cargo**: the Depot, Toolbox, Creative Crate and fallback (other mods') storages (`MountedItemStorageType`s other than the
  chest / simple / vault types), the Creative Fluid Tank, `SyncedMountedStorage` packets to clients, the storage's contents
  changing by anything but fuel burning and clicks (no Portable Storage Interface on trains, no Packager at a station), mail
  transfer (`runMailTransfer`). `changeDetected` is a diff of the contents every 20 ticks, not a call on each move.
- **Passengers**: seating by lead (`SeatBlock.getLeashed`), seating when a mob walks onto a seat in the world, the
  `#c:capturing_not_supported` tag (NeoForge's, not in the data), `seatHostileMobs` (on), `TamableAnimal` sitting, a rider's
  dismount place (`ContraptionDismountLocation`), the seat mapping packet (`ContraptionSeatMappingPacket`).
- **Conductor hat**: drawn on mobs since part 5 (below); a carriage's Blaze Burner head isn't drawn in the structure at all (its
  `renderInContraption`), so there is still no hat to put on it. `TrainHat` in a burner's data isn't read either.
- **Doors**: `SlidingDoorBlockEntity`'s animation and `VISIBLE` (the doors here swing), the elevator contact's door control
  (`ElevatorContactBlockEntity.doorControls`) and `getElevatorDoorControl`, the double-door neighbour of a platform's sliding door,
  the station screen's door widget (`DoorControl.createWidget`: `/station doors` stands in), `ContraptionDisableActorPacket`.

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
`TrackTargetingBehaviour`'s rendering and `transform`'s structure transform, `RailwaySavedData`.

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
- (Part 5 drew the overlays, the flag and the assembly spots, and made a point on a curve selectable.) The bogeys' wheels aren't
  drawn here; no door controls, display sources, map markers, computer behaviour, package ports (see parts 4 and 4b for what came later).
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
- No carriage entities: no sounds, steam, honking or particles, no couplings between carriages drawn (passengers, conductors and
  cargo are part 4, below; `/train conductor` is still there as a stand-in driver).
- No mail transfer (the cargo itself is part 4).
- The anchors' light is the anchor cell's; the bogeys' belt texture doesn't scroll; the Controls' lever and cover are still.
- A train in a chunk nobody loads still runs (Create stalls a train a player rides into unloaded chunks); trains don't cross dimensions.
- After a load a train waits (60 ticks, then every second) for the graph its wheels name and for the stations' and signals' points; a train
  whose track never comes back stays as the saved anchors draw it. The graphs are still rebuilt from the track blocks.
- The first full run of trains in game has not been done: see HANDOFF.md for what to check.

## Glue part 4: what the trains tell the world, and putting a train back (not played)

Rules in `display.rs` and `relocator.rs`; the glue, in `mod/src/machines/`: `train_display.rs` (the table of predictions refreshed
every 100 ticks per dimension, the stations that ask their Display Links to write, the three readers), `train_relocate.rs` (the
Wrench session), and the changes in `display_link.rs`, `display_link_source.rs`, `flap_display.rs` and `track_observer.rs`.

**Display sources** (`AllDisplaySources`: `station_summary`, `train_status` on `create:track_station`, `observed_train_name` on
`create:track_observer`): `rules::display_link::Source` has three more variants, found by `sources_for`, with the saved ids
`create:station_summary`, `create:train_status` and `create:observed_train_name`.
- **Station summary**: a flap-display-only source (`provideText` is `EMPTY`, so a sign or lectern gets one empty line). Its rows are
  `display::station_summary_rows` (minutes, unit, train, title and, when the filter has a `*`, the platform) and its layout
  `station_summary_layout` (`StationSummary<name column><has platform><platform column>`), which needed the board's width: `load_flap_layout`
  now takes `x_size`. The filter starts as the station's name (`populateData`, taken when the source is chosen) and is set with a
  **name tag**; the two column scroll inputs (`NameColumn` 0 to 73, `PlatformColumn` 0 to 16) are one option the Wrench steps through
  (`display::LAYOUTS`: 50/3, 25/3, 73/3, 50/0, 50/6, 0/3).
- **Train status**: the waiting status of the train standing at the station (`getPresentTrain`), empty unless it has a running schedule
  waiting at a stop that isn't the last of a non-cyclic one. "Players Seated" shows 0 passengers (no carriage entities).
- **Observed train name**: the train the observer's edge point is activated by (`TrackObserver.getCurrentTrain`), refreshed every 400
  ticks (and when the observer's power changes: `track_observer.rs` now calls `display_link::notify_gatherers`, `DisplayLinkBlock.notifyGatherers`).
- A station whose train's runtime has `displayLinkUpdateRequested` writes its links the same tick (`StationBlockEntity.tick`).

**GlobalTrainDisplayData**: `refresh` runs for every train with an unpaused schedule that isn't derailed and has a graph, through a
`ScheduleCtx` on the train itself. One table per dimension's `Railways` (Create has one for the manager); the HashMap's order between
predictions that tie is the destination's order here.

**TrainRelocator**: `/train relocate` with a Wrench in hand starts it on the nearest train, and so does a Wrench click on a carriage that no
block of it takes (part 5: `structure_click.rs` -> `train_relocate::carriage_wrenched`); the next Wrench click on a piece of track puts the
train there (`getNearestTrackAxis` from the look, `getGraphLocationAt`), a sneak-click or putting the Wrench away ends it. A failed click says
"Cannot relocate Train to here". The session ends when the train moves (`cannot_relocate_moving`). A derailed train's graph is none, so
(as in Create) only other derailed trains count as in its way. Part 5 added the preview and clicking a curve (below).


**Map markers** (`StationMarker`, `StationMapData`, `MapItemSavedDataMixin`, `StationMapDecorationRenderer`) are **not ported**. Horizon does
have map decorations: the client hook `map_decorations` (`MapInfo` with the map's id, centre, scale and dimension, and a `MapDecoration` of a
sprite, half-pixel position and rotation, drawn every frame), `map_decoration_types` (a registered type is drawn from an item's
`minecraft:map_decorations` entries) and `level_map_data` (read only). What it can't do: (1) a decoration has no text, and Create draws the
station's name under the icon; (2) "nothing is saved in the map", where Create toggles the marker in the shared `MapItemSavedData` (every copy
of the map sees it) and prunes it when the station changes (`checkStations`), so the markers would live in a mod attachment keyed by the map id
and be sent to every client, or in each stack's component (per copy); (3) the icon is `assets/create/textures/map/decorations/station.png`
of the jar, which would have to be imported at build time as an atlas sprite whose name couldn't be checked without the game. The maths
(`addStationMarker`'s 63 pixel window and `(int)(local * 2 + 0.5)`, `toggleStation`) is small once those are decided.

## Glue part 4b: cargo, passengers, conductors, doors (not played)

Glue in `mod/src/machines/`: `train_cargo.rs`, `train_riders.rs`, `train_doors.rs`, and small additions to `trains.rs` (the tick, the
world's questions, the queries the others ask), `seat.rs` (mobs on a carriage's seats), `structure_click.rs` (the Train Door and the
conductor's burner), `track_station.rs` (the station's door control) and `burner.rs` (`furnace_burn_time`, `fuel_remainder`).

- **Cargo**: each carriage's `CargoState` is made to match its structure's storage blocks (`portable_interface_storage`'s reading of
  `Items` and `TankContent`) every 20 ticks and after a burn. `TrainEvent::FuelConsumed` takes one item out of the structure's fuel slot,
  puts a bucket back for a lava bucket, and writes the structure to the anchor so what clicks changed isn't lost. Burn times come from
  the item's `minecraft:cooking_fuel` component (or the short vanilla list `burner.rs` has).
- **Passengers**: a click on a carriage's seat seats the player (the old guard that kept carriage seats empty is gone). Every 5 ticks a
  living mob touching a carriage's seat sits on it (`seat.rs`'s marker and `/ride`, with the mob's entity id), unless the seat has
  someone; a player replaces a mob. `trains::build_world` answers `checkConductors` and `countPlayerPassengers` from the riders, so
  a mob on a seat in front of Train Controls (or a lit Blaze Burner there) lets the train run its schedule without `/train conductor`
  (still there, as a stand-in).
- **Handing over a Schedule**: right-click a seated mob with a Schedule (main hand): the train takes it when the seat is in front of
  controls (else `Conductor needs to sit in front of a Controls block`); an empty hand takes it back, or resumes a paused train. Clicking
  a Blaze Burner in front of the controls (on a carriage) does the same.
- **Doors**: the five sliding doors (`andesite`, `brass`, `copper`, `train`, `framed_glass`) in a carriage open when the train
  stands at a station whose door control allows that side (`/station doors <all|north|east|south|west|none>` on the station last clicked;
  the mode is the station's `DoorControl`), and close when it leaves; the door of a platform in front of them is opened too. A click on a door
  in a carriage toggles it (and its double-door neighbour).

To check in game: seat a villager (or any mob) on a carriage's seat next to Train Controls by walking it onto the seat or pushing it,
give it a Schedule, watch the train run; put coal in a chest of a carriage and watch the fuel go (`/train info` speed with and without
fuel: `poweredTrainTopSpeed`); a Schedule with a "cargo threshold" condition; train doors opening at a station, only on the chosen
side; a Blaze Burner next to Train Controls (assemble first) taking and giving back a Schedule; a player sitting on a carriage seat counting
for a "players seated" condition.

## Glue part 5: what the client draws (not played)

Rules in `track_overlay.rs` and `train_hat.rs` (and `relocator::preview`); glue in `mod/src/machines/`: `train_overlay.rs` (the client's half of
the edge point items, the curve under the cursor and the relocation preview), `train_hat.rs` (the hat layer), and changes in `track_target.rs`,
`train_relocate.rs`, `structure_click.rs`, `track_station.rs`, `trains.rs`, `seat.rs`, `track.rs` and `lib.rs` (the new API's client hooks).

- **Held item**: with a Station, Signal or Observer in the main hand, the track under the cursor (or the spot the item already picked, or a curve
  segment) shows the overlay model (`block/track_overlay/{station,signal,observer}`) at `1 + 1/16` scale, as `TrackTargetingClient.render`
  does: `client_tick` works out the spot, checks it with `withGraphLocation` where the client can read the graphs and keeps the pose,
  `submit_custom_geometry` draws it as a model instance (`WorldDrawContext::draw_model_instances`, the model resolved with the other standalone models).
  `Affine` and `Chain` are a pose stack as a matrix and as the operations that made it: the matrix goes to the instance, the operations are
  replayed by a block entity's draw (`translate`, `scale`, `rotate_about`).
- **Placed**: each block entity of the three draws the overlay at its target track in its own pose (`TrackTargetingBehaviour.render`), with
  `Overlay` `SKIP` / `DUAL` for signals. The station raises its flag (`StationRenderer.renderFlag`: `flag_on`, `flag_off`, `flag_assemble`, the
  lift squared with the wiggle, the `LerpedFloat` that chases a tenth a tick) and, while it assembles with no train, draws a spot for each
  place a bogey may go (blue) or is (green). The station's block entity now says `TrainPresent` (synced when it changes); the client scans the
  assembly itself (`track_station::refresh`, every second). The spots are four boxes with the model's own UVs and texture, tinted, because a
  model can't be tinted all over; their texture is imported by naming `block/track_overlay/assembling` in `track.py`'s `MODELS`.
- **Curves**: every track block entity that is drawn tells `train_overlay.rs` its curves (`TRACKS_WITH_TURNS`); each client tick, while an edge point
  item is held or a relocation runs, the eye's ray is tried against each curve's segments (`pick_curves`, the same boxes and order as
  `pickCurves`, rotated into each segment's frame). The segment under the cursor is outlined (`drawCurveSelection`, black at `.4` alpha) and a right
  click on it, in either click hook, is cancelled on the client and sent as `create:curve_click` (what `CurvedTrackSelectionPacket` and the relocation
  packet carry: the block, the curve's key, the segment and the side). The server (`track_target.rs` `select_curve`; `train_relocate.rs`) answers in the
  sender's own turn. A selection on a curve is the component's `bezier` entry (`TRACK_TARGETING_ITEM_BEZIER`); `item_data` already wrote
  `Bezier` into a placed block and `ensure_point` places the point with `bezier_graph_location_at`.
- **Relocation**: a Wrench click on a carriage that no block of it took starts a session (`handleSpecialInteractions`: the one edit of `structure_click.rs`).
  While it runs the client sends the track under the cursor and the look (`create:relocation_hover`, when it changes and every 10 ticks); the
  server walks the wheels there (`relocator::preview`, the same walk as `relocate(simulate)` with the places recorded, up to the first stretch that
  fails) and answers (`create:relocation_preview`) with the places and whether the train fits. The client draws the line at `-.925` below the
  places, `0x95CD41` and the last stretch `0xEA5C2B` when it can't be, alternate pieces thinner; the server says "Can relocate to here, Click to
  Confirm" (green) or "Cannot relocate Train to here" (red) on the action bar. A curve is clicked the same way as for the edge points.
- **The Train Hat**: Horizon draws on a mob through entity layers (`register_entity_layers`, `EntityRenderLayer::submit`: the entity's frame, a
  humanoid's head pose in `PosedParts`). A layer has no host, so `train_hat.rs` has the server tell each player which entity ids wear the hat
  (`create:train_hats`, from `player_tick`: `trains::hat_wearers` over `seat::riders_with_ids`, `schedule_hand::wears_train_hat`). The pose is
  `CreateHatArmorLayer`'s chain; the head is the humanoid model's `head` part (zombies, skeletons, players) or, for villagers and illagers, the
  model's origin turned by the head's yaw and pitch.

Differences and what is left out:
- The graphs aren't on clients: the held overlay is checked against them only where the client can read the server's (single-player, a LAN
  host); elsewhere it shows on any track. The relocation preview is the server's, so it works everywhere.
- The overlay instance's light is the track block's with a floor of 4; the flag's `LerpedFloat` runs on the frame's clock; a tilted track's shift and
  the girder under a curve aren't drawn (neither is ported). `TrackBlockOutline`'s outline of straight track under the cursor isn't drawn.
- **The hat on a mob needs the mob's head**: Create reads it off the live model parts (`TrainHatInfo`); a layer sees only a humanoid's, so
  only the zombie family, skeletons, villagers, the trader, the illagers and players get one, with head cubes taken from the vanilla models (not read
  from Horizon's) and the villagers' renderer scale of `0.9375`. Mobs with a `train_hat_info` file (a pig, a witch, a piglin ...) go without. The
  helmet check (`getHatFor` returns nothing for a worn item) and `TrainHat` in a mob's data aren't read.
- **A carriage's Blaze Burner hat isn't drawn**: the burner's head isn't drawn in the structure at all (`BlazeBurnerRenderer.renderInContraption`),
  so a hat on it has nothing to sit on. `burner_wears_hat` and `hat_placement` in `schedule_hand.rs` are ready for it.
- To check in game: hold a Station, Signal and Observer over straight and curved track (overlay, side flips with the look, sneak-click clears, a
  second click places with the point on the curve), a placed signal in `DUAL` and `SKIP`, the flag rising with a train in and while assembling
  (and the spots, blue and green, where bogeys go), then a Wrench on a carriage: the line follows the cursor, green or orange, the message,
  a click on a straight piece and on a curve, sneak-click aborts; a villager on a conductor seat of a carriage with a schedule wearing the hat, the
  hat following its head, a player in that seat, and no hat on a plain seat; and no panic in `latest.log` (a hook that panics every frame costs seconds).

## Glue part 4c: more cargo, mail, lead seating, elevator doors, portals (not played)

- **Cargo**: a Depot (its slot 0), a Toolbox (`Inventory.Items`, 32 slots) and a Creative Crate (two slots; a burn leaves it whole and
  the fuel's bucket is voided) on a carriage are cargo and fuel (`train_cargo::cargo_slots` / `items_path` / `is_bottomless`;
  `portable_interface_storage::items_of_with` / `apply_items_with`, which keep a Depot's output slots). The Portable Storage Interface
  still sees only chests, barrels, shulker boxes and vaults. Other mods' inventories (`FallbackMountedStorageType`) have no copy.
- **Mail** (`machines/train_mail.rs`, `postbox.rs`): Create has it (`runMailTransfer`). Only a Postbox links to a station
  (`TrainStationFrogportTarget.canSupport`); a Frogport links to a chain. Right-click a Track Station with a Postbox in hand to pick it,
  then place the Postbox within 5 blocks: its `Target` is `{type: "create:train_station", relative_pos}`. The Postbox's tick registers it
  every 10 ticks (`lazyTick`), a Name Tag re-registers it (`filterChanged`). `TrainEvent::MailTransfer` now runs the transfer; the
  station's records and the offline buffers are saved in the station block entity as `Ports`; `port_packages` (Fetch Packages) is
  answered. Differences: no selection outline, particles are a command, no ComputerCraft events.
- **Seating by lead**: `seat::leashed_by` (`SeatBlock.getLeashed`: a mob on the player's lead within 10 blocks, using
  `entity_leash_holder`) seats the mob on a block seat. Wired for a carriage's seat in glue part 4d.
- **Elevator doors**: an Elevator Contact has a door control (`DoorControl`, saved as the constant's name); `/elevator doors <mode>` sets it for
  the contact last right-clicked. A cabin's sliding doors open when the cabin has arrived (or is slow and not held) by the control of the
  contact at the column's target floor (`train_doors::tick_elevator`, `should_open_in_elevator`).
- **Portals**: rules only. What the API allows: `entity_teleport_to_level` moves players, mobs, items and vehicles (with passengers) between
  levels and `entity_travel_to_dimension` / `entity_portal` are hooks; blocks of another level read through `host.level(dim)` when it is loaded.
  What it can't: read the portal link (`Portal.getPortalDestination`, the points of interest), build a portal, or keep an entity in two levels
  (a carriage here is an anchor block entity per dimension, which `carriage.rs` already models). So `portal_track.rs` rebuilds the destination
  from the portal blocks the glue lists, without creating a portal. **Not glued**, because it needs a redesign: one set of graphs and trains per
  dimension (`track.rs`, `trains.rs`) must become one shared set (Create has one manager), `HostTracks` must read blocks of another dimension, the
  track block needs `connectToPortal` on its tick, `BoundLocation` / `BoundDimension` in its block entity and the `bound_ends` in `getConnected`,
  anchors need to be kept in both dimensions, and the cutoff (`portal_cutoff.rs`) applied to what is drawn and to the carriage's collision.

## Glue part 4d: lead seating on a carriage seat, the Train Hat on burners and on mobs from `train_hat_info` (not played)

- **Lead seating** (`SeatInteractionBehaviour`'s caller, `AbstractContraptionEntity.handlePlayerInteraction`): `structure_click.rs`'s `sit` gets the
  clicking player's entity id from the compat layer's `server_players()` view (`player_view`: `PlayerRef::entity().id()`), refuses a player who
  rides something, and calls `seat::leashed_by` (`SeatBlock.getLeashed`); a mob on the lead sits instead through the new
  `seat::sit_led_on_structure` (same body as `sit_mob_on_structure`, but the click ejects a mob already on the seat, as `toDismount.stopRiding()`
  does; a player on the seat still refuses). Without a led mob the player sits as before.
- **The Blaze Burner's hat** (`BlazeBurnerRenderer.renderShared` / `renderInContraption`, `BlazeBurnerMovementBehaviour.shouldRenderHat`): the burner's
  head, hat and rods are now one function (`burner::draw_head`), used by the block entity and by `burner::draw_in_structure`, which every moving
  structure's draw (`contraption::draw_blocks`) and a carriage's (`train_draw::draw_structure`) calls, so burners on bearings, pistons and carriages have
  their heads (they had only the cage). A carriage's conducting burners wear the hat while the train has a schedule: `carriage_contraption::hat_burners`
  (pure) is worked out every 20 ticks in `trains::place_carriages` and written as `Hats` (local positions) into the anchor's data, which goes out
  whole when it changes. The hat is lifted `hat_placement` (0.75; in a structure the head is never the small one) and turned half a turn past
  the head. A static burner wears it when its data has `TrainHat` (Create's `BlazeBurnerBlockEntity.hat`, which only Ponder sets); the update tag carries it.
  Differences: the head faces the block's `facing` (Create turns it to the camera, or along the train's motion for a conductor), no goggles.
- **The hat on any mob with `train_hat_info`**: the 58 entries of `VanillaHatOffsetGenerator` are `rules::trains::train_hat_info::INFO` (Create writes them as
  JSON files, so there is no data file here), with `info_for` (`TrainHatInfoReloadListener.getHatInfoFor`, minus the snow golem's pumpkin entry
  `SNOW_GOLEM_PUMPKIN`, which needs entity data a layer lacks). A layer can't read the renderer's model parts as `CreateHatArmorLayer` does, so
  `train_hat::client_setup` (`FMLClientSetupEvent`) bakes each type's vanilla layer with `ClientHost::bake_layer("minecraft:cow#main")` (and
  `_baby#main`), turns it into a `ModelNode` tree and `resolve` finds the chain of parts to the head, the head's box and the offset; `mount_ops`
  is `CreateHatArmorLayer.render`'s transform chain. 26.3's models carry the renderer's scale and lift in their root (a cat's root is 0.8 scaled), so
  the chain starts at the root. A part path is looked for from the root, then below any part of that name (the bee's `body`, the wolf's
  `real_head`); equines use `head_parts`. Types with no such part (pufferfish and tropical fish: one model per variant) get no hat. Piglins take Horizon's
  head pose; other models turn their `head` part with the entity's head angles. Boxes are read off the faces (six to a box), so a grown box
  reads slightly bigger than Create's. Not read: `train_hat_info` files of resource packs and other mods (`bake_layer` is fine, but there's no way
  to list a pack's folder), the head slot, and a model replaced by a resource pack after start-up.
- **Fixed on the way**: `train_hat::hat_ops` turned a head pitch-first (x, y, z) where `rotationZYX` is `Rz * Ry * Rx`; a head both pitched and turned sat wrong.
  `rotation_ops` is the one place now.
- **Not changed**: the existing zombie, skeleton, villager and illager mounts still multiply a baby's `age_scale` (0.5); 26.3 has separate baby layers for
  zombies, villagers and others (`zombie_baby#main`), so that may shrink the hat twice. To check in game with a baby zombie conductor.
- **One-shot pose fields (`set_client_update_fields`)**: the new API has it (`ModBlockEntityView::set_client_update_fields`), and the compat layer's
  `ModBlockEntity` doesn't carry it yet, but exposing it would not make only the pose travel. The fields go *over* what `block_entity_update_tag`
  returns, in the packet a block change sends, and the hook that builds the tag can't tell a change from a chunk send, so the anchor's tag must still
  hold the whole structure; it's sent alone only when the hook returns nothing, and then a chunk send carries nothing either. What is missing is
  either a way for the tag hook to know it is building a change, or a sync call that sends fields only. A redesign that sends the structure by message when
  a player first sees the anchor (as the pose news already goes) and returns no tag could use it: not done, because it changes how every carriage appears.
  The pose news message (`POSE_CHANNEL`) keeps the per-tick path small meanwhile.

To check in game: sit a leashed villager on a carriage seat (and a second one on an occupied mob seat, and a player seat which should refuse);
a Blaze Burner in front of Train Controls with a schedule on the train (hat on, off when the schedule goes); a burner on a bearing (head shown);
a conductor cow, pig, wolf, horse, cat, bee, ghast (hat on the head, sized by the head); a baby of each.

## Next

1. Play-test glue parts 3, 4, 4b, 4c and 5 (HANDOFF.md lists what to watch).
2. `TrainDisplay`, map markers (see glue part 4), gluing portals (see glue part 4c), the station screens.
3. The girder and tilt of a curve (`GirderAngles`, `TrackBlockEntityTilt`, `yOffsetAt`), `TrackBlockOutline`'s selection box for straight
   track, `CurvedTrackInteraction`'s break progress and the Wrench's pick-up of a curve.
4. Hats: the snow golem's pumpkin entry, `train_hat_info` files of resource packs, the burner's goggles and its head turning to the camera or along
   the train, the baby scale of the older mounts (see glue part 4d).
5. One-shot update fields for the pose: needs a tag hook that knows it builds a change, or a fields-only sync (see glue part 4d).


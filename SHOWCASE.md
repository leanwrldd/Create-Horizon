# Create for Horizon

Create rewritten from scratch in Rust for Horizon, the Rust reimplementation of Minecraft Java Edition 26.3. Horizon has no Java runtime, so Create's jar can't run there, and every rule is ported by hand from Create 6.0.10's source.

- **Code layout:** the game logic lives in a pure Rust rules crate, covered by 1,560 unit tests, and a thin layer connects it to Horizon's mod API.
- **Artwork:** Create's own models and textures are imported from your local Create jar at build time and never shipped.

## At a glance

| Area | Ported | |
|---|---:|---:|
| Kinetics | 52 / 57 | 91% |
| Materials | 19 / 20 | 95% |
| Logistics | 31 / 52 | 60% |
| Fluids | 18 / 18 | **100%** |
| Contraptions | 38 / 42 | 90% |
| Schematics | 5 / 5 | **100%** |
| Curiosities | 12 / 13 | 92% |
| Palettes | 39 / 39 | **100%** |
| **Total** | **214 / 246** | **87%** |

These numbers count blocks and items that exist in the mod, so they measure breadth, not how complete each one is. Trains are most of what's left in Logistics.

## What's in

**⚙️ Kinetics**
- **Rotation:** shafts, cogwheels, gearboxes, clutches, gearshifts, belts (including sloped and vertical ones) and encased chain drives.
- **Power sources:** water wheels, windmills, Steam Engines with Boilers and Whistles, the Creative Motor and the Hand Crank.
- **Stress:** speed and stress use Create's own numbers, and the Engineer's Goggles show them.

**🏭 Processing**
- **Machines:** Millstone, Crushing Wheels, Mechanical Press, Mixer and Basin, Encased Fan (washing, smoking, blasting, haunting), Deployer, Saw and Drill.
- **Crafting:** the Mechanical Crafter, Sandpaper polishing and sequenced assembly.

**💧 Fluids (complete)**
- **Moving and storing fluids:** pipes, pumps, valves, smart pipes, Fluid Tanks and the Hose Pulley.
- **Using fluids:** Spout, Item Drain and potion fluids.

**📦 Logistics**
- **Item handling:** Depots, Chutes, Funnels, Tunnels, the Weighted Ejector, the Mechanical Arm and Item Vaults.
- **Filters:** List and Attribute Filters.
- **Information:** the Display Link with Display Boards and Nixie Tubes.
- **Redstone parts:** Redstone Links, Analog Levers, delayers and pulse repeaters, the Stockpile Switch, the Content Observer and the Rose Quartz Lamp.
- **New:** packages, the Packager, the Re-Packager, Chain Conveyors, Frogports and Postboxes.

**🏗️ Moving structures**
- **Movers:** Mechanical Pistons, Bearings, Windmills, the Clockwork Bearing, gantries, the Rope and Elevator Pulleys, chassis and Super Glue.
- **Moving drills, saws and harvesters:** Drills, Saws, Harvesters, Ploughs, Deployers and Funnels work on moving structures, switched by Contraption Controls.
- **Also:** the Sticker, the Redstone Contact and the Portable Storage Interfaces.

**📐 Schematics (complete)**
- **Saving and placing:** the Schematic And Quill saves builds to files, and a Schematic previews one in place as a hologram.
- **Printing:** the Schematic Table writes a saved build onto a schematic, and the Schematicannon prints it block by block.

**🎒 Equipment and extras**
- **Gear:** the Wrench, Goggles, Backtanks, Diving Gear and the Extendo Grip.
- **Tools:** the Potato Cannon, the Wand of Symmetry, the Worldshaper, Linked and Lectern Controllers and the Toolbox.
- **Small blocks:** the Clipboard, Placard, Desk Bell and Seats.

**🧱 Palettes and materials (complete):** every decorative stone, window, casing, copycat and girder, plus the ores, alloys and crushed ores.

**📖 Ponder:** Create's in-game tutorial screens, including a timeline you can scrub.

## Confirmed in game

On Horizon's official Nightly 2026-11-18, play-tests confirmed:

- the Press making iron sheets on a Depot, the Millstone making flour, and the Mixer making andesite alloy in a Basin;
- hoppers and chutes feeding Depots;
- the Engineer's Goggles overlay;
- the Wrench rotating blocks and picking them up;
- cogwheels and shafts snapping into place with a ghost preview, as in Create;
- standing on and riding moving structures: rope pulleys, pistons, bearings, windmills and elevators, with mobs and items pushed and carried;
- about 140 frames per second.

An earlier test confirmed:

- a piston-driven drill tunnelling through cobblestone, with the drops collected in a chest;
- the Sticker;
- the Clockwork Bearing's hands keeping time;
- the Quill saving schematics;
- the Packager boxing up items;
- the Wand of Symmetry mirroring blocks;
- the Spout making mud and filling cauldrons;
- hoppers feeding the Item Drain.

An earlier test covered the kinetics core, fluid machines, redstone parts and small kinetic blocks.

## What's left

**🔧 Fixed but not yet played:** the piston stopping a block early after drilling, the Harvester on a bearing, the Redstone Contact's lamps, the Schematic Table's menu, and drawing Chain Conveyor chains. Packages still loop on the first Chain Conveyor wheel instead of travelling along the chain.

**🧱 Moving structures: what's still missing**

- **Collision is new:** you can stand on and ride moving structures, but mobs riding one never count as standing, so they move as if floating. Riders moving with a structure when it snaps to the grid is written but not yet played.
- **No clicking:** you can't use a block on a moving structure (a lever, a door, the Contraption Controls).
- **Only one mode:** structures always set down where they stop. Create's other modes, like never placing or placing only when lined up, aren't there.
- **Drawn plain:** blocks on a moving structure are drawn as plain block models, so a chest doesn't open and a sail doesn't turn on its own.
- **Saws and drills hurt players but don't knock them back.**
- **Belts and the Creative Fluid Tank can't be moved.**

**📦 Package logistics part 2:** Stock Link, Stock Ticker, Redstone Requester, Factory Gauge, Table Cloths, Shopping List and Item Hatch.

**🏗️ Moving-structure leftovers**

- The Mechanical Roller.
- Riding seats on moving structures.

**🚂 Trains (saved for last):** tracks, stations, signals, bogeys, controls and schedules.

**🛒 Minecart contraptions:** the Cart Assembler, Controller Rail and Minecart Anchor.

**♻️ Simplifications to undo:** the mod now runs on Horizon's new mod API, which lets us replace some stand-ins:

- fixed tables of vanilla recipes instead of reading the game's own;
- polled redstone instead of reacting to neighbour updates.

**🐞 Known bugs:** a Depot under a running Press has twice lost its stack, and the Clipboard keeps no pages.

**🖥️ Screens replaced by clicks:** these have a Create screen that isn't ported. They're set with clicks, the Wrench or commands instead:

- Filters;
- the Stockpile Switch's thresholds;
- the Sequenced Gearshift's program;
- the Display Link's settings;
- the Wand of Symmetry and the Worldshaper;
- the Crafting Blueprint;
- package addresses, set with `/address`.

**🎨 Smaller gaps**

- **Translations:** English only. Create's own translations could be imported from your jar like its art.
- **Looks:** no connected textures on casings and windows, no waterlogging, copper shingles and tiles don't weather, and sliding doors swing like vanilla doors.
- **Recipes:** vanilla recipes are copied into the mod, so a data pack's changes to them aren't seen yet. Block hardness for drills is a fixed table, so it doesn't cover other mods' blocks.
- **Networks:** a rotation network stops at unloaded chunks, and speed changes jump instead of easing.
- **Display Link:** reads only the blocks this port has, so there's no boiler, train or package information.
- **Schematicannon:** no material list, and it doesn't print entities or belts.
- **Sand Paper:** takes the item only when polishing finishes, rather than while you hold it.

**🧪 Not yet tried in game:** the Saw, Plough, Mechanical Arm, Crafter, Display Link, Tunnels, Lectern Controller and Postboxes. They pass their unit tests and compile, but nobody has played with them yet.

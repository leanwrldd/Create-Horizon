//! Engineer's Goggles' overlay (`GoggleOverlayRenderer`): while the player looks at a Create
//! block, a box beside the crosshair says what its block entity says (`addToGoggleTooltip` when
//! the goggles are on the head, `addToTooltip` always).
//!
//! Thin glue: it reads the looked-at block and what the server sent of its block entity (the
//! update tag, `Host::mod_block_entity` on a client), turns that into the plain values of
//! `create_kinetics::rules::goggles_overlay::Subject`, asks the rules for the lines and draws
//! them as `create_kinetics::rules::goggles` lays them out. The words are Create's language
//! file, which the content build puts in the package (`tools/goggles_assets.py`), never typed here.
//!
//! The item itself (`create:goggles`, worn in the head slot, drawn on the head by the item
//! definition's `head` display context) is `tools/content/goggles.py`.
//!
//! Known differences from Create, for the README:
//! - The overlay is drawn by the HUD hook after the minimap and the other boxes; a value box
//!   doesn't hide it (this port has none) and a spectator sees it.
//! - The Brass Tunnel's list of what its group holds isn't sent to clients, so its goggle
//!   tooltip is missing; the Mechanical Arm's "no targets" hint shows at once, with none of
//!   Create's five ticks of warmup after placing it.
//! - A Fluid Pipe has no tooltip, as in Create; the Blaze Burner has none either (Create's
//!   `BlazeBurnerBlockEntity` is not an `IHaveGoggleInformation`).
//! - The goggles' icon is always the goggles (`IHaveCustomOverlayIcon` isn't used).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use create_kinetics::rules::chute::item_motion;
use create_kinetics::rules::deployer::Mode as DeployerMode;
use create_kinetics::rules::contraption_facts::REPLACEABLE;
use create_kinetics::rules::fluid_tank::CAPACITY as TANK_CAPACITY;
use create_kinetics::rules::goggles::{self, Lang, Layout, Line};
use create_kinetics::rules::goggles_blocks::FluidTank;
use create_kinetics::rules::goggles_kinetic::{Kinetic, capacity};
use create_kinetics::rules::goggles_overlay::{PoweredShaft, Special, Subject, overlay};
use create_kinetics::rules::steam_engine::connected_direction;
use create_kinetics::rules::{item_drain, spout};
use create_kinetics::Dir;
use horizon_client::mod_api::{
    Compound, EquipmentSlot, Hooks, Host, HudEvent, ModBlockEntity, NativeScreenKind, Tag,
    serde_json,
};

use super::large_water_wheel;
use super::steam_engine_boiler::read_boiler;
use crate::util;

type Pos = [i32; 3];

const GOGGLES: &str = "create:goggles";
/// How far the crosshair reaches (`Player.blockInteractionRange`, creative's).
const REACH: f32 = 5.0;
/// Where the language comes from: Create's, put in the package by the content build, and
/// vanilla's for the names of items and fluids.
const CREATE_LANG: &str = "assets/create/goggles/lang.json";
/// The names of Create's own blocks, items and fluids (`NAMES` of the content plugins).
const CREATE_NAMES: &str = "assets/create/lang/en_us.json";
const VANILLA_LANG: &str = "assets/minecraft/lang/en_us.json";

const POLE: &str = "create:piston_extension_pole";
const PISTONS: [&str; 2] = ["create:mechanical_piston", "create:sticky_mechanical_piston"];

struct Goggles;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Goggles)
}

/// `GoggleOverlayRenderer.hoverTicks`: the frames the overlay has been up, for its slide in.
static FRAMES: AtomicU32 = AtomicU32::new(0);

/// The language, read once per resource reload (`block_sprites_version`).
static LANG: Mutex<Option<(u64, Arc<Lang>)>> = Mutex::new(None);

fn read_lang(host: &dyn Host, path: &str, into: &mut BTreeMap<String, String>) {
    let Some(bytes) = host.client_resources().read(path) else {
        return;
    };
    if let Ok(entries) = serde_json::from_slice::<BTreeMap<String, serde_json::Value>>(&bytes) {
        for (key, value) in entries {
            if let serde_json::Value::String(text) = value {
                into.insert(key, text);
            }
        }
    }
}

pub(super) fn lang(host: &dyn Host) -> Arc<Lang> {
    let version = host.block_sprites_version();
    let mut cached = LANG.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((had, lang)) = cached.as_ref() {
        if *had == version && !lang.is_empty() {
            return Arc::clone(lang);
        }
    }
    let mut entries = BTreeMap::new();
    read_lang(host, VANILLA_LANG, &mut entries);
    read_lang(host, CREATE_NAMES, &mut entries);
    read_lang(host, CREATE_LANG, &mut entries);
    let lang = Arc::new(Lang::new(entries));
    *cached = Some((version, Arc::clone(&lang)));
    lang
}

// --- reading the block ----------------------------------------------------------------------------

fn kinetic_of(entity: &ModBlockEntity) -> Option<Kinetic> {
    // Every rotating block entity saves its speed (`kinetic::new_entity`).
    if !util::has(&entity.nbt, "Speed") {
        return None;
    }
    let speed = util::float(&entity.nbt, "Speed").unwrap_or(0.0);
    let overstressed = util::boolean(&entity.nbt, "Overstressed");
    let theoretical =
        if overstressed { util::float(&entity.nbt, "Theory").unwrap_or(0.0) } else { speed };
    Some(Kinetic { block: entity.id.to_string(), theoretical, overstressed })
}

fn named(stack: &horizon_client::mod_api::ItemStack) -> (String, i32) {
    (stack.item.to_string(), stack.count as i32)
}

/// The tanks a spout, drain or basin saves under `fluids`: `(fluid, millibuckets)` of each that
/// holds some.
fn saved_fluids(nbt: &Compound) -> Vec<(String, i32)> {
    let Some(Tag::List(list)) = util::get(nbt, "fluids") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|tank| {
            let Some(Tag::String(fluid)) = tank.get("Fluid") else {
                return None;
            };
            let amount = match tank.get("Amount") {
                Some(Tag::Int(n)) => *n,
                _ => 0,
            };
            let fluid = fluid.to_string_lossy();
            (amount > 0 && fluid != "minecraft:empty").then_some((fluid, amount))
        })
        .collect()
}

fn single_tank(nbt: &Compound, capacity: i32) -> FluidTank {
    match saved_fluids(nbt).into_iter().next() {
        Some((fluid, amount)) => FluidTank { fluid: Some(fluid), amount, capacity },
        None => FluidTank { fluid: None, amount: 0, capacity },
    }
}

/// A Piston Extension Pole's line of poles and whether a Mechanical Piston ends it
/// (`GoggleOverlayRenderer`, `PistonExtensionPoleBlock.PlacementHelper.attachedPoles`).
fn pole_length(host: &dyn Host, pos: Pos) -> Option<usize> {
    let facing = host.block_property(pos, "facing").and_then(|f| Dir::from_name(&f))?;
    let axis = facing.axis();
    let mut poles = 1;
    let mut piston = false;
    for dir in [facing, facing.opposite()] {
        let step = dir.normal();
        let mut attached = 0usize;
        let at = |n: i32| [pos[0] + step[0] * n, pos[1] + step[1] * n, pos[2] + step[2] * n];
        while attached < 64 {
            let next = at(attached as i32 + 1);
            let is_pole = host.block(next) == POLE
                && host
                    .block_property(next, "facing")
                    .and_then(|f| Dir::from_name(&f))
                    .is_some_and(|f| f.axis() == axis);
            if !is_pole {
                break;
            }
            attached += 1;
        }
        poles += attached;
        piston |= PISTONS.contains(&host.block(at(attached as i32 + 1)).as_str());
    }
    piston.then_some(poles)
}

/// The tank a Fluid Tank block belongs to: its controller's entity (`getControllerBE`).
fn tank_controller(host: &dyn Host, pos: Pos, entity: &ModBlockEntity) -> Option<ModBlockEntity> {
    match util::get(&entity.nbt, "Controller") {
        Some(Tag::IntArray(v)) if v.len() == 3 && [v[0], v[1], v[2]] != pos => {
            host.mod_block_entity([v[0], v[1], v[2]])
        }
        _ => Some(entity.clone()),
    }
}

fn tank_subject(host: &dyn Host, pos: Pos, entity: &ModBlockEntity) -> Option<Subject> {
    let controller = tank_controller(host, pos, entity)?;
    let nbt = &controller.nbt;
    let width = util::int(nbt, "Size").max(1);
    let size = width * width * util::int(nbt, "Height").max(1);
    let (fluid, amount) = match util::get(nbt, "TankContent") {
        Some(content @ Tag::Compound(_)) => {
            let fluid = match content.get("id") {
                Some(Tag::String(id)) => id.to_string_lossy(),
                _ => String::new(),
            };
            let amount = match content.get("amount") {
                Some(Tag::Int(n)) => *n,
                _ => 0,
            };
            (Some(fluid).filter(|f| !f.is_empty() && f != "minecraft:empty"), amount)
        }
        _ => (None, 0),
    };
    Some(Subject::plain(Special::Tank {
        boiler: read_boiler(nbt),
        size,
        tank: FluidTank { fluid, amount, capacity: size * TANK_CAPACITY },
        creative: entity.id == "create:creative_fluid_tank",
    }))
}

/// A Steam Engine's shaft: two blocks along the way the engine points
/// (`SteamEngineBlockEntity.getShaft`).
fn engine_shaft(host: &dyn Host, pos: Pos) -> Option<PoweredShaft> {
    let facing = Dir::from_name(&host.block_property(pos, "facing")?)?;
    let face = host.block_property(pos, "face")?;
    let way = connected_direction(&face, facing).normal();
    let at = [pos[0] + way[0] * 2, pos[1] + way[1] * 2, pos[2] + way[2] * 2];
    let shaft = host.mod_block_entity(at).filter(|e| &*e.id == "create:powered_shaft")?;
    Some(PoweredShaft {
        kinetic: kinetic_of(&shaft)?,
        power: util::float(&shaft.nbt, "EnginePower").unwrap_or(0.0),
        direction: util::int(&shaft.nbt, "Direction"),
    })
}

/// What the goggles need of the block at `pos`: `None` for one with nothing to say.
fn subject(host: &dyn Host, mut pos: Pos, mut block: String) -> Option<Subject> {
    // `IProxyHoveringInformation`: the blocks round a Large Water Wheel speak for the wheel.
    if block == large_water_wheel::STRUCTURE {
        if let Some(master) = large_water_wheel::master_of(host, pos) {
            pos = master;
            block = host.block(pos);
        }
    }
    if block == POLE {
        return Some(Subject::plain(Special::Pole(pole_length(host, pos))));
    }
    let entity = host.mod_block_entity(pos)?;
    let nbt = &entity.nbt;
    let kinetic = kinetic_of(&entity);
    let id = &*entity.id;
    let special = match id {
        "create:speedometer" => Special::Speedometer,
        "create:stressometer" => Special::Stressometer {
            stress: util::float(nbt, "Stress").unwrap_or(0.0),
            capacity: util::float(nbt, "Capacity").unwrap_or(0.0),
        },
        "create:powered_shaft" => Special::PoweredShaft {
            power: util::float(nbt, "EnginePower").unwrap_or(0.0),
            direction: util::int(nbt, "Direction"),
        },
        "create:steam_engine" => Special::SteamEngine(engine_shaft(host, pos)),
        "create:fluid_tank" | "create:creative_fluid_tank" => {
            return tank_subject(host, pos, &entity);
        }
        "create:spout" => Special::FluidContainer(single_tank(nbt, spout::CAPACITY)),
        "create:item_drain" => Special::FluidContainer(single_tank(nbt, item_drain::CAPACITY)),
        "create:basin" => Special::Basin {
            items: util::stacks_of(nbt, "DRAWN").iter().map(named).collect(),
            fluids: saved_fluids(nbt),
        },
        "create:chute" | "create:smart_chute" => {
            let pull = util::float(nbt, "Pull").unwrap_or(0.0);
            let push = util::float(nbt, "Push").unwrap_or(0.0);
            Special::Chute {
                pull,
                push,
                downward: item_motion(push, pull) < 0.0,
                item: util::get(nbt, "Item").and_then(util::stack_of).as_ref().map(named),
            }
        }
        "create:deployer" => Special::Deployer {
            using: util::get(nbt, "Mode").is_some_and(|mode| {
                matches!(mode, Tag::String(m) if m.to_string_lossy() == DeployerMode::Use.name())
            }),
            held: util::get(nbt, "HeldItem").and_then(util::stack_of).as_ref().map(named),
            overflow: util::boolean(nbt, "Overflow"),
        },
        "create:analog_lever" => Special::AnalogLever(util::int(nbt, "State")),
        "create:steam_whistle" => Special::Whistle(util::int(nbt, "Pitch")),
        "create:mechanical_arm" => Special::Arm { has_targets: util::boolean(nbt, "HasTargets") },
        // `HosePulleyBlockEntity`: the client's `Infinite`.
        "create:hose_pulley" => Special::HosePulley { infinite: util::boolean(nbt, "Infinite") },
        "create:mechanical_bearing" => {
            let front = host
                .block_property(pos, "facing")
                .and_then(|f| Dir::from_name(&f))
                .map(|f| {
                    let n = f.normal();
                    host.block([pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]])
                });
            Special::Bearing {
                running: util::boolean(nbt, "Running"),
                front_replaceable: front.is_some_and(|b| REPLACEABLE.contains(&b.as_str())),
                windmill: false,
            }
        }
        _ if capacity(id) > 0.0 => Special::Generator,
        _ => Special::Plain,
    };
    // `IDisplayAssemblyExceptions`: the bearing and the pistons say why they didn't assemble.
    let exception = match util::get(nbt, "LastException") {
        Some(Tag::String(text)) if !text.to_string_lossy().is_empty() => Some(text.to_string_lossy()),
        _ => None,
    };
    if kinetic.is_none() && matches!(special, Special::Plain | Special::Generator) {
        return None;
    }
    Some(Subject { kinetic, special, exception })
}

// --- drawing --------------------------------------------------------------------------------------

fn draw_rects(host: &dyn Host, event: &mut HudEvent<'_>, layout: &Layout, dx: f32) {
    for rect in &layout.rects {
        let [x, y, w, h] = rect.rect;
        if rect.top == rect.bottom {
            host.fill(event.batch, [x + dx, y, w, h], rect.top);
            continue;
        }
        let rows = h as usize;
        for row in 0..rows {
            let color = goggles::gradient(rect.top, rect.bottom, row, rows);
            host.fill(event.batch, [x + dx, y + row as f32, w, 1.0], color);
        }
    }
}

fn draw_lines(host: &dyn Host, event: &mut HudEvent<'_>, layout: &Layout, lines: &[Line], dx: f32) {
    for (index, line) in lines.iter().enumerate() {
        let y = layout.line_y(index);
        let mut x = layout.text[0] + dx;
        for seg in line {
            let width = host.text_width(&seg.text);
            host.text(event.batch, &seg.text, [x, y], seg.color, true);
            if seg.strike {
                host.fill(event.batch, [x, y + 4.0, width, 1.0], 0xFF00_0000 | seg.color);
            }
            x += width;
        }
    }
}

impl Hooks for Goggles {
    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        let host: &dyn Host = &*host;
        let Some(client) = host.client_state() else {
            return;
        };
        if client.screen != NativeScreenKind::Playing || client.gui_hidden {
            FRAMES.store(0, Ordering::Relaxed);
            return;
        }
        let Some(hit) = host.raycast(REACH) else {
            FRAMES.store(0, Ordering::Relaxed);
            return;
        };
        // A cheap test first: nearly every block looked at is not Create's.
        if !hit.block.starts_with("create:") {
            FRAMES.store(0, Ordering::Relaxed);
            return;
        }
        let pos = [hit.block_x, hit.block_y, hit.block_z];
        let Some(subject) = subject(host, pos, hit.block.clone()) else {
            FRAMES.store(0, Ordering::Relaxed);
            return;
        };
        let wearing = host
            .equipment(EquipmentSlot::Head)
            .is_some_and(|stack| &*stack.item == GOGGLES);
        let lang = lang(host);
        let width = |text: &str| host.text_width(text);
        let lines = overlay(&lang, &subject, wearing, host.sneaking(), &width);
        if lines.is_empty() {
            FRAMES.store(0, Ordering::Relaxed);
            return;
        }
        let frames = FRAMES.fetch_add(1, Ordering::Relaxed) as f32;
        let text_width = lines
            .iter()
            .map(|line| line.iter().map(|seg| host.text_width(&seg.text)).sum::<f32>())
            .fold(0.0, f32::max);
        let layout = goggles::layout(client.gui_size, text_width.ceil(), lines.len());
        let dx = goggles::slide(frames);
        draw_rects(host, event, &layout, dx);
        draw_lines(host, event, &layout, &lines, dx);
        host.item(
            event.batch,
            GOGGLES,
            [layout.icon[0] + dx, layout.icon[1]],
            16.0,
        );
    }
}

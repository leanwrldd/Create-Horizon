//! Resolve storyboard blocks and draw a scene through Horizon's GUI renderer.

use std::collections::{BTreeMap, BTreeSet};

use crate::compat::{
    BlockEntitySubmitEvent, BlockStateId, DisplayContext, Host, HudBatch, ItemStack,
    ModBlockEntity, NativeSceneBlock, NativeSceneView, Tag,
};

use super::scene::{Frame, Scene, State};
use super::screen::Showing;

pub(crate) const ANGLE: &str = "CreatePonderAngle";
pub(crate) const PRESS_PROGRESS: &str = "CreatePonderPressProgress";
pub(crate) const PRESS_MODE: &str = "CreatePonderPressMode";
pub(crate) const TICK: &str = "CreatePonderTick";
/// A number a scene instruction moved: a bearing's angle, a pulley's rope offset, a deployer's hand offset.
pub(crate) const ANIMATED: &str = "CreatePonderAnimated";
/// The four angles of a Mechanical Arm a scene instructed (`rules::ponder_arm`).
pub(crate) const ARM_ANGLES: &str = "CreatePonderArmAngles";
pub(crate) const BASIN_ABOVE: &str = "CreatePonderBasinAbove";
pub(super) const DRAW_STATE: &str = "CreatePonderDrawState";
const ITEM: &str = "CreatePonderItem";
const MODELS: &str = "CreatePonderModels";
const GEOMETRY: &str = "CreatePonderGeometry";
const POSE: &str = "CreatePonderPose";
const POSE_KEYS: [&str; 6] = ["X", "Y", "Z", "Pitch", "Yaw", "Roll"];
const ITEM_POS: [&str; 3] = [
    "CreatePonderItemX",
    "CreatePonderItemY",
    "CreatePonderItemZ",
];
pub(super) const PROXY: &str = "minecraft:barrier";
const BASE: State = ("minecraft:polished_andesite", &[]);

pub type Resolved = BTreeMap<State, BlockStateId>;

pub(super) fn store_models<'a>(
    entity: &mut ModBlockEntity,
    models: impl Iterator<Item = (&'a str, f32, f32)>,
) {
    let models = models
        .map(|(name, x, y)| {
            let mut nbt = crate::Compound::new();
            nbt.insert(
                "Name".into(),
                Tag::String(format!("create:{name}").as_str().into()),
            );
            crate::set_float(&mut nbt, "X", x);
            crate::set_float(&mut nbt, "Y", y);
            Tag::Compound(nbt)
        })
        .collect();
    entity.nbt.insert(MODELS.into(), Tag::List(models));
}

/// The arm's angles, and the item in its claw (`HeldItem`, as the arm saves it).
pub(super) fn store_arm(entity: &mut ModBlockEntity, angles: [f32; 4], held: Option<&str>) {
    entity.nbt.insert(
        ARM_ANGLES.into(),
        Tag::List(angles.iter().map(|angle| Tag::Float(angle.to_bits())).collect()),
    );
    if let Some(item) = held {
        entity
            .nbt
            .insert("HeldItem".into(), crate::util::stack_tag(&ItemStack::new(item, 1)));
    }
}

pub(super) fn store_item(entity: &mut ModBlockEntity, id: &str, offset: [f32; 3]) {
    entity.nbt.insert(ITEM.into(), Tag::String(id.into()));
    for (key, value) in ITEM_POS.into_iter().zip(offset) {
        crate::set_float(&mut entity.nbt, key, value);
    }
}

pub(super) fn store_geometry(
    entity: &mut ModBlockEntity,
    id: &str,
    properties: &[(String, String)],
) {
    let mut geometry = crate::Compound::new();
    geometry.insert("Name".into(), Tag::String(id.into()));
    geometry.insert(
        "Properties".into(),
        Tag::Compound(
            properties
                .iter()
                .map(|(key, value)| (key.as_str().into(), Tag::String(value.as_str().into())))
                .collect(),
        ),
    );
    entity.nbt.insert(GEOMETRY.into(), Tag::Compound(geometry));
}

pub(super) fn store_pose(entity: &mut ModBlockEntity, translation: [f32; 3], rotation: [f32; 3]) {
    let mut pose = crate::Compound::new();
    for (key, value) in POSE_KEYS
        .into_iter()
        .zip(translation.into_iter().chain(rotation))
    {
        crate::set_float(&mut pose, key, value);
    }
    entity.nbt.insert(POSE.into(), Tag::Compound(pose));
}

pub fn resolve(host: &dyn Host, scene: &Scene) -> Resolved {
    let mut resolved = BTreeMap::new();
    let mut variants: BTreeMap<&str, Vec<State>> = BTreeMap::new();
    let states: BTreeSet<_> = scene
        .steps
        .iter()
        .flat_map(|step| step.blocks)
        .map(|block| block.state)
        .chain([BASE, (PROXY, &[][..])])
        .collect();
    for spec @ (id, props) in states {
        if id == "minecraft:air" {
            continue;
        }
        if let Some(default) = host.default_state(id) {
            if props
                .iter()
                .all(|&(key, value)| host.block_state_property(default, key) == Some(value))
            {
                resolved.insert(spec, default);
            } else {
                variants.entry(id).or_default().push(spec);
            }
        }
    }
    // Host exposes state properties by id and a snapshot of the registry. Only
    // take the snapshot when the storyboard needs non-default states.
    if !variants.is_empty() {
        for (id, facts) in host.block_states().iter() {
            let Some(specs) = variants.get(facts.name.as_ref()) else {
                continue;
            };
            for &spec in specs {
                if !resolved.contains_key(&spec)
                    && spec
                        .1
                        .iter()
                        .all(|&(key, value)| host.block_state_property(id, key) == Some(value))
                {
                    resolved.insert(spec, id);
                }
            }
        }
    }
    resolved
}

pub fn draw(
    host: &dyn Host,
    batch: &mut HudBatch,
    rect: [f32; 4],
    showing: Showing,
    frame: Frame<'_>,
    tick: f32,
    resolved: &Resolved,
) {
    let mut models = Vec::new();
    if let Some(&state) = resolved.get(&BASE) {
        for x in 0..5 {
            for z in 0..5 {
                models.push(([x, 0, z], state, None));
            }
        }
    }
    for (&pos, &(block, angle)) in &frame.blocks {
        let Some(&state) = resolved.get(&block.state) else {
            continue;
        };
        let id = block.state.0;
        let item = frame.step.item.as_ref().filter(|item| item.anchor == pos);
        let mut entity = if block.rpm.is_some() || item.is_some() {
            let mut entity = ModBlockEntity::default();
            entity.id = id.to_owned();
            crate::set_float(&mut entity.nbt, TICK, tick);
            if let Some(rpm) = block.rpm {
                crate::set_float(&mut entity.nbt, crate::SPEED, rpm);
                crate::set_float(&mut entity.nbt, ANGLE, angle);
            }
            if frame.step.press == Some(pos) {
                crate::set_float(&mut entity.nbt, PRESS_PROGRESS, frame.progress);
            }
            let [x, y, z] = pos;
            if id == "create:blaze_burner"
                && frame
                    .blocks
                    .get(&[x, y + 1, z])
                    .is_some_and(|(b, _)| b.state.0 == "create:basin")
            {
                entity.nbt.insert(BASIN_ABOVE.into(), Tag::Byte(1));
            }
            if let Some(item) = item {
                let offset = std::array::from_fn(|i| {
                    (item.to[i] - item.from[i]).mul_add(frame.progress, item.from[i])
                });
                store_item(&mut entity, item.id, offset);
            }
            Some(entity)
        } else {
            None
        };

        // Horizon submits a block's baked model before its block entity, even
        // for ENTITYBLOCK_ANIMATED. These Create renderers already draw the
        // whole block. Use the barrier's empty baked model for those positions
        // to avoid a second, static copy in the scene.
        let mut scene_state = state;
        let draws_whole_block = id == crate::belt::ID
            || crate::Block::of(id).is_some_and(|block| block != crate::Block::Press);
        if draws_whole_block
            && let (Some(entity), Some(proxy)) = (entity.as_mut(), resolved.get(&(PROXY, &[][..])))
        {
            scene_state = *proxy;
            entity
                .nbt
                .insert(DRAW_STATE.into(), Tag::Int(state.0 as i32));
        }
        models.push((pos, scene_state, entity));
    }

    let blocks: Vec<_> = models
        .iter()
        .map(|(pos, state, entity)| NativeSceneBlock {
            pos: *pos,
            state: *state,
            entity: entity.as_ref(),
        })
        .collect();
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    host.draw_scene(
        batch,
        rect,
        &blocks,
        NativeSceneView {
            center: [2.5, 1.95, 2.5],
            yaw: showing.yaw,
            pitch: showing.pitch,
            // Imported scenes draw a 5 x 5 base at 30 pixels a block: keep these the same size.
            scale: (width / 8.0).min(height / 6.2).min(30.0) * showing.zoom,
        },
    );
}

pub fn submit(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let nbt = &event.entity.nbt;
    let posed = event.block == PROXY && matches!(nbt.get(&POSE.into()), Some(Tag::Compound(_)));
    if posed && let Some(Tag::Compound(pose)) = nbt.get(&POSE.into()) {
        let [x, y, z, pitch, yaw, roll] =
            POSE_KEYS.map(|key| crate::float(pose, key).unwrap_or(0.0));
        let draw = &mut *event.draw;
        draw.push();
        draw.translate([x, y, z]);
        for (axis, angle) in [
            ([0.0, 0.0, 1.0], roll),
            ([0.0, 1.0, 0.0], yaw),
            ([1.0, 0.0, 0.0], pitch),
        ] {
            draw.rotate_about(axis, angle, [0.5; 3]);
        }
    }
    if event.block == PROXY
        && let Some(geometry) = nbt.get(&GEOMETRY.into())
        && let (Some(Tag::String(name)), Some(Tag::Compound(properties))) =
            (geometry.get("Name"), geometry.get("Properties"))
    {
        let pairs: Vec<_> = properties
            .iter()
            .filter_map(|(key, value)| match value {
                Tag::String(value) => Some((key.to_string_lossy(), value.to_string_lossy())),
                _ => None,
            })
            .collect();
        let properties: Vec<_> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        host.draw_block(event.draw, name.to_string_lossy().as_str(), &properties);
    }
    if event.block == PROXY
        && let Some(Tag::List(models)) = nbt.get(&MODELS.into())
    {
        let draw = &mut *event.draw;
        for model in models {
            if let Tag::Compound(values) = model
                && let Some(Tag::String(name)) = model.get("Name")
            {
                draw.push();
                draw.rotate_about(
                    [0.0, 1.0, 0.0],
                    -crate::float(values, "Y").unwrap_or(0.0),
                    [0.5; 3],
                );
                draw.rotate_about(
                    [1.0, 0.0, 0.0],
                    -crate::float(values, "X").unwrap_or(0.0),
                    [0.5; 3],
                );
                host.draw_model(draw, name.to_string_lossy().as_str(), 0xFF_FFFF);
                draw.pop();
            }
        }
    }
    if event.block == PROXY
        && let Some(Tag::Int(state)) = nbt.get(&DRAW_STATE.into())
    {
        crate::Core::draw_entity(
            host,
            event.draw,
            &event.entity.id,
            event.pos,
            BlockStateId(*state as u32),
            event.entity,
        );
    }
    if let Some(Tag::String(item)) = nbt.get(&ITEM.into()) {
        let draw = &mut *event.draw;
        draw.push();
        draw.translate(ITEM_POS.map(|key| crate::float(nbt, key).unwrap_or_default()));
        draw.rotate_about([1.0, 0.0, 0.0], 90.0, [0.0; 3]);
        draw.scale([0.9; 3]);
        host.draw_item(
            draw,
            &ItemStack::new(item.to_string_lossy().as_str(), 1),
            DisplayContext::Ground,
            None,
            0,
        );
        draw.pop();
    }
    if posed {
        event.draw.pop();
    }
}

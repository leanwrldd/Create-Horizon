//! `Host` (the old trait) on the new hosts.
//!
//! The old `Host` was one trait of about 500 methods; the modules call 146 of them (`old_api.rs`). A method that is
//! not overridden here is a logged neutral value (`gap`): MIGRATION.md lists them, and `tools/hook_map.txt` the hooks.
//! What follows is the first group: the world, block entities, drawing and the few services the kinetics core and the
//! press, millstone and goggles paths use. The rest are step 1b, one group at a time.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use super::*;

/// What a hook was handed by the new API: the server's host, a client's, the shared base of both, or nothing
/// (registration time, and the hooks that have no host at all).
pub enum Adapter<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a dyn ClientHost),
    ClientMut(&'a mut dyn ClientHost),
    Base(&'a dyn mod_api::Host),
    None,
}

/// The standalone models the modules asked for, and the handles they resolved to once the client baked them.
pub(crate) static MODEL_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static MODELS: LazyLock<Mutex<HashMap<String, ModelRef>>> = LazyLock::new(Mutex::default);

/// `model_baking_completed`: resolve every registered standalone model to its handle.
pub fn models_baked(host: &mut dyn ClientHost) {
    let names = MODEL_NAMES.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut resolved = HashMap::new();
    for name in names {
        if let Ok(id) = Identifier::parse(&name) {
            if let Ok(model) = host.render_assets().model(&id) {
                resolved.insert(name, model);
            }
        }
    }
    *MODELS.lock().unwrap_or_else(PoisonError::into_inner) = resolved;
}

fn model(name: &str) -> Option<ModelRef> {
    MODELS.lock().unwrap_or_else(PoisonError::into_inner).get(name).copied()
}

/// The new flag word for the old integer one.
fn flags_of(flags: i32) -> BlockUpdateFlags {
    const ALL: [BlockUpdateFlags; 10] = [
        BlockUpdateFlags::NEIGHBORS,
        BlockUpdateFlags::CLIENTS,
        BlockUpdateFlags::INVISIBLE,
        BlockUpdateFlags::KNOWN_SHAPE,
        BlockUpdateFlags::SUPPRESS_DROPS,
        BlockUpdateFlags::MOVE_BY_PISTON,
        BlockUpdateFlags::SKIP_SHAPE_UPDATE_ON_WIRE,
        BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS,
        BlockUpdateFlags::SKIP_ON_PLACE,
        BlockUpdateFlags::NONE,
    ];
    ALL.iter().filter(|flag| flag.bits() != 0 && flags & flag.bits() != 0).fold(BlockUpdateFlags::NONE, |a, b| a | *b)
}

/// `block_state_property` hands out a `&str` tied to `&self`; the properties of a state never change, so each is
/// leaked once and remembered.
static PROPERTIES: LazyLock<Mutex<HashMap<(u32, String), Option<&'static str>>>> = LazyLock::new(Mutex::default);

/// The property `key` of the state text `Name[a=b,c=d]`.
fn property_of(text: &str, key: &str) -> Option<String> {
    let inner = text.split_once('[')?.1.strip_suffix(']')?;
    inner.split(',').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

impl Adapter<'_> {
    fn state_text_at(&self, pos: [i32; 3]) -> Option<String> {
        let host = self.new_host();
        let level = host.level(host.dimension())?;
        let state = level.block_state(host, pos)?;
        host.block_state_string(state)
    }
}

impl Host for Adapter<'_> {
    fn new_host(&self) -> &dyn mod_api::Host {
        match self {
            Adapter::Server(host) => &**host,
            Adapter::Client(host) => &**host,
            Adapter::ClientMut(host) => &**host,
            Adapter::Base(host) => *host,
            Adapter::None => panic!("compat: no host at registration time"),
        }
    }
    fn new_server(&mut self) -> &mut dyn ServerHost {
        match self {
            Adapter::Server(host) => &mut **host,
            _ => panic!("compat: a server-only call off the server"),
        }
    }

    // --- the level and its blocks --------------------------------------------------------------------------

    fn dimension(&self) -> &'static str {
        match self {
            Adapter::None => "minecraft:overworld",
            _ => self.new_host().dimension(),
        }
    }

    fn level(&self, dimension: &str) -> Option<LevelRef> {
        self.new_host().level(dimension)
    }

    fn block(&self, pos: [i32; 3]) -> String {
        self.state_text_at(pos).map(|text| text.split('[').next().unwrap_or("").to_owned()).unwrap_or_default()
    }

    fn block_property(&self, pos: [i32; 3], key: &str) -> Option<String> {
        property_of(&self.state_text_at(pos)?, key)
    }

    fn block_state_property(&self, state: BlockStateId, key: &str) -> Option<&str> {
        let mut cache = PROPERTIES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = cache.get(&(state.0, key.to_owned())) {
            return *found;
        }
        let text = state.state().and_then(|s| self.new_host().block_state_string(s));
        let value = text.and_then(|text| property_of(&text, key)).map(|v| &*Box::leak(v.into_boxed_str()));
        cache.insert((state.0, key.to_owned()), value);
        value
    }

    fn default_state(&self, block: &str) -> Option<BlockStateId> {
        self.new_host().block_state_id(block).map(Into::into)
    }

    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str) -> bool {
        match self {
            Adapter::Server(host) => host.set_block_property(pos, key, value),
            _ => false,
        }
    }

    fn set_block(&mut self, dimension: &str, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        let Adapter::Server(host) = self else { return false };
        let (Some(level), Some(state)) = (host.level(dimension), state.state()) else { return false };
        level.set_block(&mut **host, pos, state, flags_of(flags)).unwrap_or(false)
    }

    // --- block entities ------------------------------------------------------------------------------------

    fn mod_block_entity(&self, pos: [i32; 3]) -> Option<ModBlockEntity> {
        let host = self.new_host();
        let level = host.level(host.dimension())?;
        let entity = host.block_entity(level, pos)?;
        let id = entity.kind(host).map(|kind| kind.id(host)).unwrap_or_default();
        Some(ModBlockEntity {
            id,
            nbt: entity.data(host)?,
            // The slots are not readable through the new handle (gap): empty.
            items: Vec::new(),
            data: entity.data_slots(host).unwrap_or_default(),
        })
    }

    fn change_mod_block_entity(&mut self, pos: [i32; 3], change: &mut dyn FnMut(&mut ModBlockEntity)) -> bool {
        let Some(mut entity) = self.mod_block_entity(pos) else { return false };
        let before = entity.nbt.clone();
        change(&mut entity);
        if entity.nbt == before {
            return true;
        }
        let Adapter::Server(host) = self else { return false };
        let Some(level) = host.level(host.dimension()) else { return false };
        let Some(handle) = host.block_entity(level, pos) else { return false };
        handle.replace_data(&mut **host, entity.nbt).is_ok()
    }

    fn block_entity_sync(&mut self, pos: [i32; 3]) {
        if let Adapter::Server(host) = self {
            host.block_entity_sync(pos);
        }
    }

    // --- drawing (client, inside block_entity_submit) --------------------------------------------------------

    fn draw_model(&self, draw: &mut BlockEntityDraw, model_name: &str, tint: u32) -> bool {
        let Some(handle) = model(model_name) else { return false };
        draw.ctx().model(handle, tint).is_ok()
    }

    fn draw_block(&self, draw: &mut BlockEntityDraw, block: &str, properties: &[(&str, &str)]) -> bool {
        let Some(key) = self.new_host().block_state_id(block).map(|state| state.block()) else { return false };
        draw.ctx().block(key, properties)
    }

    fn draw_cube(
        &self,
        draw: &mut BlockEntityDraw,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        fn face(face: Option<CubeFace<'_>>) -> Option<mod_api::CubeFace<'_>> {
            face.map(|face| {
                let new = mod_api::CubeFace::new(face.sprite);
                match face.uv {
                    Some(uv) => new.with_uv(uv),
                    None => new,
                }
            })
        }
        let [a, b, c, d, e, f] = faces;
        let faces = [face(a), face(b), face(c), face(d), face(e), face(f)];
        draw.ctx().cube(from, to, faces, tint)
    }

    fn draw_quad(&self, draw: &mut BlockEntityDraw, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        draw.ctx().quad(sprite, corners, uv, tint)
    }

    fn draw_item(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        display: DisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Some(new) = events::stack_new(self.new_host(), stack) else { return false };
        draw.ctx().item(&new, display.into(), owner_yaw, seed)
    }

    // --- small services ------------------------------------------------------------------------------------

    fn run_command(&mut self, line: &str) -> bool {
        match self {
            Adapter::Server(host) => host.commands().run_command(line),
            _ => false,
        }
    }

    fn window(&self) -> bool {
        matches!(self, Adapter::Client(_) | Adapter::ClientMut(_))
    }
}

impl From<DisplayContext> for mod_api::ItemDisplayContext {
    fn from(context: DisplayContext) -> Self {
        match context {
            DisplayContext::None => Self::None,
            DisplayContext::ThirdPersonLeftHand => Self::ThirdPersonLeftHand,
            DisplayContext::ThirdPersonRightHand => Self::ThirdPersonRightHand,
            DisplayContext::FirstPersonLeftHand => Self::FirstPersonLeftHand,
            DisplayContext::FirstPersonRightHand => Self::FirstPersonRightHand,
            DisplayContext::Head => Self::Head,
            DisplayContext::Gui => Self::Gui,
            DisplayContext::Ground => Self::Ground,
            DisplayContext::Fixed => Self::Fixed,
            DisplayContext::OnShelf => Self::OnShelf,
        }
    }
}

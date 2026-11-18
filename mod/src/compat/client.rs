//! Step 2, the client: the HUD, key bindings, screens and tooltips, turned into the old hooks, and the `Host`
//! drawing and key methods the old modules call (`fill`, `text`, `item`, `is_key_down`, ...).
//!
//! The old modules drew on a `HudBatch` they were handed (HUD, mod screens, tooltips). The new API hands a `DrawContext`
//! for the same moments; `HudBatch` wraps it (types.rs), so a `Host::fill(batch, ..)` draws at once.
use super::events::stack_old;
use super::*;

fn create() -> &'static dyn Hooks {
    crate::create()
}

impl Adapter<'_> {
    /// The client's host, on a client hook; None on the server.
    pub(crate) fn client_host(&self) -> Option<&dyn ClientHost> {
        match self {
            Adapter::Client(host) => Some(&**host),
            Adapter::ClientMut(host) => Some(&**host),
            _ => None,
        }
    }

    // --- drawing on a batch --------------------------------------------------------------------------------

    pub(crate) fn hud_fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        if let Some(draw) = batch.draw() {
            draw.fill(rect, argb);
        }
    }

    pub(crate) fn hud_quad(&self, batch: &mut HudBatch, points: [[f32; 2]; 4], argb: u32) {
        if let Some(draw) = batch.draw() {
            draw.quad(points, argb);
        }
    }

    /// The old `text` took an opaque `rgb`.
    pub(crate) fn hud_text(&self, batch: &mut HudBatch, text: &str, at: [f32; 2], rgb: u32, shadow: bool) {
        if let Some(draw) = batch.draw() {
            draw.text(text, at, 0xFF00_0000 | (rgb & 0x00FF_FFFF), shadow);
        }
    }

    pub(crate) fn hud_sprite(&self, batch: &mut HudBatch, name: &str, src: [f32; 4], dst: [f32; 4]) -> bool {
        batch.draw().is_some_and(|draw| draw.sprite(name, src, dst, None))
    }

    pub(crate) fn hud_item(&self, batch: &mut HudBatch, item: &str, at: [f32; 2], size: f32) -> bool {
        let Ok(id) = Identifier::parse(item) else { return false };
        let Some(key) = self.new_host().item_key(&id) else { return false };
        batch.draw().is_some_and(|draw| draw.item(key, at, size))
    }

    pub(crate) fn hud_tooltip(&self, batch: &mut HudBatch, lines: &[Text], at: [f32; 2]) {
        if let Some(draw) = batch.draw() {
            let _ = draw.gui_tooltip(lines, at);
        }
    }

    /// `Font.width` through the client's tooltip font; 0 off the client.
    pub(crate) fn hud_text_width(&self, text: &str) -> f32 {
        self.client_host()
            .and_then(|host| host.text_width(&Text::literal(text.to_owned())).ok())
            .map_or(0.0, |width| width as f32)
    }

    pub(crate) fn hud_scene(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        blocks: &[NativeSceneBlock<'_>],
        view: NativeSceneView,
    ) {
        let Some(draw) = batch.draw() else { return };
        let blocks = blocks
            .iter()
            .filter_map(|block| {
                let entity = match block.entity {
                    Some(entity) => Some(mod_api::SceneBlockEntity::new(
                        Identifier::parse(&entity.id).ok()?,
                        mod_api::item::containers::ModBlockEntityFields {
                            nbt: entity.nbt.clone(),
                            items: Vec::new(),
                            data: entity.data.clone(),
                        },
                    )),
                    None => None,
                };
                Some(mod_api::SceneBlock::new(block.pos, block.state.state()?, entity))
            })
            .collect();
        let view = mod_api::SceneView::new(view.center, view.yaw, view.pitch, view.scale);
        let _ = draw.draw_scene(rect, &mod_api::Scene::new(blocks), view);
    }

    // --- modifiers and keys ----------------------------------------------------------------------------------

    pub(crate) fn modifiers_down(&self) -> ClientModifiers {
        self.client_host().map_or_else(ClientModifiers::default, |host| host.modifiers())
    }

    /// The old `KeyCode` was a GLFW code; the new host names keys as vanilla does (`key.keyboard.w`).
    pub(crate) fn key_down(&self, key: KeyCode) -> bool {
        let Some(host) = self.client_host() else { return false };
        match key_name(key) {
            Some(name) => host.is_key_down(&name),
            None => false,
        }
    }
}

/// Vanilla's name for a GLFW key code: letters and digits, the rest are not asked for.
pub fn key_name(key: KeyCode) -> Option<String> {
    let code = key.0;
    match code {
        0x41..=0x5A | 0x30..=0x39 => Some(format!("key.keyboard.{}", (code as u8 as char).to_ascii_lowercase())),
        0x20 => Some("key.keyboard.space".to_owned()),
        0x100 => Some("key.keyboard.escape".to_owned()),
        _ => None,
    }
}

/// Vanilla's name for a GLFW mouse button.
pub fn mouse_name(button: i32) -> String {
    match button {
        0 => "key.mouse.left".to_owned(),
        1 => "key.mouse.right".to_owned(),
        2 => "key.mouse.middle".to_owned(),
        other => format!("key.mouse.{}", other + 1),
    }
}

/// The old `NativeScreenView` for the new `ScreenView`.
fn screen_old<'a>(screen: &'a mod_api::ScreenView<'a>) -> NativeScreenView<'a> {
    NativeScreenView {
        id: screen.id,
        menu: screen.menu,
        title: screen.title,
        rect: screen.rect,
        slots: screen.slots,
        mouse: screen.mouse,
        partial_tick: screen.partial_tick,
    }
}

// --- the HUD ---------------------------------------------------------------------------------------------------

/// `Hooks::render_gui_post`: the old `hud`, drawn once a frame over the HUD (not under F1).
pub fn render_gui_post(host: &dyn ClientHost, event: &mut mod_api::RenderGuiPostEvent<'_>) {
    // SAFETY: `batch` is dropped at the end of this function, inside the borrow of `event.draw`.
    let mut batch = unsafe { HudBatch::new(&mut *event.draw) };
    let mut corners = Corners::default();
    let mut old = HudEvent { batch: &mut batch, corners: &mut corners };
    create().hud(&mut Adapter::Client(host), &mut old);
}

// --- tooltips --------------------------------------------------------------------------------------------------

/// `Hooks::render_tooltip_pre`: the old `tooltip_render_pre`, which could cancel the tooltip.
pub fn render_tooltip_pre(host: &dyn ClientHost, event: &mut mod_api::RenderTooltipPreEvent<'_>) -> mod_api::Flow {
    let Some(stack) = event.stack.and_then(|stack| stack_old(host, stack)) else { return mod_api::Flow::Continue };
    // SAFETY: see `render_gui_post`.
    let mut batch = unsafe { HudBatch::new(&mut *event.draw) };
    let mut old = TooltipRenderPreEvent {
        stack: &stack,
        batch: &mut batch,
        x: event.place.x as f32,
        y: event.place.y as f32,
    };
    create().tooltip_render_pre(&mut Adapter::Client(host), &mut old)
}

// --- keys ------------------------------------------------------------------------------------------------------

/// `Hooks::input_key`: the old `key`, which fired on a press and on a release.
pub fn input_key(host: &mut dyn ClientHost, event: &mut mod_api::InputKeyEvent<'_>) {
    let pressed = match event.action {
        mod_api::KeyAction::Press => true,
        mod_api::KeyAction::Release => false,
        _ => return,
    };
    let mut old = KeyEvent { key: event.key, pressed };
    create().key(&mut Adapter::ClientMut(host), &mut old);
}

/// `Hooks::mouse_button_post`: the old `key` for a mouse button (`key.mouse.right`).
pub fn mouse_button_post(host: &mut dyn ClientHost, event: &mut mod_api::MouseButtonPostEvent) {
    let name = mouse_name(event.button);
    let mut old = KeyEvent { key: &name, pressed: event.pressed };
    create().key(&mut Adapter::ClientMut(host), &mut old);
}

pub fn movement_input(host: &dyn ClientHost, event: &mut mod_api::MovementInputEvent) {
    let mut old =
        MovementInputEvent { forward: event.forward, strafe: event.strafe, jump: event.jump, sneak: event.sneak };
    create().movement_input(&Adapter::Client(host), &mut old);
    event.forward = old.forward;
    event.strafe = old.strafe;
    event.jump = old.jump;
    event.sneak = old.sneak;
}

// --- screens ---------------------------------------------------------------------------------------------------

pub fn screen_render_post(host: &dyn ClientHost, event: &mut mod_api::ScreenRenderPostEvent<'_>) {
    let screen = screen_old(event.screen);
    // SAFETY: see `render_gui_post`.
    let mut batch = unsafe { HudBatch::new(&mut *event.draw) };
    let mut old = ScreenRenderPostEvent { batch: &mut batch, screen: &screen };
    create().screen_render_post(&Adapter::Client(host), &mut old);
}

pub fn screen_closing(host: &dyn ClientHost, event: &mut mod_api::ScreenClosingEvent<'_>) {
    let screen = screen_old(event.screen);
    let mut old = ScreenClosingEvent { screen: &screen };
    create().screen_closing(&Adapter::Client(host), &mut old);
}

pub fn screen_key_pressed_pre(host: &dyn ClientHost, event: &mut mod_api::ScreenKeyPressedPreEvent<'_>) -> mod_api::Flow {
    let screen = screen_old(event.screen);
    let mut old =
        ScreenKeyPressedPreEvent { screen: &screen, key: event.key, repeat: event.repeat, modifiers: event.modifiers };
    // The old hook returned nothing: it cancelled by `Flow` in later versions; here it is answered by the module.
    create().screen_key_pressed_pre(&Adapter::Client(host), &mut old)
}

pub fn screen_char_typed_pre(host: &dyn ClientHost, event: &mut mod_api::ScreenCharTypedPreEvent<'_>) -> mod_api::Flow {
    let screen = screen_old(event.screen);
    let mut old = ScreenCharTypedPreEvent { screen: &screen, c: event.c, modifiers: event.modifiers };
    create().screen_char_typed_pre(&Adapter::Client(host), &mut old)
}

pub fn screen_mouse_button_pressed_pre(
    host: &dyn ClientHost,
    event: &mut mod_api::ScreenMouseButtonPressedPreEvent<'_>,
) -> mod_api::Flow {
    let screen = screen_old(event.screen);
    let mut old = ScreenMouseButtonPressedPreEvent { screen: &screen, button: event.button, modifiers: event.modifiers };
    create().screen_mouse_button_pressed_pre(&Adapter::Client(host), &mut old)
}

pub fn screen_mouse_button_released_pre(
    host: &dyn ClientHost,
    event: &mut mod_api::ScreenMouseButtonReleasedPreEvent<'_>,
) -> mod_api::Flow {
    let screen = screen_old(event.screen);
    let mut old = ScreenMouseButtonReleasedPreEvent { screen: &screen, button: event.button, modifiers: event.modifiers };
    create().screen_mouse_button_released_pre(&Adapter::Client(host), &mut old)
}

pub fn screen_mouse_scrolled_pre(
    host: &dyn ClientHost,
    event: &mut mod_api::ScreenMouseScrolledPreEvent<'_>,
) -> mod_api::Flow {
    let screen = screen_old(event.screen);
    let mut old = ScreenMouseScrolledPreEvent { screen: &screen, scroll: event.scroll, modifiers: event.modifiers };
    create().screen_mouse_scrolled_pre(&Adapter::Client(host), &mut old)
}

// --- the world --------------------------------------------------------------------------------------------------

/// `Hooks::render_level_stage`: the old `world_primitives`, once a frame after the translucent blocks.
pub fn render_level_stage(host: &dyn ClientHost, event: &mut mod_api::RenderLevelStageEvent<'_>) {
    if event.stage != mod_api::RenderLevelStage::AfterTranslucentBlocks {
        return;
    }
    let mut primitives = WorldPrimitives::default();
    let mut old =
        WorldPrimitivesEvent { partial: event.view.partial_tick, camera: event.view.camera, primitives: &mut primitives };
    create().world_primitives(&Adapter::Client(host), &mut old);
    if !primitives.lines.is_empty() {
        let _ = event.draw.lines(&primitives.lines);
    }
    if !primitives.quads.is_empty() {
        let _ = event.draw.quads(&primitives.quads);
    }
}

// --- mod screens ------------------------------------------------------------------------------------------------

impl Adapter<'_> {
    /// `Host::mod_screen`: the mod whose own screen is open.
    pub(crate) fn screen_of_mod(&self) -> Option<&str> {
        self.client_host()?.mod_screen()
    }

    /// `Host::mod_screen_open`: only a client hook that holds the client mutably can open one.
    pub(crate) fn open_mod_screen(&mut self, mod_id: &str) -> bool {
        match self {
            Adapter::ClientMut(host) => host.mod_screen_open(mod_id),
            _ => false,
        }
    }

    pub(crate) fn close_mod_screen(&mut self, mod_id: &str) {
        if let Adapter::ClientMut(host) = self {
            host.mod_screen_close(mod_id);
        }
    }
}

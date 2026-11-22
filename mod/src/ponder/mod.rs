//! Client-side Ponder hooks. Inventory Ponder keeps the underlying menu open.

mod archive;
mod archive_models;
mod render;
mod scene;
mod scenes;
mod screen;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::compat::{
    BlockEntitySubmitEvent, ClientTickPostEvent, Flow, Hooks, Host, KeyCode, KeyEvent,
    NativeScreenView, ScreenCharTypedPreEvent, ScreenClosingEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, StandaloneModelsEvent, Text, TooltipEvent,
    TooltipRenderPreEvent,
};

pub(crate) use render::{ANGLE, ANIMATED, ARM_ANGLES, BASIN_ABOVE, PRESS_MODE, PRESS_PROGRESS, TICK};
use scene::{Hold, Topic};
use scenes::{SCENES, for_item};
use screen::{Layout, Viewer};

const MOD: &str = "create";
const LIBRARY_KEY: &str = "key.keyboard.k";

#[derive(Default)]
pub struct Ponder {
    windows: Mutex<HashMap<u64, Window>>,
    catalogs: Mutex<HashMap<u64, (u64, Option<Arc<archive::Catalog>>)>>,
}

#[derive(Default)]
struct Window {
    active: Option<Active>,
    hovered: Option<Topic>,
    hover_at: Option<[f32; 2]>,
    hold: Hold,
    states: Option<(u64, usize, Arc<render::Resolved>)>,
    catalog: Option<Arc<archive::Catalog>>,
    imported: [Option<CachedScene>; 2],
    close_requested: bool,
}

type CachedScene = (u64, usize, Option<Arc<archive::Scene>>);

fn input_scene(cache: &[Option<CachedScene>; 2], page: screen::Page) -> Option<&archive::Scene> {
    let screen::Page::Imported(showing) = page else {
        return None;
    };
    cache
        .iter()
        .flatten()
        .find(|(_, index, _)| *index == showing.index)
        .and_then(|(_, _, scene)| scene.as_deref())
}

struct Active {
    /// "mod" for the library opened in-world, or the inventory underneath Ponder.
    screen: &'static str,
    viewer: Viewer,
    orbiting: Option<[f32; 2]>,
}

impl Window {
    fn exit(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.screen == "mod")
        {
            self.close_requested = true;
        }
        self.active = None;
        self.hovered = None;
    }
}

impl Ponder {
    fn catalog(&self, host: &dyn Host, window: u64) -> Option<Arc<archive::Catalog>> {
        let version = host.block_sprites_version();
        let mut catalogs = self.catalogs.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = catalogs
            .entry(window)
            .or_insert_with(|| (version, archive::Catalog::load(host).map(Arc::new)));
        if entry.0 != version {
            *entry = (version, archive::Catalog::load(host).map(Arc::new));
        }
        entry.1.clone()
    }

    fn imported(
        &self,
        host: &dyn Host,
        window: u64,
        catalog: &archive::Catalog,
        index: usize,
    ) -> Option<Arc<archive::Scene>> {
        let version = host.block_sprites_version();
        if let Some((_, _, scene)) = self.windows().get(&window).and_then(|w| {
            w.imported
                .iter()
                .flatten()
                .find(|(key, id, _)| (*key, *id) == (version, index))
        }) {
            return scene.clone();
        }
        let scene = archive::Scene::load(host, catalog.scenes.get(index)?).map(Arc::new);
        if let Some(window) = self.windows().get_mut(&window) {
            window.imported.rotate_right(1);
            window.imported[0] = Some((version, index, scene.clone()));
        }
        scene
    }

    fn windows(&self) -> MutexGuard<'_, HashMap<u64, Window>> {
        self.windows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn input(
        &self,
        host: &dyn Host,
        screen: &NativeScreenView<'_>,
        apply: impl FnOnce(&mut Window, Layout) -> Flow,
    ) -> Flow {
        if screen.id == "mod" && host.mod_screen() != Some(MOD) {
            return Flow::Continue;
        }
        let Some(client) = host.client_state() else {
            return Flow::Continue;
        };
        let mut windows = self.windows();
        let Some(window) = windows.get_mut(&client.window_id) else {
            return Flow::Continue;
        };
        if window
            .active
            .as_ref()
            .is_some_and(|active| active.screen == screen.id)
        {
            apply(window, Layout::new(client.gui_size))
        } else {
            Flow::Continue
        }
    }

    fn states(&self, host: &dyn Host, window_id: u64, index: usize) -> Arc<render::Resolved> {
        let version = host.block_sprites_version();
        if let Some((cached_version, cached_index, states)) = self
            .windows()
            .get(&window_id)
            .and_then(|w| w.states.as_ref())
            && (*cached_version, *cached_index) == (version, index)
        {
            return Arc::clone(states);
        }
        let states = Arc::new(render::resolve(host, &SCENES[index]));
        if let Some(window) = self.windows().get_mut(&window_id) {
            window.states = Some((version, index, Arc::clone(&states)));
        }
        states
    }

    fn over_topic(&self, host: &dyn Host, screen: &NativeScreenView<'_>) -> bool {
        if !matches!(screen.id, "inventory" | "creative") {
            return false;
        }
        let Some(client) = host.client_state() else {
            return false;
        };
        self.windows()
            .get(&client.window_id)
            .and_then(|w| w.hover_at)
            == Some(screen.mouse)
    }

    fn viewer(
        &self,
        window_id: u64,
        screen: &NativeScreenView<'_>,
        focused: bool,
        holding_w: bool,
        now: Instant,
    ) -> Option<Viewer> {
        let mut windows = self.windows();
        let window = windows.get_mut(&window_id)?;
        if let Some(active) = &mut window.active {
            if active.screen != screen.id {
                return None;
            }
            if !focused {
                active.orbiting = None;
            }
            if let Some(previous) = &mut active.orbiting
                && let Some(showing) = active.viewer.showing()
            {
                // Track pointer deltas independently of inventory drag handling.
                showing.yaw =
                    (showing.yaw + (screen.mouse[0] - previous[0]) * 0.8).rem_euclid(360.0);
                showing.pitch =
                    (showing.pitch + (screen.mouse[1] - previous[1]) * 0.6).clamp(-70.0, 80.0);
                *previous = screen.mouse;
            }
            return Some(active.viewer);
        }
        let hovering = window
            .hovered
            .take()
            .filter(|_| focused && matches!(screen.id, "inventory" | "creative"));
        window.hover_at = hovering.as_ref().map(|_| screen.mouse);
        let topic = window.hold.update(hovering.filter(|_| holding_w), now)?;
        window.hold = Hold::default();
        window.hover_at = None;
        let mut viewer = Viewer::library(topic.imported);
        let item = window
            .catalog
            .as_ref()
            .filter(|_| topic.imported)
            .and_then(|catalog| catalog.scenes.get(topic.index))
            .and_then(|entry| {
                entry
                    .items
                    .iter()
                    .position(|item| item == topic.item.as_ref())
            });
        if let Some(item) = item {
            viewer.open_item(topic.index, item, now);
        } else {
            viewer.open(topic.index, topic.imported, now);
        }
        window.active = Some(Active {
            screen: screen.id,
            viewer,
            orbiting: None,
        });
        Some(viewer)
    }
}

impl Hooks for Ponder {
    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        if host.mod_screen() != Some(MOD) {
            return;
        }
        let Some(client) = host.client_state() else {
            return;
        };
        let close = self
            .windows()
            .get_mut(&client.window_id)
            .is_some_and(|w| std::mem::take(&mut w.close_requested));
        if close {
            host.mod_screen_close(MOD);
        }
    }

    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend(
            archive_models::MODELS
                .lines()
                .map(str::to_owned),
        );
    }

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        if !event.pressed
            || event.key != LIBRARY_KEY
            || !host.playing()
            || host.default_state("create:shaft").is_none()
        {
            return;
        }
        let Some(client) = host.client_state() else {
            return;
        };
        if host.mod_screen_open(MOD) {
            let catalog = self.catalog(host, client.window_id);
            let mut windows = self.windows();
            let window = windows.entry(client.window_id).or_default();
            window.active = Some(Active {
                screen: "mod",
                viewer: Viewer::library(catalog.is_some()),
                orbiting: None,
            });
            window.catalog = catalog;
        }
    }

    fn tooltip(&self, host: &dyn Host, event: &mut TooltipEvent<'_>) {
        let item = &event.stack.item;
        if for_item(item).is_some()
            || host
                .client_state()
                .and_then(|client| self.catalog(host, client.window_id))
                .is_some_and(|catalog| catalog.for_item(item).is_some())
        {
            event
                .lines
                .push((Text::literal("Hold [W] to Ponder"), 0xFFDF_B776));
        }
    }

    fn tooltip_render_pre(
        &self,
        host: &mut dyn Host,
        event: &mut TooltipRenderPreEvent<'_>,
    ) -> Flow {
        let Some(client) = host.client_state() else {
            return Flow::Continue;
        };
        let catalog = self.catalog(host, client.window_id);
        let item = &event.stack.item;
        let hovered =
            if let Some(index) = catalog.as_ref().and_then(|catalog| catalog.for_item(item)) {
                Some(Topic {
                    index,
                    item: item.as_ref().into(),
                    imported: true,
                })
            } else {
                for_item(item).map(|(index, id)| Topic {
                    index,
                    item: id.into(),
                    imported: false,
                })
            };
        let mut windows = self.windows();
        if hovered.is_some() {
            let window = windows.entry(client.window_id).or_default();
            window.hovered = hovered;
            window.catalog = catalog;
        }
        if windows
            .get(&client.window_id)
            .is_some_and(|w| w.active.is_some())
        {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn screen_render_post(&self, host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>) {
        let id = event.screen.id;
        if id == "mod" && host.mod_screen() != Some(MOD) {
            return;
        }
        let Some(client) = host.client_state() else {
            return;
        };
        let now = Instant::now();
        let holding_w =
            host.is_key_down(KeyCode::KeyW) && !host.has_control_down() && !host.has_alt_down();
        let Some(viewer) = self.viewer(
            client.window_id,
            event.screen,
            client.focused,
            holding_w,
            now,
        ) else {
            return;
        };
        let layout = Layout::new([event.batch.width, event.batch.height]);
        let catalog = self
            .windows()
            .get(&client.window_id)
            .and_then(|w| w.catalog.clone());
        let imported =
            if let (Some(catalog), screen::Page::Imported(showing)) = (&catalog, viewer.page) {
                self.imported(host, client.window_id, catalog, showing.index)
            } else {
                None
            };
        let outgoing =
            if let (Some(catalog), screen::Page::Imported(showing)) = (&catalog, viewer.page) {
                showing
                    .outgoing(now)
                    .and_then(|index| self.imported(host, client.window_id, catalog, index))
            } else {
                None
            };
        screen::draw(
            host,
            event.batch,
            layout,
            viewer,
            event.screen.mouse,
            now,
            screen::Archive {
                catalog: catalog.as_deref(),
                scene: imported.as_deref(),
                outgoing: outgoing.as_deref(),
            },
            |batch, showing, rect, frame, tick| {
                let states = self.states(host, client.window_id, showing.index);
                render::draw(host, batch, rect, showing, frame, tick, &states);
            },
        );
    }

    fn screen_closing(&self, host: &dyn Host, event: &mut ScreenClosingEvent<'_>) {
        let Some(client) = host.client_state() else {
            return;
        };
        let mut windows = self.windows();
        if let Some(window) = windows.get_mut(&client.window_id) {
            window.hold = Hold::default();
            window.hovered = None;
            window.hover_at = None;
            if window
                .active
                .as_ref()
                .is_none_or(|active| active.screen == event.screen.id)
            {
                windows.remove(&client.window_id);
            }
        }
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        // A focused creative search can otherwise replace its results when
        // W is pressed, before the hold can complete over the item.
        if event.key == "key.keyboard.w"
            && event.modifiers & 0b1110 == 0
            && self.over_topic(host, event.screen)
        {
            return Flow::Cancel;
        }
        self.input(host, event.screen, |window, _| {
            if matches!(event.key, "key.keyboard.escape" | "key.keyboard.e") {
                if !event.repeat && window.active.as_mut().unwrap().viewer.back() {
                    window.exit();
                }
            } else if !event.repeat {
                let archive = screen::Archive {
                    catalog: window.catalog.as_deref(),
                    scene: input_scene(
                        &window.imported,
                        window.active.as_ref().unwrap().viewer.page,
                    ),
                    outgoing: None,
                };
                window
                    .active
                    .as_mut()
                    .unwrap()
                    .viewer
                    .key(event.key, Instant::now(), archive);
            }
            Flow::Cancel
        })
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        if event.c.eq_ignore_ascii_case(&'w')
            && event.modifiers & 0b1110 == 0
            && self.over_topic(host, event.screen)
        {
            return Flow::Cancel;
        }
        self.input(host, event.screen, |_, _| Flow::Cancel)
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        self.input(host, event.screen, |window, layout| {
            let active = window.active.as_mut().unwrap();
            if event.button == 0 {
                active.orbiting = (active.viewer.showing().is_some()
                    && layout.can_orbit(event.screen.mouse))
                .then_some(event.screen.mouse);
                let archive = screen::Archive {
                    catalog: window.catalog.as_deref(),
                    scene: input_scene(&window.imported, active.viewer.page),
                    outgoing: None,
                };
                if active
                    .viewer
                    .click(layout, event.screen.mouse, Instant::now(), archive)
                {
                    window.exit();
                }
            }
            Flow::Cancel
        })
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        self.input(host, event.screen, |window, _| {
            if event.button == 0 {
                window.active.as_mut().unwrap().orbiting = None;
            }
            Flow::Cancel
        })
    }

    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        self.input(host, event.screen, |window, layout| {
            let viewer = &mut window.active.as_mut().unwrap().viewer;
            if event.modifiers & 2 != 0
                && layout.can_orbit(event.screen.mouse)
                && let Some(showing) = viewer.showing()
            {
                showing.zoom =
                    (showing.zoom * 1.12_f32.powf(event.scroll[1] as f32)).clamp(0.6, 1.7);
            } else {
                let archive = screen::Archive {
                    catalog: window.catalog.as_deref(),
                    scene: input_scene(&window.imported, viewer.page),
                    outgoing: None,
                };
                viewer.scroll(layout, archive, event.scroll[1], Instant::now());
            }
            Flow::Cancel
        })
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        render::submit(host, event);
    }
}

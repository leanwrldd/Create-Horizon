//! The layouts and interaction rules of Ponder 1.0.82.

use std::time::Instant;

use crate::compat::{Host, HudBatch, Text};

use super::archive::{self, CaptionStyle};
use super::scene::{Category, Frame, Playback};
use super::scenes::SCENES;

const INK: u32 = 0xFFEE_EEEE;
const MUTED: u32 = 0xFFA3_A3A3;
const EDGE: u32 = 0x60C0_C0FF;
const HOT: u32 = 0xF0C0_C0FF;
const WIDGETS: &str = "create:gui/ponder/widgets";
const CATEGORIES: [(Category, &str); 3] = [
    (Category::Rotation, "Rotation"),
    (Category::Transport, "Transport"),
    (Category::Processing, "Processing"),
];

#[derive(Clone, Copy)]
pub struct Viewer {
    pub page: Page,
    category: Category,
    previous: [Option<Page>; 8],
    comfy: bool,
}

#[derive(Clone, Copy)]
pub enum Page {
    Library,
    Index(usize),
    Topic(Option<usize>, usize),
    Scene(Showing),
    Imported(Showing),
}

#[derive(Clone, Copy)]
pub struct Showing {
    pub index: usize,
    play: Playback,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub shift: [f32; 2],
    subject: Option<(usize, usize)>,
    inspect: Option<bool>,
    slide: Option<Slide>,
}

#[derive(Clone, Copy)]
struct Slide {
    index: usize,
    tick: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    sign: f32,
    began: Instant,
}

impl Showing {
    fn new(index: usize, now: Instant, subject: Option<(usize, usize)>) -> Self {
        Self {
            index,
            play: Playback::new(now),
            yaw: -35.0,
            pitch: 35.0,
            zoom: 1.0,
            shift: [0.0; 2],
            subject,
            inspect: None,
            slide: None,
        }
    }

    fn tick(self, now: Instant, limit: f32, comfy: bool, scene: Option<&archive::Scene>) -> f32 {
        self.play
            .tick_with(now, limit, |tick, elapsed| match (comfy, scene) {
                (true, Some(scene)) => scene.reading_tick(tick, elapsed),
                (true, None) => tick + elapsed / 3.0,
                _ => tick + elapsed,
            })
    }

    pub fn outgoing(self, now: Instant) -> Option<usize> {
        let slide = self.slide?;
        (slide.residual(now) >= 1.0 / 512.0).then_some(slide.index)
    }

    fn animation(self, now: Instant) -> (Self, Option<(Self, f32)>) {
        let Some(slide) = self.slide.filter(|_| self.outgoing(now).is_some()) else {
            return (self, None);
        };
        let residual = slide.residual(now);
        let mut incoming = self;
        incoming.shift = slide_offset(slide.sign * residual);
        let mut old = Self::new(slide.index, now, self.subject);
        old.yaw = slide.yaw;
        old.pitch = slide.pitch;
        old.zoom = slide.zoom;
        old.shift = slide_offset(slide.sign * (residual - 1.0));
        (incoming, Some((old, slide.tick + slide.elapsed(now))))
    }
}

impl Slide {
    fn elapsed(self, now: Instant) -> f32 {
        now.duration_since(self.began).as_secs_f32() * 20.0
    }

    fn residual(self, now: Instant) -> f32 {
        0.75_f32.powf(self.elapsed(now))
    }
}

fn slide_offset(index_diff: f32) -> [f32; 2] {
    let slide = (200.0 + 400.0 * index_diff.powi(2)) * index_diff;
    let angle = 55.0_f32.to_radians();
    [
        slide * angle.cos(),
        -slide * angle.sin() * 35.0_f32.to_radians().sin(),
    ]
}

#[derive(Clone, Copy, Default)]
pub struct Archive<'a> {
    pub catalog: Option<&'a archive::Catalog>,
    pub scene: Option<&'a archive::Scene>,
    pub outgoing: Option<&'a archive::Scene>,
}

impl Archive<'_> {
    fn label<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.catalog
            .map_or(fallback, |catalog| catalog.ui(key, fallback))
    }
}

impl Viewer {
    pub fn library(full: bool) -> Self {
        Self {
            page: if full { Page::Index(0) } else { Page::Library },
            category: Category::Rotation,
            previous: [None; 8],
            comfy: false,
        }
    }

    fn push(&mut self, page: Page) {
        self.previous.rotate_right(1);
        self.previous[0] = Some(self.page);
        self.page = page;
    }

    /// Returns true when there is no earlier Ponder screen to return to.
    pub fn back(&mut self) -> bool {
        if let Some(page) = self.previous[0] {
            self.page = page;
            self.previous.rotate_left(1);
            self.previous[7] = None;
            false
        } else {
            true
        }
    }

    pub fn open(&mut self, index: usize, imported: bool, now: Instant) {
        if imported {
            self.open_item(index, 0, now);
        } else {
            self.category = SCENES[index].category;
            self.page = Page::Scene(Showing::new(index, now, None));
        }
    }

    pub fn open_item(&mut self, index: usize, item: usize, now: Instant) {
        self.page = Page::Imported(Showing::new(index, now, Some((index, item))));
    }

    pub fn showing(&mut self) -> Option<&mut Showing> {
        match &mut self.page {
            Page::Scene(showing) | Page::Imported(showing) => Some(showing),
            _ => None,
        }
    }

    fn adjacent(
        self,
        showing: Showing,
        forward: bool,
        catalog: Option<&archive::Catalog>,
    ) -> Option<usize> {
        let entries: Vec<_> = match self.page {
            Page::Scene(_) => self.guides().map(|(i, _)| i).collect(),
            Page::Imported(_) => {
                let catalog = catalog?;
                let item = subject(catalog, showing)?;
                catalog
                    .scenes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, entry)| entry.items.iter().any(|id| id == item).then_some(i))
                    .collect()
            }
            _ => return None,
        };
        let row = entries.iter().position(|&entry| entry == showing.index)?;
        let next = if forward {
            row + 1
        } else {
            row.checked_sub(1)?
        };
        entries.get(next).copied()
    }

    fn navigate(&mut self, forward: bool, now: Instant, archive: Archive<'_>) {
        let Some(showing) = self.showing().copied() else {
            return;
        };
        if let Some(index) = self.adjacent(showing, forward, archive.catalog) {
            let mut next = Showing::new(index, now, showing.subject);
            if let Some(scene) = archive.scene {
                next.slide = Some(Slide {
                    index: showing.index,
                    tick: showing.tick(now, scene.duration(), self.comfy, Some(scene)),
                    yaw: showing.yaw,
                    pitch: showing.pitch,
                    zoom: showing.zoom,
                    sign: if forward { 1.0 } else { -1.0 },
                    began: now,
                });
            }
            *self.showing().unwrap() = next;
        }
    }

    pub fn key(&mut self, key: &str, now: Instant, archive: Archive<'_>) {
        match key {
            "key.keyboard.home" => {
                self.page = if archive.catalog.is_some() {
                    Page::Index(0)
                } else {
                    Page::Library
                };
                self.previous = [None; 8];
                return;
            }
            "key.keyboard.left" | "key.keyboard.a" => {
                return self.navigate(false, now, archive);
            }
            "key.keyboard.right" | "key.keyboard.d" => {
                return self.navigate(true, now, archive);
            }
            _ => {}
        }
        let (duration, scene) = match self.page {
            Page::Scene(showing) => (SCENES[showing.index].duration(), None),
            Page::Imported(_) => (
                archive.scene.map_or(1.0, archive::Scene::duration),
                archive.scene,
            ),
            _ => return,
        };
        let comfy = self.comfy;
        let showing = self.showing().unwrap();
        let tick = showing.tick(now, duration, comfy, scene);
        match key {
            "key.keyboard.down" | "key.keyboard.s" | "key.keyboard.r" => {
                showing.play = Playback::new(now);
                showing.inspect = None;
            }
            "key.keyboard.q" if scene.is_some() => {
                if let Some(resume) = showing.inspect.take() {
                    showing.play.set(tick, now, resume);
                } else {
                    showing.inspect = Some(showing.play.running() && tick < duration);
                    showing.play.seek(tick);
                }
            }
            "key.keyboard.space" => {
                showing.inspect = None;
                if comfy {
                    let running = showing.play.running() && tick < duration;
                    showing
                        .play
                        .set(if tick == duration { 0.0 } else { tick }, now, !running);
                } else {
                    showing.play.toggle(now, duration);
                }
            }
            "comfy" => {
                showing.play.set(tick, now, showing.play.running());
                self.comfy = !comfy;
            }
            _ => {}
        }
    }

    /// True when the close button returns control to Horizon.
    pub fn click(
        &mut self,
        layout: Layout,
        at: [f32; 2],
        now: Instant,
        archive: Archive<'_>,
    ) -> bool {
        if self.previous[0].is_some() && contains(layout.back(), at) {
            return self.back();
        }
        match self.page {
            Page::Library => {
                let entries: Vec<_> = self.guides().collect();
                let grid = layout.grid(self.page, entries.len());
                for (slot, (index, _)) in entries.into_iter().enumerate() {
                    if contains(grid.tile(slot), at) {
                        self.push(Page::Scene(Showing::new(index, now, None)));
                        return false;
                    }
                }
                for (i, (category, _)) in CATEGORIES.into_iter().enumerate() {
                    if contains(layout.tab(i), at) {
                        self.category = category;
                    }
                }
            }
            Page::Index(page) | Page::Topic(_, page) => {
                let Some(catalog) = archive.catalog else {
                    return false;
                };
                if matches!(self.page, Page::Index(_)) {
                    if contains(layout.extra(false), at) {
                        self.push(Page::Topic(None, 0));
                        return false;
                    }
                    if contains(layout.extra(true), at) {
                        self.push(Page::Library);
                        return false;
                    }
                }
                let cards = self.cards(catalog);
                let grid = layout.grid(self.page, cards.len());
                // A tag's main item, drawn left of the grid.
                if let Some(card) = self.main_card(catalog)
                    && contains(grid.main_tile(), at)
                    && let Some(Open::Story(scene, item)) = card.action
                {
                    self.push(Page::Imported(Showing::new(scene, now, Some((scene, item)))));
                    return false;
                }
                if page > 0 && contains(layout.page(false), at) {
                    self.set_page(page - 1);
                } else if page + 1 < grid.pages && contains(layout.page(true), at) {
                    self.set_page(page + 1);
                } else {
                    for (slot, card) in cards
                        .into_iter()
                        .skip(page * grid.capacity)
                        .take(grid.capacity)
                        .enumerate()
                    {
                        if contains(grid.tile(slot), at) {
                            if let Some(action) = card.action {
                                match action {
                                    Open::Tag(tag) => self.push(Page::Topic(Some(tag), 0)),
                                    Open::Story(scene, item) => self.push(Page::Imported(
                                        Showing::new(scene, now, Some((scene, item))),
                                    )),
                                }
                            }
                            break;
                        }
                    }
                }
            }
            page @ (Page::Scene(showing) | Page::Imported(showing)) => {
                if contains(layout.control(2), at) {
                    return self.back();
                }
                if let (Page::Imported(_), Some(catalog)) = (page, archive.catalog) {
                    for (slot, tag) in self
                        .tags(catalog, showing)
                        .take(layout.tag_capacity())
                        .enumerate()
                    {
                        if contains(layout.tag(slot), at) {
                            self.push(Page::Topic(Some(tag), 0));
                            return false;
                        }
                    }
                }
                for (slot, key) in [
                    (0, "key.keyboard.q"),
                    (1, "key.keyboard.left"),
                    (3, "key.keyboard.right"),
                    (4, "key.keyboard.down"),
                    (5, "comfy"),
                ] {
                    if contains(layout.control(slot), at) {
                        self.key(key, now, archive);
                        return false;
                    }
                }
                let (duration, markers) = match page {
                    Page::Scene(_) => (
                        SCENES[showing.index].duration(),
                        SCENES[showing.index].markers().collect(),
                    ),
                    _ => archive.scene.map_or((1.0, Vec::new()), |scene| {
                        (scene.duration(), scene.markers().to_vec())
                    }),
                };
                if let Some(at) = hovered_mark(layout, at, duration, &markers) {
                    self.showing()
                        .unwrap()
                        .play
                        .set(at, now, showing.inspect.is_none());
                } else if contains(layout.next(), at)
                    && showing.tick(now, f32::MAX, self.comfy, archive.scene) >= duration + 50.0
                {
                    self.navigate(true, now, archive);
                }
            }
        }
        false
    }

    pub fn scroll(&mut self, layout: Layout, archive: Archive<'_>, delta: f64, now: Instant) {
        if delta == 0.0 {
            return;
        }
        if self.showing().is_some() {
            self.navigate(delta > 0.0, now, archive);
        } else if let (Some(catalog), Page::Index(page) | Page::Topic(_, page)) =
            (archive.catalog, self.page)
        {
            let last = layout.grid(self.page, self.cards(catalog).len()).pages - 1;
            self.set_page(if delta < 0.0 {
                (page + 1).min(last)
            } else {
                page.saturating_sub(1)
            });
        }
    }

    fn set_page(&mut self, page: usize) {
        self.page = match self.page {
            Page::Topic(tag, _) => Page::Topic(tag, page),
            _ => Page::Index(page),
        }
    }

    fn tags(self, catalog: &archive::Catalog, showing: Showing) -> impl Iterator<Item = usize> {
        let item = subject(catalog, showing);
        catalog.tags.iter().enumerate().filter_map(move |(i, tag)| {
            (catalog.scenes[showing.index].tags.contains(&tag.id)
                || item.is_some_and(|item| tag.items.iter().any(|name| name == item)))
            .then_some(i)
        })
    }

    fn cards(self, catalog: &archive::Catalog) -> Vec<Card<'_>> {
        match self.page {
            Page::Index(_) => catalog
                .tags
                .iter()
                .enumerate()
                .filter(|(_, tag)| tag.index)
                .map(|(i, tag)| Card {
                    action: Some(Open::Tag(i)),
                    title: tag.title.clone(),
                    description: &tag.description,
                    icon: &tag.icon,
                    art: Some(&tag.id),
                })
                .collect(),
            Page::Topic(Some(tag), _) => {
                let tag = &catalog.tags[tag];
                tag.items
                    .iter()
                    .filter(|item| Some(*item) != tag.main_item.as_ref())
                    .map(|item| item_card(catalog, item))
                    .collect()
            }
            Page::Topic(None, _) => catalog
                .scenes
                .iter()
                .enumerate()
                .map(|(i, entry)| Card {
                    action: Some(Open::Story(i, 0)),
                    title: entry.title.clone(),
                    description: "",
                    icon: entry.items.first().map_or("minecraft:book", String::as_str),
                    art: None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn main_card(self, catalog: &archive::Catalog) -> Option<Card<'_>> {
        let Page::Topic(Some(tag), _) = self.page else {
            return None;
        };
        Some(item_card(catalog, catalog.tags[tag].main_item.as_deref()?))
    }

    fn guides(self) -> impl Iterator<Item = (usize, &'static super::scene::Scene)> {
        SCENES
            .iter()
            .enumerate()
            .filter(move |(_, scene)| scene.category == self.category)
    }
}

enum Open {
    Tag(usize),
    Story(usize, usize),
}

struct Card<'a> {
    action: Option<Open>,
    title: String,
    description: &'a str,
    icon: &'a str,
    art: Option<&'a str>,
}

fn subject(catalog: &archive::Catalog, showing: Showing) -> Option<&str> {
    let (scene, item) = showing.subject.unwrap_or((showing.index, 0));
    catalog
        .scenes
        .get(scene)?
        .items
        .get(item)
        .map(String::as_str)
}

#[derive(Clone, Copy)]
pub struct Layout([f32; 2]);

impl Layout {
    pub fn new(size: [f32; 2]) -> Self {
        Self(size)
    }

    fn scene(self) -> [f32; 4] {
        [0.0, 0.0, self.0[0], self.0[1]]
    }

    pub fn can_orbit(self, at: [f32; 2]) -> bool {
        contains(
            [
                61.0,
                61.0,
                (self.0[0] - 82.0).max(0.0),
                (self.0[1] - 117.0).max(0.0),
            ],
            at,
        )
    }

    fn control(self, slot: usize) -> [f32; 4] {
        let [w, h] = self.0;
        let x = [-96.0, -38.0, -10.0, 18.0, 76.0]
            .get(slot)
            .map_or(w - 51.0, |dx| w * 0.5 + dx);
        [x, h - 51.0, 20.0, 20.0]
    }

    fn back(self) -> [f32; 4] {
        [31.0, self.0[1] - 51.0, 20.0, 20.0]
    }

    fn timeline(self) -> [f32; 4] {
        let w = self.0[0].min(220.0);
        [(self.0[0] - w) * 0.5, self.0[1] - 29.0, w, 21.0]
    }

    fn tag(self, slot: usize) -> [f32; 4] {
        [31.0, 81.0 + 30.0 * slot as f32, 20.0, 20.0]
    }

    fn tag_capacity(self) -> usize {
        ((self.0[1] - 136.0) / 30.0).max(0.0).ceil() as usize
    }

    fn next(self) -> [f32; 4] {
        let [w, h] = self.0;
        [
            w * 0.5 + 40.0,
            h - 90.0,
            (w * 0.5 - 53.0).clamp(1.0, 180.0),
            31.0,
        ]
    }

    fn extra(self, right: bool) -> [f32; 4] {
        [
            if right { self.0[0] - 96.0 } else { 8.0 },
            self.0[1] - 20.0,
            88.0,
            14.0,
        ]
    }

    fn page(self, next: bool) -> [f32; 4] {
        [
            self.0[0] * 0.5 + if next { 80.0 } else { -100.0 },
            self.0[1] - 38.0,
            20.0,
            20.0,
        ]
    }

    fn tab(self, i: usize) -> [f32; 4] {
        [self.0[0] * 0.5 - 104.0 + 70.0 * i as f32, 72.0, 66.0, 16.0]
    }

    fn grid(self, page: Page, count: usize) -> Grid {
        let [width, height] = self.0;
        let original = matches!(page, Page::Index(_) | Page::Topic(Some(_), _));
        let step = if original { 36.0 } else { 28.0 };
        let columns = ((width - 24.0) / step).floor().max(1.0) as usize;
        let cap_rows = if matches!(page, Page::Index(_)) { 1 } else { 3 };
        let center = match page {
            Page::Index(_) => 154.0,
            Page::Topic(Some(_), _) => (height * 0.15 + 85.0).floor(),
            _ => height * 0.52,
        };
        let capacity = columns * cap_rows;
        let page = match page {
            Page::Index(i) | Page::Topic(_, i) => i,
            _ => 0,
        };
        let count_on_page = count.saturating_sub(page * capacity).min(capacity);
        Grid {
            width,
            center,
            step,
            rows: count_on_page.div_ceil(columns.min(11)).clamp(1, cap_rows),
            capacity,
            pages: count.div_ceil(capacity).max(1),
            count: count_on_page,
        }
    }
}

struct Grid {
    width: f32,
    center: f32,
    step: f32,
    rows: usize,
    capacity: usize,
    pages: usize,
    count: usize,
}

impl Grid {
    fn tile(&self, mut i: usize) -> [f32; 4] {
        let mut row = 0;
        let mut columns = self.row_size(row);
        while columns > 0 && i >= columns && row + 1 < self.rows {
            i -= columns;
            row += 1;
            columns = self.row_size(row);
        }
        [
            self.width.floor() / 2.0 - (self.step * columns as f32 + 28.0 - self.step) / 2.0
                + 4.0
                + self.step * i as f32,
            self.center - self.area_height() / 2.0 + 4.0 + self.step * row as f32,
            20.0,
            20.0,
        ]
    }

    fn row_size(&self, row: usize) -> usize {
        self.count / self.rows + usize::from(row < self.count % self.rows)
    }

    fn area_height(&self) -> f32 {
        self.step * (self.rows - 1) as f32 + 28.0
    }

    fn main_tile(&self) -> [f32; 4] {
        [self.tile(0)[0] - 52.0, self.center - 10.0, 20.0, 20.0]
    }
}

fn item_card<'a>(catalog: &archive::Catalog, item: &'a str) -> Card<'a> {
    Card {
        action: catalog.for_item(item).and_then(|scene| {
            let index = catalog.scenes[scene]
                .items
                .iter()
                .position(|id| id == item)?;
            Some(Open::Story(scene, index))
        }),
        title: item_title(item),
        description: "",
        icon: item,
        art: None,
    }
}

pub fn contains([x, y, w, h]: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= x && at[0] < x + w && at[1] >= y && at[1] < y + h
}

/// The marker the pointer is on (`rules::ponder_timeline`: the bar's markers are drawn over its inside, `width - 2`
/// pixels, and the pointer is read over the same, so a click exactly on a marker takes it).
fn hovered_mark(layout: Layout, mouse: [f32; 2], duration: f32, markers: &[f32]) -> Option<f32> {
    let [x, _, width, _] = layout.timeline();
    if !contains(layout.timeline(), mouse) {
        return None;
    }
    create_kinetics::rules::ponder_timeline::hovered_mark(x, width, mouse[0], duration, markers)
}

pub fn draw(
    host: &dyn Host,
    batch: &mut HudBatch,
    layout: Layout,
    viewer: Viewer,
    mouse: [f32; 2],
    now: Instant,
    archive: Archive<'_>,
    draw_local: impl FnOnce(&mut HudBatch, Showing, [f32; 4], Frame<'_>, f32),
) {
    let [w, h] = layout.0;
    host.apply_screen_blur();
    host.fill(batch, [0.0, 0.0, w, h], 0xB510_1010);
    let rect = layout.scene();
    match viewer.page {
        Page::Scene(showing) => {
            let scene = &SCENES[showing.index];
            let tick = showing.tick(now, scene.duration(), viewer.comfy, None);
            let frame = scene.frame(tick);
            let caption = frame.step.caption;
            draw_local(batch, showing, rect, frame, tick);
            caption_box(host, batch, layout, caption, None, None);
            scene_hud(
                host,
                batch,
                layout,
                viewer,
                showing,
                mouse,
                archive,
                scene.title,
                scene.items[0],
                tick,
                scene.duration(),
                &scene.markers().collect::<Vec<_>>(),
            );
        }
        Page::Imported(showing) => {
            let Some(catalog) = archive.catalog else {
                return;
            };
            let entry = &catalog.scenes[showing.index];
            let Some(scene) = archive.scene else {
                heading(
                    host,
                    batch,
                    [21.0, 21.0],
                    "Scene file unavailable",
                    "",
                    "minecraft:book",
                    None,
                );
                symbol(
                    host,
                    batch,
                    layout.control(2),
                    1,
                    "Close",
                    mouse,
                    true,
                    false,
                );
                return;
            };
            let tick = showing.tick(now, scene.duration(), viewer.comfy, Some(scene));
            let frame = scene.frame(tick);
            let (showing, exiting) = showing.animation(now);
            if let (Some(scene), Some((old, tick))) = (archive.outgoing, exiting) {
                let tick = tick.min(scene.duration());
                scene.draw(host, batch, rect, old, &scene.frame(tick), tick);
            }
            scene.draw(host, batch, rect, showing, &frame, tick);
            if showing.inspect.is_none() {
                let target = scene.caption_target(rect, showing, &frame);
                caption_box(
                    host,
                    batch,
                    layout,
                    frame.caption,
                    frame.caption_style,
                    target,
                );
            }
            scene_hud(
                host,
                batch,
                layout,
                viewer,
                showing,
                mouse,
                archive,
                &entry.title,
                subject(catalog, showing).unwrap_or("minecraft:book"),
                tick,
                scene.duration(),
                scene.markers(),
            );
            let hovered_tag = viewer
                .tags(catalog, showing)
                .take(layout.tag_capacity())
                .enumerate()
                .any(|(slot, _)| contains(layout.tag(slot), mouse));
            if showing.inspect.is_some() && mouse[1] < h - 80.0 && !hovered_tag {
                if let Some(id) = scene.inspected_block(rect, showing, &frame, mouse) {
                    tooltip(host, batch, layout, mouse, &item_title(id), "");
                } else {
                    let label = archive
                        .label(
                            "identify_mode",
                            "Identify mode active.\nUnpause with [%1$s]",
                        )
                        .replace("%1$s", "Q");
                    let (title, rest) = label.split_once('\n').unwrap_or((&label, ""));
                    tooltip(host, batch, layout, mouse, title, rest);
                }
            } else if showing.tick(now, f32::MAX, viewer.comfy, Some(scene))
                >= scene.duration() + 50.0
                && let Some(next) = viewer.adjacent(showing, true, archive.catalog)
            {
                let rect @ [x, y, w, _] = layout.next();
                border(host, batch, rect, contains(rect, mouse), false);
                host.text(
                    batch,
                    archive.label("next_up", "Up Next:"),
                    [x + 4.0, y + 3.0],
                    MUTED,
                    false,
                );
                for (i, line) in wrap(host, &catalog.scenes[next].title, w - 8.0)
                    .iter()
                    .take(2)
                    .enumerate()
                {
                    host.text(
                        batch,
                        line,
                        [x + 4.0, y + 13.0 + i as f32 * 9.0],
                        INK,
                        false,
                    );
                }
            }
        }
        Page::Library => {
            heading(
                host,
                batch,
                [w * 0.5 - 120.0, 30.0],
                "Horizon tutorials",
                "",
                "minecraft:book",
                Some("logo"),
            );
            for (i, (category, label)) in CATEGORIES.into_iter().enumerate() {
                label_button(
                    host,
                    batch,
                    layout.tab(i),
                    label,
                    mouse,
                    category == viewer.category,
                );
            }
            let entries: Vec<_> = viewer.guides().collect();
            let grid = layout.grid(viewer.page, entries.len());
            for (slot, (_, scene)) in entries.into_iter().enumerate() {
                let rect = grid.tile(slot);
                item_tile(host, batch, rect, scene.items[0], None, mouse, false);
                if contains(rect, mouse) {
                    tooltip(host, batch, layout, mouse, scene.title, "");
                }
            }
        }
        Page::Index(page) | Page::Topic(_, page) => {
            let Some(catalog) = archive.catalog else {
                return;
            };
            let cards = viewer.cards(catalog);
            let grid = layout.grid(viewer.page, cards.len());
            match viewer.page {
                Page::Index(_) => {
                    heading(
                        host,
                        batch,
                        [w * 0.5 - 120.0, 30.0],
                        archive.label("welcome", "Welcome to Ponder"),
                        "",
                        "minecraft:book",
                        Some("logo"),
                    );
                    centered_box(host, batch, w * 0.5, 80.0_f32.min(h * 0.34), w * 0.5,
                        archive.label("index_description", "Click one of the icons below to learn about its associated Items and Blocks"));
                    let title = archive
                        .label("categories", "Available Categories for %1$s")
                        .replace("%1$s", "Create");
                    centered_box(
                        host,
                        batch,
                        w * 0.5,
                        grid.tile(0)[1] - 22.0,
                        w * 0.85,
                        &title,
                    );
                    label_button(host, batch, layout.extra(false), "All scenes", mouse, false);
                    label_button(
                        host,
                        batch,
                        layout.extra(true),
                        "Horizon tutorials",
                        mouse,
                        false,
                    );
                }
                Page::Topic(Some(i), _) => {
                    let tag = &catalog.tags[i];
                    heading(
                        host,
                        batch,
                        [w * 0.5 - 99.0, h * 0.15 - 19.0],
                        &tag.title,
                        archive.label("pondering_tag", "Pondering about..."),
                        &tag.icon,
                        Some(&tag.id),
                    );
                    let y = (grid.center + 14.0 * grid.rows as f32 + 13.0)
                        .min(h - 66.0);
                    centered_box(host, batch, w * 0.5, y, w * 0.45, &tag.description);
                }
                _ => heading(
                    host,
                    batch,
                    [w * 0.5 - 120.0, 30.0],
                    archive.label("index_title", "Ponder Index"),
                    "",
                    "minecraft:book",
                    Some("logo"),
                ),
            }
            let mut hovered = None;
            if !cards.is_empty() {
                let x = grid.tile(0)[0] - 23.0;
                streak(
                    host,
                    batch,
                    [
                        x,
                        grid.center - grid.area_height() / 2.0 - 5.0,
                        w - 2.0 * x,
                        grid.area_height() + 10.0,
                    ],
                );
            }
            if let Some(card) = viewer.main_card(catalog) {
                let rect = grid.main_tile();
                item_tile(
                    host,
                    batch,
                    rect,
                    card.icon,
                    None,
                    mouse,
                    card.action.is_none(),
                );
                if contains(rect, mouse) {
                    hovered = Some(card);
                }
            }
            for (slot, card) in cards
                .into_iter()
                .skip(page * grid.capacity)
                .take(grid.capacity)
                .enumerate()
            {
                let rect = grid.tile(slot);
                item_tile(
                    host,
                    batch,
                    rect,
                    card.icon,
                    card.art,
                    mouse,
                    card.action.is_none(),
                );
                if contains(rect, mouse) {
                    hovered = Some(card);
                }
            }
            if grid.pages > 1 {
                symbol(
                    host,
                    batch,
                    layout.page(false),
                    0,
                    "Previous Page",
                    mouse,
                    page > 0,
                    false,
                );
                symbol(
                    host,
                    batch,
                    layout.page(true),
                    2,
                    "Next Page",
                    mouse,
                    page + 1 < grid.pages,
                    false,
                );
                centered_box(
                    host,
                    batch,
                    w * 0.5,
                    h - 32.0,
                    120.0,
                    &format!("Page {}/{}", page + 1, grid.pages),
                );
            }
            if let Some(card) = hovered {
                tooltip(host, batch, layout, mouse, &card.title, card.description)
            }
        }
    }
    if let Some(previous) = viewer.previous[0] {
        let at = layout.back();
        let label = archive.label("think_back", "Think Back");
        if let (Page::Topic(Some(i), _), Some(catalog)) = (previous, archive.catalog) {
            let tag = &catalog.tags[i];
            item_tile(host, batch, at, &tag.icon, Some(&tag.id), mouse, false);
            if contains(at, mouse) {
                tooltip(host, batch, layout, mouse, label, "");
            }
        } else {
            symbol(host, batch, at, 3, label, mouse, true, false);
        }
    }
}

fn scene_hud(
    host: &dyn Host,
    batch: &mut HudBatch,
    layout: Layout,
    viewer: Viewer,
    showing: Showing,
    mouse: [f32; 2],
    archive: Archive<'_>,
    title: &str,
    icon: &str,
    tick: f32,
    duration: f32,
    markers: &[f32],
) {
    heading(
        host,
        batch,
        [21.0, 21.0],
        title,
        archive.label("pondering", "Pondering about..."),
        icon,
        None,
    );
    if let (Page::Imported(_), Some(catalog)) = (viewer.page, archive.catalog) {
        for (slot, i) in viewer
            .tags(catalog, showing)
            .take(layout.tag_capacity())
            .enumerate()
        {
            let tag = &catalog.tags[i];
            let rect = layout.tag(slot);
            item_tile(host, batch, rect, &tag.icon, Some(&tag.id), mouse, false);
            if contains(rect, mouse) {
                tooltip(host, batch, layout, mouse, &tag.title, &tag.description)
            }
        }
    }
    let [w, h] = layout.0;
    for sign in [-1.0_f32, 1.0] {
        let x = w * 0.5 + sign * 40.0;
        host.quad(
            batch,
            [
                [x - sign * 20.0, h - 51.0],
                [x, h - 51.0],
                [x + sign * 6.0, h - 41.0],
                [x, h - 31.0],
            ],
            0x3839_384C,
        );
    }
    let previous = viewer.adjacent(showing, false, archive.catalog).is_some();
    let next = viewer.adjacent(showing, true, archive.catalog).is_some();
    for (slot, icon, label, enabled, selected) in [
        (
            0,
            3,
            archive.label("identify", "Identify"),
            matches!(viewer.page, Page::Imported(_)),
            showing.inspect.is_some(),
        ),
        (
            1,
            0,
            archive.label("previous", "Previous Scene"),
            previous,
            false,
        ),
        (2, 1, archive.label("close", "Close"), true, false),
        (3, 2, archive.label("next", "Next Scene"), next, false),
        (4, 4, archive.label("replay", "Replay"), true, false),
        (
            5,
            6,
            archive.label("slow_text", "Comfy Reading"),
            true,
            viewer.comfy,
        ),
    ] {
        symbol(
            host,
            batch,
            layout.control(slot),
            icon,
            label,
            mouse,
            enabled,
            selected,
        );
    }
    let [x, y, width, _] = layout.timeline();
    border(host, batch, [x, y, width, 3.0], false, false);
    host.fill(
        batch,
        [x + 1.0, y + 1.0, (width - 2.0) * tick / duration, 2.0],
        0x80AA_AADD,
    );
    let selected = hovered_mark(layout, mouse, duration, markers);
    for mark in markers
        .iter()
        .copied()
        .filter(|&mark| mark > 0.0 && mark < duration)
        .chain(selected.filter(|&mark| mark == 0.0 || mark == duration))
    {
        let pos = create_kinetics::rules::ponder_timeline::mark_x(x, width, mark, duration);
        if selected == Some(mark) {
            host.fill(batch, [pos, y - 3.0, 2.0, 9.0], HOT);
            host.text(
                batch,
                if tick < mark { ">" } else { "<" },
                [pos + 4.0, y + 8.0],
                INK,
                false,
            );
        } else {
            host.fill(batch, [pos, y - 2.0, 2.0, 5.0], EDGE);
        }
    }
}

fn caption_box(
    host: &dyn Host,
    batch: &mut HudBatch,
    layout: Layout,
    caption: &str,
    style: Option<&CaptionStyle>,
    target: Option<[f32; 2]>,
) {
    if caption.is_empty() {
        return;
    }
    let [w, h] = layout.0;
    let [tx, ty] = target.unwrap_or([
        w * 0.5,
        (h - 200.0) * 0.5 + style.and_then(|s| s.y).unwrap_or(100.0) - 8.0,
    ]);
    let ratio = ((h * 0.5 - ty - 10.0) / 100.0).powi(2);
    let mut anchor_x = w * (0.75 - ratio * 0.125);
    if style.is_some_and(|s| s.near) {
        anchor_x = anchor_x.min(tx + 50.0);
    }
    let x = (anchor_x - 10.0).clamp(7.0, (w - 118.0).max(7.0));
    let lines = wrap(host, caption, (w - x - 12.0).clamp(1.0, 180.0));
    let width = lines
        .iter()
        .fold(0.0_f32, |max, line| max.max(host.text_width(line)))
        + 8.0;
    let height = lines.len() as f32 * 9.0 + 5.0;
    let y = ty.clamp(6.0, (h - height - 58.0).max(6.0));
    let palette = match style.and_then(|s| s.color.as_deref()) {
        Some("red") => [255, 93, 108],
        Some("green") => [140, 186, 81],
        Some("blue") => [95, 108, 175],
        Some("slow") => [34, 255, 34],
        Some("medium") => [0, 132, 255],
        Some("fast") => [255, 85, 255],
        Some("input") => [127, 205, 224],
        Some("output") => [221, 193, 102],
        _ => [238, 238, 238],
    };
    let [r, g, b] = std::array::from_fn(|i| (([255, 255, 221][i] + palette[i]) / 2) as u8);
    let tint = u32::from_be_bytes([255, r, g, b]);
    if target.is_some() {
        let start = tx.clamp(0.0, w);
        let end = x + 4.0;
        let left = start.min(end);
        host.fill(batch, [left, y, (start - end).abs(), 1.0], tint);
        host.fill(
            batch,
            [left, y + 1.0, (start - end).abs(), 1.0],
            0xFF49_4949,
        );
    }
    host.fill(
        batch,
        [x - 1.0, y + 2.0, width + 2.0, height + 2.0],
        0x607A_6000,
    );
    host.fill(batch, [x, y + 3.0, width, height], 0xFF00_0000);
    for (i, line) in lines.iter().enumerate() {
        host.text(
            batch,
            line,
            [x + 4.0, y + 6.0 + i as f32 * 9.0],
            tint,
            false,
        );
    }
}

fn heading(
    host: &dyn Host,
    batch: &mut HudBatch,
    [x, y]: [f32; 2],
    title: &str,
    detail: &str,
    item: &str,
    art: Option<&str>,
) {
    let limit = (batch.width - x - 47.0).clamp(1.0, 180.0);
    let lines = wrap(host, title, limit);
    let height = if detail.is_empty() {
        35.0
    } else {
        26.0 + lines.len() as f32 * 9.0
    };
    streak(
        host,
        batch,
        [
            x + 34.0,
            y - 2.0,
            (batch.width - x - 34.0).min(280.0),
            height,
        ],
    );
    border(host, batch, [x, y, 30.0, 30.0], false, false);
    item_image(host, batch, [x - 1.0, y - 1.0], 32.0, item, art);
    if !detail.is_empty() {
        host.text(batch, detail, [x + 38.0, y + 4.0], MUTED, false);
    }
    let large = art == Some("logo")
        && lines.len() == 1
        && host.text_width(title) * 2.0 <= batch.width - x - 45.0;
    let at = [
        x + 38.0,
        y + if detail.is_empty() {
            if large { 8.0 } else { 11.0 }
        } else {
            18.0
        },
    ];
    let start = batch.vertices.len();
    for (i, line) in lines.iter().enumerate() {
        host.text(batch, line, [at[0], at[1] + i as f32 * 9.0], INK, false);
    }
    if large {
        batch.scale_since(start, at[0], at[1], 2.0, 2.0);
    }
}

fn centered_box(host: &dyn Host, batch: &mut HudBatch, center: f32, y: f32, max: f32, text: &str) {
    let lines = wrap(host, text, max);
    let w = lines
        .iter()
        .fold(0.0_f32, |width, line| width.max(host.text_width(line)))
        + 8.0;
    border(
        host,
        batch,
        [center - w * 0.5, y - 3.0, w, lines.len() as f32 * 9.0 + 6.0],
        false,
        false,
    );
    for (i, line) in lines.iter().enumerate() {
        host.text(
            batch,
            line,
            [center - host.text_width(line) * 0.5, y + i as f32 * 9.0],
            INK,
            false,
        );
    }
}

fn item_tile(
    host: &dyn Host,
    batch: &mut HudBatch,
    rect: [f32; 4],
    item: &str,
    art: Option<&str>,
    mouse: [f32; 2],
    disabled: bool,
) {
    border(host, batch, rect, contains(rect, mouse), disabled);
    if art.is_some() {
        item_image(host, batch, [rect[0], rect[1]], 20.0, item, art);
    } else {
        item_image(
            host,
            batch,
            [rect[0] - 2.0, rect[1] - 2.0],
            24.0,
            item,
            None,
        );
    }
}

fn item_image(
    host: &dyn Host,
    batch: &mut HudBatch,
    at: [f32; 2],
    size: f32,
    item: &str,
    art: Option<&str>,
) {
    let sprite = match art {
        Some("logo") => Some(("create:gui/ponder/logo", 32.0)),
        _ => None,
    };
    if let Some((name, pixels)) = sprite
        && host.sprite(
            batch,
            name,
            [0.0, 0.0, pixels, pixels],
            [at[0], at[1], size, size],
        )
    {
        return;
    }
    if !host.item(batch, item, at, size) {
        host.item(batch, "minecraft:book", at, size);
    }
}

fn symbol(
    host: &dyn Host,
    batch: &mut HudBatch,
    rect: [f32; 4],
    column: u8,
    label: &str,
    mouse: [f32; 2],
    active: bool,
    selected: bool,
) {
    let hover = contains(rect, mouse);
    border(host, batch, rect, (hover && active) || selected, !active);
    let [x, y, _, _] = rect;
    if !host.sprite(
        batch,
        WIDGETS,
        [f32::from(column) * 16.0, 32.0, 16.0, 16.0],
        [x + 2.0, y + 2.0, 16.0, 16.0],
    ) {
        let glyph = match column {
            0 => "<",
            1 => "x",
            2 => ">",
            3 => "?",
            4 => "R",
            _ => "~",
        };
        host.text(batch, glyph, [x + 7.0, y + 6.0], INK, false);
    }
    if !active {
        host.fill(batch, [x + 1.0, y + 1.0, 18.0, 18.0], 0x6A00_0000);
    }
    if hover {
        let width = host.text_width(label);
        let left = (x + 10.0 - width * 0.5).clamp(1.0, (batch.width - width - 1.0).max(1.0));
        host.text(
            batch,
            label,
            [left, (y + 35.0).min(batch.height - 11.0)],
            MUTED,
            false,
        );
    }
}

fn label_button(
    host: &dyn Host,
    batch: &mut HudBatch,
    rect: [f32; 4],
    label: &str,
    mouse: [f32; 2],
    active: bool,
) {
    border(host, batch, rect, active || contains(rect, mouse), false);
    let [x, y, w, h] = rect;
    host.text(
        batch,
        label,
        [x + (w - host.text_width(label)) * 0.5, y + (h - 8.0) * 0.5],
        INK,
        false,
    );
}

fn border(
    host: &dyn Host,
    batch: &mut HudBatch,
    [x, y, w, h]: [f32; 4],
    highlight: bool,
    disabled: bool,
) {
    let color = if disabled {
        0x5090_9090
    } else if highlight {
        HOT
    } else {
        EDGE
    };
    let bottom = (color & 0x00FF_FFFF) | ((color >> 25) << 24);
    host.fill(batch, [x, y, w, h], 0xFF00_0000);
    host.fill(batch, [x, y, w, 1.0], color);
    host.fill(batch, [x, y + h - 1.0, w, 1.0], bottom);
    for side in [x, x + w - 1.0] {
        let body = (h - 2.0).max(0.0) * 0.5;
        host.fill(batch, [side, y + 1.0, 1.0, body], color);
        host.fill(batch, [side, y + 1.0 + body, 1.0, body], bottom);
    }
}

fn streak(host: &dyn Host, batch: &mut HudBatch, [x, y, w, h]: [f32; 4]) {
    for i in 0..16 {
        let t = i as f32 / 16.0;
        let end = if t < 0.5 {
            158.0 - 64.0 * t
        } else {
            126.0 * ((1.0 - t) * 2.0).powi(3)
        };
        host.fill(
            batch,
            [x + t * w, y, w / 16.0, h],
            (end as u32) << 24 | 0x0010_1010,
        );
    }
}

fn tooltip(
    host: &dyn Host,
    batch: &mut HudBatch,
    layout: Layout,
    mouse: [f32; 2],
    title: &str,
    description: &str,
) {
    let mut lines = vec![Text::literal(title)];
    lines.extend(
        wrap(host, description, 170.0)
            .into_iter()
            .map(Text::literal),
    );
    let x = mouse[0].min(layout.0[0] - 72.0).max(0.0);
    let y = mouse[1]
        .min(layout.0[1] - 19.0 - lines.len() as f32 * 9.0)
        .max(0.0);
    host.gui_tooltip(batch, &lines, [x, y]);
}

fn item_title(id: &str) -> String {
    id.rsplit(':')
        .next()
        .unwrap_or(id)
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn wrap(host: &dyn Host, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && host.text_width(&next) > width {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

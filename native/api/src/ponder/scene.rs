//! Ponder's storyboard and playback, independent of the game and its clock.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type Pos = [i32; 3];
pub type Properties = &'static [(&'static str, &'static str)];
pub type State = (&'static str, Properties);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
    pub index: usize,
    pub item: Arc<str>,
    pub imported: bool,
}

const HOLD_TIME: Duration = Duration::from_millis(350);

#[derive(Default)]
pub struct Hold(Option<(Topic, Instant)>);

impl Hold {
    pub fn update(&mut self, hovering: Option<Topic>, now: Instant) -> Option<Topic> {
        let Some(topic) = hovering else {
            self.0 = None;
            return None;
        };
        let since = self
            .0
            .as_ref()
            .filter(|(held, _)| held == &topic)
            .map_or(now, |(_, since)| *since);
        self.0 = Some((topic.clone(), since));
        (now.duration_since(since) >= HOLD_TIME).then_some(topic)
    }
}

#[derive(Clone, Copy)]
pub struct Block {
    pub pos: Pos,
    pub state: State,
    pub rpm: Option<f32>,
}

impl Block {
    pub const fn new(pos: Pos, id: &'static str, properties: Properties) -> Self {
        Self {
            pos,
            state: (id, properties),
            rpm: None,
        }
    }

    pub const fn rotating(mut self, rpm: f32) -> Self {
        self.rpm = Some(rpm);
        self
    }
}

pub struct Item {
    pub id: &'static str,
    pub anchor: Pos,
    pub from: [f32; 3],
    pub to: [f32; 3],
}

pub struct Step {
    pub ticks: u16,
    pub caption: &'static str,
    pub blocks: &'static [Block],
    pub item: Option<Item>,
    pub press: Option<Pos>,
}

impl Step {
    pub const fn new(ticks: u16, caption: &'static str, blocks: &'static [Block]) -> Self {
        Self {
            ticks,
            caption,
            blocks,
            item: None,
            press: None,
        }
    }

    pub const fn item(
        mut self,
        id: &'static str,
        anchor: Pos,
        from: [f32; 3],
        to: [f32; 3],
    ) -> Self {
        self.item = Some(Item {
            id,
            anchor,
            from,
            to,
        });
        self
    }

    pub const fn press(mut self, pos: Pos) -> Self {
        self.press = Some(pos);
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Rotation,
    Transport,
    Processing,
}

pub struct Scene {
    pub title: &'static str,
    pub category: Category,
    pub items: &'static [&'static str],
    pub steps: &'static [Step],
}

pub struct Frame<'a> {
    pub step: &'a Step,
    pub progress: f32,
    pub blocks: BTreeMap<Pos, (Block, f32)>,
}

impl Scene {
    pub fn duration(&self) -> f32 {
        self.steps.iter().map(|step| f32::from(step.ticks)).sum()
    }

    pub fn markers(&self) -> impl Iterator<Item = f32> + '_ {
        self.steps.iter().scan(0.0, |start, step| {
            let marker = *start;
            *start += f32::from(step.ticks);
            Some(marker)
        })
    }

    pub fn frame(&self, tick: f32) -> Frame<'_> {
        let tick = tick.clamp(0.0, self.duration());
        let mut blocks = BTreeMap::<Pos, (Block, f32, f32)>::new();
        let mut step = &self.steps[0];
        let mut started = 0.0;
        for (next, at) in self
            .steps
            .iter()
            .zip(self.markers())
            .take_while(|(_, at)| *at <= tick)
        {
            step = next;
            started = at;
            for &block in next.blocks {
                let angle = blocks
                    .get(&block.pos)
                    .filter(|(old, _, _)| old.state == block.state)
                    .map_or(0.0, |(old, angle, since)| angle + degrees(*old, at - since));
                blocks.insert(block.pos, (block, angle, at));
            }
        }
        Frame {
            step,
            progress: (tick - started) / f32::from(step.ticks),
            blocks: blocks
                .into_iter()
                .map(|(pos, (block, angle, since))| {
                    (pos, (block, angle + degrees(block, tick - since)))
                })
                .collect(),
        }
    }
}

fn degrees(block: Block, ticks: f32) -> f32 {
    block.rpm.unwrap_or(0.0) * 0.3 * ticks
}

#[derive(Clone, Copy)]
pub struct Playback {
    tick: f32,
    since: Option<Instant>,
}

impl Playback {
    pub fn new(now: Instant) -> Self {
        Self {
            tick: 0.0,
            since: Some(now),
        }
    }

    pub fn tick(self, now: Instant, duration: f32) -> f32 {
        self.tick_with(now, duration, |tick, elapsed| tick + elapsed)
    }

    pub fn tick_with(
        self,
        now: Instant,
        duration: f32,
        advance: impl FnOnce(f32, f32) -> f32,
    ) -> f32 {
        let elapsed = self
            .since
            .map_or(0.0, |since| now.duration_since(since).as_secs_f32());
        advance(self.tick, elapsed * 20.0).min(duration)
    }

    pub fn set(&mut self, tick: f32, now: Instant, playing: bool) {
        self.tick = tick;
        self.since = playing.then_some(now);
    }

    pub fn running(self) -> bool {
        self.since.is_some()
    }

    pub fn playing(self, now: Instant, duration: f32) -> bool {
        self.since.is_some() && self.tick(now, duration) < duration
    }

    pub fn toggle(&mut self, now: Instant, duration: f32) {
        let tick = self.tick(now, duration);
        if self.playing(now, duration) {
            self.seek(tick);
        } else {
            self.tick = if tick >= duration { 0.0 } else { tick };
            self.since = Some(now);
        }
    }

    pub fn seek(&mut self, tick: f32) {
        self.tick = tick;
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAFT: Block = Block::new([0, 1, 0], "create:shaft", &[("axis", "x")]);
    const SCENE: Scene = Scene {
        title: "Shaft",
        category: Category::Rotation,
        items: &["create:shaft"],
        steps: &[
            Step::new(20, "Start", &[SHAFT.rotating(10.0)]),
            Step::new(20, "Reverse", &[SHAFT.rotating(-10.0)]),
        ],
    };

    #[test]
    fn seeking_rebuilds_the_scene_and_preserves_the_angle_when_speed_changes() {
        let angle = |tick| SCENE.frame(tick).blocks[&SHAFT.pos].1;
        assert_eq!(angle(20.0), 60.0);
        assert_eq!(angle(30.0), 30.0);
        assert_eq!(angle(10.0), 30.0);
        assert_eq!(SCENE.frame(100.0).progress, 1.0);
    }

    #[test]
    fn pause_seek_resume_and_replay_use_the_same_timeline() {
        let start = Instant::now();
        let later = start + Duration::from_secs(1);
        let end = later + Duration::from_secs(10);
        let duration = SCENE.duration();
        let mut play = Playback::new(start);
        play.toggle(later, duration);
        assert_eq!(play.tick(end, duration), 20.0);
        play.seek(5.0);
        play.toggle(later, duration);
        assert_eq!(play.tick(end, duration), duration);
        play.toggle(end, duration);
        assert!(play.playing(end, duration));
        assert_eq!(play.tick(end, duration), 0.0);
    }

    #[test]
    fn changing_the_hovered_item_or_releasing_w_restarts_the_hold() {
        let now = Instant::now();
        let mut hold = Hold::default();
        let a = Topic {
            index: 0,
            item: "a".into(),
            imported: false,
        };
        let b = Topic {
            index: 0,
            item: "b".into(),
            imported: false,
        };
        assert_eq!(hold.update(Some(a), now), None);
        assert_eq!(hold.update(Some(b.clone()), now + HOLD_TIME), None);
        assert_eq!(hold.update(None, now + 2 * HOLD_TIME), None);
        assert_eq!(hold.update(Some(b.clone()), now + 3 * HOLD_TIME), None);
        assert_eq!(hold.update(Some(b.clone()), now + 4 * HOLD_TIME), Some(b));
    }
}

//! `CurvedTrackInteraction` and `CurvedTrackDestroyPacket` (content/trains/track/CurvedTrackInteraction.java,
//! CurvedTrackDestroyPacket.java): breaking a curve by holding attack on it, and picking it up with the Wrench.
//!
//! The client half is [`BreakProgress`], `clientTick`'s state: while the attack key is held on a picked curve the progress grows by
//! an eighth of what a block of the same hardness would (a flat eighth in creative), shows the crack stage on the track block that
//! holds it, and sends [`CurveDestroy`] when it is full; letting go for two ticks resets it. The server half is what
//! `CurvedTrackDestroyPacket.applySettings` decides: whether the sender may ([`may_destroy`]), and what a Wrench gives the player
//! ([`items_for_player`]).
//!
//! Known differences: Horizon has no client call for a crack, so the stage goes to the server in a [`CurveBreakStage`] and the server
//! sets it on the sender's behalf; the destroy particles (`spawnDestroyParticles`) are left out.

use super::bezier::BezierConnection;
use crate::Pos;

/// The ticks the attack may be let go of before the progress is lost (`breakTimeout = 2`).
pub const TIMEOUT: u32 = 2;

/// What the client does in a tick of `CurvedTrackInteraction.clientTick`, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// The block's hit sound, every fourth tick (volume `(volume + 1) / 8`, pitch halved).
    HitSound,
    /// A block-crack particle at the picked point, offset randomly by a quarter block.
    Particle,
    /// `destroyBlockProgress(player, pos, stage)`: stage -1 clears it.
    Crack(Pos, i32),
    /// `player.swing(MAIN_HAND)`.
    Swing,
    /// The progress is full: send a [`CurveDestroy`] for the picked curve (the crack is cleared right after).
    Destroy,
}

/// What `clientTick` reads of the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// `player.getAbilities().mayBuild`.
    pub may_build: bool,
    /// `mc.options.keyAttack.isDown()`.
    pub attack_down: bool,
    /// The track block entity that holds the picked curve (`result.blockEntity().getBlockPos()`), `None` without a pick.
    pub picked: Option<Pos>,
    /// `blockState.isAir()` at it.
    pub block_is_air: bool,
    /// `getAbilities().instabuild`.
    pub creative: bool,
    /// `blockState.getDestroyProgress(player, level, pos)` for a survival player.
    pub destroy_progress: f32,
}

/// `breakTicks`, `breakTimeout`, `breakProgress` and `breakPos`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BreakProgress {
    ticks: u32,
    timeout: u32,
    progress: f32,
    pos: Option<Pos>,
}

impl BreakProgress {
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// `resetBreakProgress`: clears the crack that was shown, if any.
    pub fn reset(&mut self, out: &mut Vec<Effect>) {
        if let Some(pos) = self.pos {
            out.push(Effect::Crack(pos, -1));
        }
        self.progress = 0.0;
        self.ticks = 0;
        self.pos = None;
    }

    /// `clientTick`.
    pub fn tick(&mut self, input: &Input) -> Vec<Effect> {
        let mut out = Vec::new();
        if !input.may_build {
            return out;
        }
        if let (true, Some(picked)) = (input.attack_down, input.picked) {
            self.pos = Some(picked);
            if input.block_is_air {
                self.reset(&mut out);
                return out;
            }
            if self.ticks % 4 == 0 {
                out.push(Effect::HitSound);
            }
            self.ticks += 1;
            self.timeout = TIMEOUT;
            self.progress += if input.creative { 0.125 } else { input.destroy_progress / 8.0 };
            out.push(Effect::Particle);
            out.push(Effect::Crack(picked, (self.progress * 10.0) as i32 - 1));
            out.push(Effect::Swing);
            if self.progress >= 1.0 {
                out.push(Effect::Destroy);
                self.reset(&mut out);
            }
            return out;
        }
        if self.timeout == 0 {
            return out;
        }
        self.timeout -= 1;
        if self.timeout > 0 {
            return out;
        }
        self.reset(&mut out);
        out
    }
}

// --- what the client sends --------------------------------------------------------------------------------------------------------

/// Client to server: the picked curve is broken (`CurvedTrackDestroyPacket`).
pub const CHANNEL_DESTROY: &str = "create:curve_destroy";
/// Client to server: the crack stage to show on a track block (stands in for `destroyBlockProgress`, which a client can't show alone).
pub const CHANNEL_STAGE: &str = "create:curve_break";

fn put_pos(out: &mut Vec<u8>, p: Pos) {
    for v in p {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

fn take_pos(data: &[u8], at: usize) -> Option<Pos> {
    let word = |i: usize| data.get(at + i * 4..at + i * 4 + 4).and_then(|b| b.try_into().ok()).map(i32::from_be_bytes);
    Some([word(0)?, word(1)?, word(2)?])
}

/// `CurvedTrackDestroyPacket(pos, targetPos, soundSource, wrench)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveDestroy {
    /// The track block entity that holds the curve.
    pub block: Pos,
    /// `targetPos`: the curve's key (its other end).
    pub curve: Pos,
    /// Where the sounds play (`BlockPos.containing(result.vec())`).
    pub sound_source: Pos,
    /// Picked up with the Wrench (the items go to the player, with a different sound) rather than broken.
    pub wrench: bool,
}

impl CurveDestroy {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.wrench)];
        for p in [self.block, self.curve, self.sound_source] {
            put_pos(&mut out, p);
        }
        out
    }

    pub fn decode(data: &[u8]) -> Option<CurveDestroy> {
        let wrench = *data.first()? != 0;
        Some(CurveDestroy { wrench, block: take_pos(data, 1)?, curve: take_pos(data, 13)?, sound_source: take_pos(data, 25)? })
    }
}

/// The crack on a track block: stage 0 to 9, or -1 for none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveBreakStage {
    pub block: Pos,
    pub stage: i32,
}

impl CurveBreakStage {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        put_pos(&mut out, self.block);
        out.extend_from_slice(&self.stage.to_be_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Option<CurveBreakStage> {
        let stage = i32::from_be_bytes(data.get(12..16)?.try_into().ok()?);
        Some(CurveBreakStage { block: take_pos(data, 0)?, stage: stage.clamp(-1, 9) })
    }
}

// --- what the server decides ------------------------------------------------------------------------------------------------------

/// `AllConfigs.server().trains.maxTrackPlacementLength` (default 32).
pub const MAX_TRACK_PLACEMENT_LENGTH: i32 = 32;
/// `Player.blockInteractionRange`.
pub const BLOCK_INTERACTION_RANGE: f64 = 4.5;
/// The eye height of a standing player.
pub const EYE_HEIGHT: f64 = 1.62;

/// `Player.canInteractWithBlock(pos, extra)`: the block's box is within `range + extra` of the eye.
pub fn can_interact_with_block(eye: [f64; 3], pos: Pos, extra: f64) -> bool {
    let mut distance_sqr = 0.0;
    for i in 0..3 {
        let lo = f64::from(pos[i]);
        let d = (lo - eye[i]).max(eye[i] - (lo + 1.0)).max(0.0);
        distance_sqr += d * d;
    }
    let reach = BLOCK_INTERACTION_RANGE + extra;
    distance_sqr < reach * reach
}

/// What the server asks of the sender of a [`CurveDestroy`] before it does anything: not a spectator or adventure player
/// (`BlockEntityConfigurationPacket.handle`), within `maxRange()` of the block (`maxTrackPlacementLength + 16`), and within
/// `maxTrackPlacementLength * 4` of it (`applySettings`).
pub fn may_destroy(game_mode: i32, eye: [f64; 3], block: Pos) -> bool {
    // 0 survival, 1 creative, 2 adventure, 3 spectator.
    matches!(game_mode, 0 | 1)
        && can_interact_with_block(eye, block, f64::from(MAX_TRACK_PLACEMENT_LENGTH + 16))
        && can_interact_with_block(eye, block, f64::from(MAX_TRACK_PLACEMENT_LENGTH * 4))
}

/// What a crack stage from a client may ask for: the same reach as the break itself.
pub fn may_show_stage(game_mode: i32, eye: [f64; 3], block: Pos) -> bool {
    matches!(game_mode, 0 | 1) && can_interact_with_block(eye, block, f64::from(MAX_TRACK_PLACEMENT_LENGTH + 16))
}

/// `addItemsToPlayer`: the stacks a Wrench gives for a curve, tracks first and then girders, 64 at a time (`true` for a girder).
pub fn items_for_player(bc: &BezierConnection) -> Vec<(bool, u32)> {
    let mut out = Vec::new();
    for (girder, mut left) in [(false, bc.track_item_cost()), (true, bc.girder_item_cost())] {
        while left > 0 {
            let n = left.min(64);
            out.push((girder, n as u32));
            left -= n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(attack: bool) -> Input {
        Input { may_build: true, attack_down: attack, picked: Some([3, 64, 3]), block_is_air: false, creative: false, destroy_progress: 0.04 }
    }

    #[test]
    fn holding_attack_on_a_curve_cracks_the_block_and_breaks_it_when_full() {
        let mut state = BreakProgress::default();
        let mut crack_stages = Vec::new();
        let mut destroyed_at = None;
        for tick in 0..400 {
            let effects = state.tick(&input(true));
            if tick == 0 {
                // The first tick plays the hit sound, a particle, shows a crack and swings.
                assert_eq!(effects[0], Effect::HitSound);
                assert!(effects.contains(&Effect::Particle) && effects.contains(&Effect::Swing));
            }
            for e in &effects {
                if let Effect::Crack(_, stage) = e {
                    crack_stages.push(*stage);
                }
                if *e == Effect::Destroy && destroyed_at.is_none() {
                    destroyed_at = Some(tick);
                }
            }
            if destroyed_at.is_some() {
                // Destroying clears the crack and the progress in the same tick.
                assert_eq!(effects.last(), Some(&Effect::Crack([3, 64, 3], -1)));
                assert_eq!(state.progress(), 0.0);
                break;
            }
        }
        // 0.04 / 8 = 0.005 a tick: 200 ticks for one.
        assert!(matches!(destroyed_at, Some(198..=200)), "{destroyed_at:?}");
        assert_eq!(crack_stages[0], -1, "the first crack is stage (int)(0.05) - 1");
        assert!(crack_stages.iter().any(|s| *s == 8));
    }

    #[test]
    fn creative_breaks_in_eight_ticks() {
        let mut state = BreakProgress::default();
        let mut i = input(true);
        i.creative = true;
        let mut ticks = 0;
        loop {
            ticks += 1;
            if state.tick(&i).contains(&Effect::Destroy) {
                break;
            }
            assert!(ticks < 20);
        }
        assert_eq!(ticks, 8);
    }

    #[test]
    fn the_hit_sound_comes_every_fourth_tick() {
        let mut state = BreakProgress::default();
        let sounds: Vec<bool> = (0..9).map(|_| state.tick(&input(true)).contains(&Effect::HitSound)).collect();
        assert_eq!(sounds, [true, false, false, false, true, false, false, false, true]);
    }

    #[test]
    fn letting_go_for_two_ticks_loses_the_progress() {
        let mut state = BreakProgress::default();
        for _ in 0..10 {
            state.tick(&input(true));
        }
        assert!(state.progress() > 0.0);
        // One tick of grace, then the crack is cleared.
        assert!(state.tick(&input(false)).is_empty());
        assert!(state.progress() > 0.0);
        assert_eq!(state.tick(&input(false)), vec![Effect::Crack([3, 64, 3], -1)]);
        assert_eq!(state.progress(), 0.0);
        // And nothing more happens while idle.
        assert!(state.tick(&input(false)).is_empty());
    }

    #[test]
    fn nothing_happens_to_a_player_who_may_not_build_or_to_air() {
        let mut state = BreakProgress::default();
        let mut i = input(true);
        i.may_build = false;
        assert!(state.tick(&i).is_empty());
        let mut i = input(true);
        i.block_is_air = true;
        assert_eq!(state.tick(&i), vec![Effect::Crack([3, 64, 3], -1)]);
        // No pick: the attack is a plain one.
        let mut i = input(true);
        i.picked = None;
        assert!(state.tick(&i).is_empty());
    }

    #[test]
    fn the_messages_round_trip() {
        let d = CurveDestroy { block: [1, -64, 3], curve: [-10, 70, 33], sound_source: [0, 65, 9], wrench: true };
        assert_eq!(CurveDestroy::decode(&d.encode()), Some(d));
        assert_eq!(CurveDestroy::decode(&d.encode()[..20]), None);
        let s = CurveBreakStage { block: [5, 5, 5], stage: 7 };
        assert_eq!(CurveBreakStage::decode(&s.encode()), Some(s));
        let wild = CurveBreakStage { block: [5, 5, 5], stage: 400 };
        assert_eq!(CurveBreakStage::decode(&wild.encode()).unwrap().stage, 9);
    }

    #[test]
    fn the_server_checks_the_senders_reach_and_mode() {
        let eye = [0.5, 65.62, 0.5];
        assert!(may_destroy(0, eye, [10, 64, 10]));
        assert!(may_destroy(1, eye, [40, 64, 0]));
        // Beyond maxTrackPlacementLength + 16 (48) plus the reach.
        assert!(!may_destroy(0, eye, [60, 64, 0]));
        assert!(!may_destroy(2, eye, [1, 64, 1]), "adventure");
        assert!(!may_destroy(3, eye, [1, 64, 1]), "spectator");
        assert!(may_show_stage(0, eye, [1, 64, 1]));
    }

    #[test]
    fn a_wrench_gives_the_tracks_and_girders_in_stacks() {
        let bc = BezierConnection::new(
            [[0, 64, 0], [40, 64, 0]],
            [[0.5, 64.0, 0.5], [40.5, 64.0, 0.5]],
            [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            [[0.0, 1.0, 0.0]; 2],
            true,
            true,
            "create:andesite",
        );
        let items = items_for_player(&bc);
        let tracks: u32 = items.iter().filter(|(g, _)| !*g).map(|(_, n)| n).sum();
        let girders: u32 = items.iter().filter(|(g, _)| *g).map(|(_, n)| n).sum();
        assert_eq!(tracks as usize, bc.track_item_cost());
        assert_eq!(girders as usize, bc.girder_item_cost());
        assert!(items.iter().all(|(_, n)| *n >= 1 && *n <= 64));
        assert!(items.iter().position(|(g, _)| *g) > items.iter().rposition(|(g, _)| !*g), "tracks come first");
    }
}

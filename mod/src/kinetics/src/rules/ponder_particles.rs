//! Particles in Ponder scenes: `effects.superGlue` (Create 6.0.10, `CreateSceneBuilder.EffectInstructions.superGlue`
//! and `SuperGlueItem.spawnParticles`) and `effects.emitParticles` (the scene library's `EmitParticlesInstruction`
//! with a `simpleParticleEmitter` or `particleEmitterWithinBlockSpace`, used by Create's fan, deployer, press, mixer,
//! cart assembler and spout scenes). No game types.
//!
//! What is Create's: the glue burst (40 particles on a full face, 15 on a partial one, on a ring about the face's
//! axis, each flying outward at 1/16 of a block a tick), and every call that feeds an emitter. What is not: the
//! particle's own motion. Vanilla's `Particle` and `BreakingItemParticle` / `SmokeParticle` / `SpitParticle` classes are
//! not in Create's source; their constants here (initial speed scatter, gravity, friction, lifetime) are the
//! well-known vanilla ones, written from memory. The player sees small flat squares, not the vanilla sprites.

/// A cheap seeded generator (SplitMix64), so a scene draws the same particles every time it is scrubbed to a tick.
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn seeded(parts: &[u64]) -> Self {
        let mut rng = Self(0x9E37_79B9_7F4A_7C15);
        for &part in parts {
            rng.0 ^= part.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            rng.next_u64();
        }
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `nextFloat`: in [0, 1).
    pub fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// What a particle looks like and how it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The glue's slime ball (`ItemParticleOption(ParticleTypes.ITEM, SLIME_BALL)`).
    Slime,
    /// Any other item or block break particle.
    Item,
    LargeSmoke,
    Spit,
    Redstone,
}

impl Kind {
    /// The `ParticleTypes` name a storyboard uses.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "slime" => Self::Slime,
            "item" | "block" => Self::Item,
            "large_smoke" | "smoke" => Self::LargeSmoke,
            "spit" => Self::Spit,
            "redstone" => Self::Redstone,
            _ => return None,
        })
    }

    /// An ARGB colour for the square drawn.
    pub const fn colour(self) -> u32 {
        match self {
            Self::Slime => 0xFF7E_BF6E,
            Self::Item => 0xFFC9_B38A,
            Self::LargeSmoke => 0xFF8A_8A8A,
            Self::Spit => 0xFFE9_F1EC,
            Self::Redstone => 0xFFE8_2D2D,
        }
    }

    /// Pixels a particle is across at the scene's usual scale: vanilla's `quadSize` times the block size.
    pub const fn size(self) -> f32 {
        match self {
            Self::Slime | Self::Item => 2.0,
            Self::Spit | Self::Redstone => 2.0,
            Self::LargeSmoke => 5.0,
        }
    }

    /// Per tick: downward acceleration (`gravity` x 0.04), and the factor the speed keeps (`friction`).
    const fn physics(self) -> (f64, f64) {
        match self {
            Self::Slime | Self::Item => (0.04, 0.98),
            Self::Spit => (0.04 * 0.5, 0.98),
            Self::LargeSmoke => (-0.004, 0.96),
            Self::Redstone => (0.0, 0.96),
        }
    }

    /// `Particle.lifetime` for a draw `r` in [0, 1): `(int)(4 / (r * 0.9 + 0.1))` for the item and block particles,
    /// `(int)(8 / (r * 0.8 + 0.2))` times the scale (2.5) for the large smoke.
    fn lifetime(self, r: f64) -> f32 {
        match self {
            Self::LargeSmoke => (((8.0 / (r * 0.8 + 0.2)) as i32) as f64 * 2.5).max(1.0).floor() as f32,
            Self::Redstone => 20.0,
            _ => ((4.0 / (r * 0.9 + 0.1)) as i32) as f32,
        }
    }
}

/// Where a particle starts and the motion it was given (`Level.addParticle(data, x, y, z, vx, vy, vz)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    pub pos: [f64; 3],
    pub motion: [f64; 3],
}

/// A particle ready to be placed at any age.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub kind: Kind,
    pub pos: [f64; 3],
    pub velocity: [f64; 3],
    pub lifetime: f32,
}

/// `VecHelper.axisAlingedPlaneOf`: (1, 1, 1) less the absolute normal.
fn plane_of(normal: [f64; 3]) -> [f64; 3] {
    let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
    normal.map(|v| 1.0 - (v / length).abs())
}

/// `VecHelper.rotate(vec, degrees, axis)` about the axis `axis` (0, 1 or 2).
fn rotate(v: [f64; 3], degrees: f64, axis: usize) -> [f64; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    match axis {
        0 => [v[0], v[1] * cos - v[2] * sin, v[2] * cos + v[1] * sin],
        1 => [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin],
        _ => [v[0] * cos - v[1] * sin, v[1] * cos + v[0] * sin, v[2]],
    }
}

/// `SuperGlueItem.spawnParticles(world, pos, direction, fullBlock)`: the particles of one burst, on the face of the
/// block at `pos` that `normal` points out of.
pub fn glue_burst(pos: [i32; 3], normal: [i32; 3], full_block: bool, rng: &mut Rng) -> Vec<Spawn> {
    let vec = normal.map(f64::from);
    let plane = plane_of(vec);
    let face: [f64; 3] = std::array::from_fn(|i| f64::from(pos[i]) + 0.5 + vec[i] * 0.5);
    let distance = if full_block { 1.0 } else { 0.25 + 0.25 * (rng.float() - 0.5) };
    let plane = plane.map(|v| v * distance);
    let axis = normal.iter().position(|&v| v != 0).unwrap_or(1);
    (0..if full_block { 40 } else { 15 })
        .map(|_| {
            let mut offset = rotate(plane, 360.0 * rng.float(), axis);
            let length = offset.iter().map(|v| v * v).sum::<f64>().sqrt();
            let motion = offset.map(|v| if length == 0.0 { 0.0 } else { v / length / 16.0 });
            if full_block {
                offset = offset.map(|v| v.clamp(-0.5, 0.5));
            }
            Spawn { pos: std::array::from_fn(|i| face[i] + offset[i]), motion }
        })
        .collect()
}

/// `EmitParticlesInstruction`: how many particles this tick, for `runs_per_tick` a tick (the whole part, and one more
/// with the chance of the fraction).
pub fn emission_runs(runs_per_tick: f32, rng: &mut Rng) -> u32 {
    let whole = runs_per_tick.floor();
    whole as u32 + u32::from(rng.float() < f64::from(runs_per_tick - whole))
}

/// `particleEmitterWithinBlockSpace`: a point anywhere in the block about `centre`.
pub fn within_block(centre: [f64; 3], rng: &mut Rng) -> [f64; 3] {
    centre.map(|v| v + rng.float() - 0.5)
}

/// Vanilla's `Particle` constructor with a given motion, as `BreakingItemParticle` and `SmokeParticle` use it: the
/// motion is scattered, scaled and added to the given one.
pub fn particle(kind: Kind, spawn: Spawn, rng: &mut Rng) -> Particle {
    let scatter = [0, 1, 2].map(|_| (rng.float() * 2.0 - 1.0) * 0.4);
    let speed = (rng.float() + rng.float() + 1.0) * 0.15;
    let length = scatter.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-9);
    let own = scatter.map(|v| v / length * speed * 0.4);
    // The break and smoke particles keep a tenth (smoke, a hundredth) of that and add the motion given.
    let keep = if kind == Kind::LargeSmoke { 0.01 } else { 0.1 };
    Particle {
        kind,
        pos: spawn.pos,
        velocity: std::array::from_fn(|i| own[i] * keep + spawn.motion[i]),
        lifetime: kind.lifetime(rng.float()),
    }
}

impl Particle {
    /// Where it is `age` ticks after it was added, or `None` once its lifetime is over. Each tick: `yd -= 0.04 *
    /// gravity`, move, scale the speed by the friction.
    pub fn at(&self, age: f32) -> Option<[f64; 3]> {
        if age < 0.0 || age >= self.lifetime {
            return None;
        }
        let (gravity, friction) = self.kind.physics();
        let whole = age.floor() as u32;
        let mut pos = self.pos;
        let mut velocity = self.velocity;
        for _ in 0..whole {
            velocity[1] -= gravity;
            for i in 0..3 {
                pos[i] += velocity[i];
                velocity[i] *= friction;
            }
        }
        let part = f64::from(age - age.floor());
        velocity[1] -= gravity * f64::from(u8::from(part > 0.0));
        Some(std::array::from_fn(|i| pos[i] + velocity[i] * part))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_face_bursts_into_40_particles_and_a_partial_one_into_15() {
        let mut rng = Rng::seeded(&[1]);
        assert_eq!(glue_burst([2, 1, 3], [0, 1, 0], true, &mut rng).len(), 40);
        assert_eq!(glue_burst([2, 1, 3], [0, 1, 0], false, &mut rng).len(), 15);
    }

    #[test]
    fn the_burst_stays_on_the_face_and_flies_outward_at_a_sixteenth_of_a_block() {
        let mut rng = Rng::seeded(&[7]);
        for spawn in glue_burst([2, 1, 3], [0, 1, 0], true, &mut rng) {
            // On the top face of the block, clamped to its edges.
            assert!((spawn.pos[1] - 2.0).abs() < 1e-9);
            assert!((2.0..=3.0).contains(&spawn.pos[0]) && (3.0..=4.0).contains(&spawn.pos[2]));
            let speed = spawn.motion.iter().map(|v| v * v).sum::<f64>().sqrt();
            assert!((speed - 1.0 / 16.0).abs() < 1e-9);
            assert_eq!(spawn.motion[1], 0.0);
        }
    }

    #[test]
    fn a_side_face_is_laid_in_the_plane_of_that_face() {
        let mut rng = Rng::seeded(&[3]);
        for spawn in glue_burst([0, 0, 0], [1, 0, 0], false, &mut rng) {
            assert!((spawn.pos[0] - 1.0).abs() < 1e-9);
            assert_eq!(spawn.motion[0], 0.0);
        }
    }

    #[test]
    fn emission_runs_are_the_whole_part_plus_a_chance_of_one() {
        let mut rng = Rng::seeded(&[9]);
        assert!((0..50).all(|_| emission_runs(3.0, &mut rng) == 3));
        let total: u32 = (0..2000).map(|_| emission_runs(0.5, &mut rng)).sum();
        assert!((800..1200).contains(&total));
    }

    #[test]
    fn a_particle_falls_slows_and_ends_with_its_lifetime() {
        let mut rng = Rng::seeded(&[5]);
        let p = particle(Kind::Slime, Spawn { pos: [0.0; 3], motion: [0.0625, 0.0, 0.0] }, &mut rng);
        assert!((4.0..=40.0).contains(&p.lifetime));
        let early = p.at(1.0).unwrap();
        let later = p.at(3.0).unwrap();
        assert!(later[1] < early[1] && later[0] > early[0]);
        assert_eq!(p.at(p.lifetime), None);
        // Smoke rises.
        let smoke = particle(Kind::LargeSmoke, Spawn { pos: [0.0; 3], motion: [0.0; 3] }, &mut rng);
        assert!(smoke.at(5.0).unwrap()[1] > 0.0 || smoke.lifetime < 6.0);
    }

    #[test]
    fn the_same_seed_gives_the_same_burst() {
        let a = glue_burst([0, 0, 0], [0, 1, 0], true, &mut Rng::seeded(&[4, 2]));
        let b = glue_burst([0, 0, 0], [0, 1, 0], true, &mut Rng::seeded(&[4, 2]));
        assert_eq!(a, b);
    }
}

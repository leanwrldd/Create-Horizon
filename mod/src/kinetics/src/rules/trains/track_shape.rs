//! `TrackShape` (content/trains/track/TrackShape.java): the shapes a track block takes, each with
//! the axes the rails run along and the up normal; mirroring and rotating a shape.

use super::vec::{Vec3, normalize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrackShape {
    None,
    Zo,
    Xo,
    Pd,
    Nd,
    An,
    As,
    Ae,
    Aw,
    Tn,
    Ts,
    Te,
    Tw,
    CrO,
    CrD,
    CrPdx,
    CrPdz,
    CrNdx,
    CrNdz,
}

/// `net.minecraft.world.level.block.Rotation`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    CounterClockwise90,
}

/// `net.minecraft.world.level.block.Mirror`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}

/// A horizontal facing, for `asPortal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    North,
    South,
    East,
    West,
}

use TrackShape::*;

impl TrackShape {
    pub const ALL: [TrackShape; 19] = [
        None, Zo, Xo, Pd, Nd, An, As, Ae, Aw, Tn, Ts, Te, Tw, CrO, CrD, CrPdx, CrPdz, CrNdx, CrNdz,
    ];

    /// The serialized name (`Lang.asId(name())`): the enum constant in lower case.
    pub fn serialized_name(self) -> &'static str {
        match self {
            None => "none",
            Zo => "zo",
            Xo => "xo",
            Pd => "pd",
            Nd => "nd",
            An => "an",
            As => "as",
            Ae => "ae",
            Aw => "aw",
            Tn => "tn",
            Ts => "ts",
            Te => "te",
            Tw => "tw",
            CrO => "cr_o",
            CrD => "cr_d",
            CrPdx => "cr_pdx",
            CrPdz => "cr_pdz",
            CrNdx => "cr_ndx",
            CrNdz => "cr_ndz",
        }
    }

    /// The model name each shape is drawn with (the constructor's first argument).
    pub fn model(self) -> &'static str {
        match self {
            None => "",
            Zo => "z_ortho",
            Xo => "x_ortho",
            Pd => "diag",
            Nd => "diag_2",
            An | As | Ae | Aw => "ascending",
            Tn | Ts | Te | Tw => "teleport",
            CrO => "cross_ortho",
            CrD => "cross_diag",
            CrPdx => "cross_d1_xo",
            CrPdz => "cross_d1_zo",
            CrNdx => "cross_d2_xo",
            CrNdz => "cross_d2_zo",
        }
    }

    /// `getModelRotation()`.
    pub fn model_rotation(self) -> i32 {
        match self {
            An | Tn => 180,
            As | Ts => 0,
            Ae | Te => 270,
            Aw | Tw => 90,
            _ => 0,
        }
    }

    /// `getAxes()`: the directions the rails run along. A junction has two; the others one.
    pub fn axes(self) -> Vec<Vec3> {
        match self {
            None => vec![[0.0, 0.0, 0.0]],
            Zo => vec![[0.0, 0.0, 1.0]],
            Xo => vec![[1.0, 0.0, 0.0]],
            Pd => vec![[1.0, 0.0, 1.0]],
            Nd => vec![[-1.0, 0.0, 1.0]],
            An => vec![[0.0, 1.0, -1.0]],
            As => vec![[0.0, 1.0, 1.0]],
            Ae => vec![[1.0, 1.0, 0.0]],
            Aw => vec![[-1.0, 1.0, 0.0]],
            Tn => vec![[0.0, 0.0, -1.0]],
            Ts => vec![[0.0, 0.0, 1.0]],
            Te => vec![[1.0, 0.0, 0.0]],
            Tw => vec![[-1.0, 0.0, 0.0]],
            CrO => vec![[0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            CrD => vec![[1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]],
            CrPdx => vec![[1.0, 0.0, 0.0], [1.0, 0.0, 1.0]],
            CrPdz => vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]],
            CrNdx => vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 1.0]],
            CrNdz => vec![[0.0, 0.0, 1.0], [-1.0, 0.0, 1.0]],
        }
    }

    /// `getNormal()`: up for flat shapes; the slope's normal (normalised) for the ascending ones
    /// and the portals.
    pub fn normal(self) -> Vec3 {
        match self {
            An => normalize([0.0, 1.0, 1.0]),
            As => normalize([0.0, 1.0, -1.0]),
            Ae => normalize([-1.0, 1.0, 0.0]),
            Aw => normalize([1.0, 1.0, 0.0]),
            _ => [0.0, 1.0, 0.0],
        }
    }

    /// `isJunction()`: more than one axis.
    pub fn is_junction(self) -> bool {
        self.axes().len() > 1
    }

    /// `isPortal()`.
    pub fn is_portal(self) -> bool {
        matches!(self, Te | Tn | Ts | Tw)
    }

    /// `asPortal(Direction)`: west for anything that is not east, north or south.
    pub fn as_portal(facing: Facing) -> TrackShape {
        match facing {
            Facing::East => Te,
            Facing::North => Tn,
            Facing::South => Ts,
            Facing::West => Tw,
        }
    }

    fn z_mirror(self) -> TrackShape {
        match self {
            Pd => Nd,
            Nd => Pd,
            An => As,
            As => An,
            CrPdx => CrNdx,
            CrNdx => CrPdx,
            CrPdz => CrNdz,
            CrNdz => CrPdz,
            s => s,
        }
    }

    fn x_mirror(self) -> TrackShape {
        match self {
            Pd => Nd,
            Nd => Pd,
            Ae => Aw,
            Aw => Ae,
            CrPdx => CrNdx,
            CrNdx => CrPdx,
            CrPdz => CrNdz,
            CrNdz => CrPdz,
            s => s,
        }
    }

    fn clockwise(self) -> TrackShape {
        match self {
            Pd => Nd,
            Nd => Pd,
            Xo => Zo,
            Zo => Xo,
            Ae => As,
            As => Aw,
            Aw => An,
            An => Ae,
            CrPdx => CrNdz,
            CrNdx => CrPdz,
            CrPdz => CrNdx,
            CrNdz => CrPdx,
            s => s,
        }
    }

    /// `mirror(Mirror)`: front-back uses the x table, left-right the z table.
    pub fn mirror(self, mirror: Mirror) -> TrackShape {
        match mirror {
            Mirror::None => self,
            Mirror::FrontBack => self.x_mirror(),
            Mirror::LeftRight => self.z_mirror(),
        }
    }

    /// `rotate(Rotation)`: one clockwise step per rotation ordinal.
    pub fn rotate(self, rotation: Rotation) -> TrackShape {
        let mut shape = self;
        for _ in 0..(rotation as usize) {
            shape = shape.clockwise();
        }
        shape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junctions_and_portals() {
        let junctions: Vec<_> = TrackShape::ALL.iter().filter(|s| s.is_junction()).collect();
        assert_eq!(junctions.len(), 6);
        assert!(TrackShape::CrO.is_junction() && !TrackShape::Zo.is_junction());
        assert!(TrackShape::Te.is_portal() && !TrackShape::Ae.is_portal());
        assert_eq!(TrackShape::as_portal(Facing::West), TrackShape::Tw);
        assert_eq!(TrackShape::as_portal(Facing::North), TrackShape::Tn);
    }

    #[test]
    fn slope_normals_are_unit_and_tilted() {
        for s in [TrackShape::An, TrackShape::As, TrackShape::Ae, TrackShape::Aw] {
            let n = s.normal();
            assert!((super::super::vec::length(n) - 1.0).abs() < 1e-12);
            assert!(n[1] > 0.0 && n[1] < 1.0);
        }
        assert_eq!(TrackShape::Xo.normal(), [0.0, 1.0, 0.0]);
        // The normal of AS leans against its axis (0, 1, 1): -z.
        assert!(TrackShape::As.normal()[2] < 0.0);
    }

    #[test]
    fn a_full_turn_comes_back() {
        for s in TrackShape::ALL {
            let mut r = s;
            for _ in 0..4 {
                r = r.rotate(Rotation::Clockwise90);
            }
            // Four quarter turns, except where the table swaps in pairs (two turns return it).
            assert_eq!(r, s, "{s:?}");
            assert_eq!(s.rotate(Rotation::None), s);
        }
        assert_eq!(TrackShape::Ae.rotate(Rotation::Clockwise90), TrackShape::As);
        assert_eq!(TrackShape::Xo.rotate(Rotation::Clockwise180), TrackShape::Xo);
        assert_eq!(TrackShape::Pd.rotate(Rotation::CounterClockwise90), TrackShape::Nd);
    }

    #[test]
    fn mirrors() {
        assert_eq!(TrackShape::Pd.mirror(Mirror::FrontBack), TrackShape::Nd);
        assert_eq!(TrackShape::Ae.mirror(Mirror::FrontBack), TrackShape::Aw);
        assert_eq!(TrackShape::Ae.mirror(Mirror::LeftRight), TrackShape::Ae);
        assert_eq!(TrackShape::An.mirror(Mirror::LeftRight), TrackShape::As);
        assert_eq!(TrackShape::Zo.mirror(Mirror::FrontBack), TrackShape::Zo);
        assert_eq!(TrackShape::Pd.mirror(Mirror::None), TrackShape::Pd);
    }
}

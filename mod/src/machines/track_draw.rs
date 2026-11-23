//! Drawing the curves of the track (`TrackRenderer.renderBezierTurn`): along each curve the block entity at its start
//! sets a tie and the two rail pieces at every sample (`BezierConnection.SegmentAngles`: the poses are
//! `rules::trains::track_render::baked_segments`), each drawn with its partial model (`TrackMaterial.TrackModelHolder`:
//! `tie`, `segment_left`, `segment_right`, `.obj` meshes of Create's, imported from the player's jar). The straight
//! track is the chunk model of the block's state. A curve is drawn by its primary end only, as in Create.
//!
//! The draw is kept (`draw.keep()`): it only changes when the block entity's data does.
//!
//! Known differences from Create: the girder under a curve (`GirderAngles`) isn't drawn (no curve is made with one), the
//! tie and rails take the light of the block entity's cell rather than of their own, and the lines are drawn in the
//! order Create's `TrackRenderer` takes (a tie, then the left rail piece, then the right).

use create_kinetics::rules::trains::bezier::BezierConnection;
use create_kinetics::rules::trains::track_block::TRACK;
use create_kinetics::rules::trains::track_render::{Pose, baked_segments};
use crate::compat::{
    BlockEntityDraw, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, Hooks, Host, StandaloneModelsEvent,
};

use super::track as world;

/// `AllPartialModels.TRACK_TIE`, `TRACK_SEGMENT_LEFT` and `TRACK_SEGMENT_RIGHT`: `.obj` meshes.
const TIE: &str = "create:block/track/tie";
const LEFT: &str = "create:block/track/segment_left";
const RIGHT: &str = "create:block/track/segment_right";

struct Draw;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Draw)
}

fn f32s(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

/// A model set down by a pose: `translate(at).rotateY(yaw).rotateX(pitch).rotateZ(roll).translate(then)` and the
/// z scale (`SegmentAngles`).
fn place(host: &dyn Host, draw: &mut BlockEntityDraw, pose: &Pose, model: &str) {
    draw.push();
    draw.translate(f32s(pose.at));
    draw.rotate_about([0.0, 1.0, 0.0], pose.yaw.to_degrees(), [0.0; 3]);
    draw.rotate_about([1.0, 0.0, 0.0], pose.pitch.to_degrees(), [0.0; 3]);
    draw.rotate_about([0.0, 0.0, 1.0], pose.roll.to_degrees(), [0.0; 3]);
    draw.translate(f32s(pose.then));
    if pose.scale_z != 1.0 {
        draw.scale([1.0, 1.0, pose.scale_z]);
    }
    host.draw_model(draw, model, 0xFF_FFFF);
    draw.pop();
}

/// `TrackRenderer.renderBezierTurn`.
fn curve(host: &dyn Host, draw: &mut BlockEntityDraw, bc: &BezierConnection) {
    for piece in baked_segments(bc) {
        place(host, draw, &piece.tie, TIE);
        place(host, draw, &piece.rails[0], LEFT);
        place(host, draw, &piece.rails[1], RIGHT);
    }
}

impl Hooks for Draw {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [TIE, LEFT, RIGHT] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != TRACK {
            return;
        }
        for bc in world::connections_in(&event.entity.nbt, event.pos).iter().filter(|c| c.is_primary()) {
            curve(host, &mut *event.draw, bc);
        }
        event.draw.keep();
    }

    /// `TrackBlockEntity.getRenderBoundingBox` is infinite; here the box of the curves the entity draws, grown a block.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != TRACK {
            return;
        }
        let at = event.pos.map(f64::from);
        let mut bounds = [0.0_f64, 0.0, 0.0, 1.0, 1.0, 1.0];
        for bc in world::connections_in(&event.entity.nbt, event.pos).iter().filter(|c| c.is_primary()) {
            let [lo, hi] = bc.bounds();
            for i in 0..3 {
                bounds[i] = bounds[i].min(lo[i] - at[i] - 1.0);
                bounds[i + 3] = bounds[i + 3].max(hi[i] - at[i] + 1.0);
            }
        }
        *event.bounds = bounds.map(|v| v as f32);
    }
}

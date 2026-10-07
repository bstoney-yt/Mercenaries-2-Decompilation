//! The state layers' props that move on their own (the searchlights on
//! `VzaCon001`'s island sweeping their beams), posed each tick for
//! [`Props::draw`](crate::props::Props::draw).

use std::collections::HashMap;

use m2_engine::motion::Posed;
use m2_math::glam::Vec3;

use crate::scene::Scene;

/// Props further than this from the focus stand still (metres): a beam
/// is seen from well over a kilometre at night.
const MOVING_REACH: f32 = 3000.0;

/// The models of the moving props, posed.
#[derive(Debug, Default)]
pub struct MovingProps {
    /// By model and motion (entities differ in their hinges' angles):
    /// `None` when it couldn't be read.
    posed: HashMap<(u32, u32), Option<Posed>>,
    /// Seconds played.
    time: f32,
}

impl MovingProps {
    /// Moves the clips on by `dt` and poses the shown props near `focus`
    /// that move.
    pub fn update(&mut self, scene: &mut Scene, focus: Vec3, dt: f32) {
        self.time += dt;
        let Some(world) = &scene.world else {
            return;
        };
        let mut poses = HashMap::new();
        for (&entity, motion) in &world.index.motions {
            let Some((model, at)) = scene.props.placed(entity) else {
                continue;
            };
            if at.distance(focus) > MOVING_REACH {
                continue;
            }
            // Entities sharing a model and template differ only in their
            // hinge angles: key by those too.
            let angles = motion
                .hinges
                .iter()
                .fold(0u32, |h, &(n, a)| h.rotate_left(5) ^ n ^ a.to_bits());
            let posed = self.posed.entry((model, angles)).or_insert_with(|| {
                let source = scene.source.as_mut()?;
                match Posed::load(source, model, motion) {
                    Ok(p) => {
                        eprintln!(
                            "moving prop {entity:08x}: model {model:08x}, clip {:08x?}, hinges {:?}, {}",
                            motion.clip,
                            motion.hinges,
                            if p.is_some() { "posed" } else { "no such nodes" }
                        );
                        p
                    }
                    Err(e) => {
                        eprintln!("moving prop {entity:08x}: model {model:08x}: {e}");
                        None
                    }
                }
            });
            if let Some(p) = posed {
                // Each starts its loop at its own point, so that
                // neighbours don't sweep in step.
                let offset = (entity % 97) as f32 * 0.71;
                poses.insert(entity, p.nodes(self.time + offset));
            }
        }
        scene.props.set_poses(poses);
    }
}

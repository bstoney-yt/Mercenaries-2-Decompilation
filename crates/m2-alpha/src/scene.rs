//! What is drawn: the streamed island, or (with `--model`) a ground grid
//! with models from the install standing on it.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use anyhow::Context as _;
use m2_engine::anim::load_clips;
use m2_engine::model::MODEL_TEXTURE_WIDTH;
use m2_engine::vehicle::{DRIVABLE, catalogue};
use m2_engine::{
    Animator, AssetSource, BoundClip, Catalogue, CatalogueEntry, CpuModel, Frustum, Gait, GpuModel,
    Install, Locomotion, MaterialCache, PersonModel, Pose, Skeleton, VehicleId, VehicleSpec, World,
    WorldSettings,
};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::{
    BlendMode, BufferUsage, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle,
    IndexFormat, Layer, MaterialDesc, MaterialId, Primitive, RenderBackend, TextureDesc,
    TextureFormat, Topology, VertexBufferDesc, VertexBufferHandle, VertexFormat, VertexLayout,
    VertexSemantic, View, generate_mips,
};

use crate::cache;
use crate::combat::{self, Arsenal, Effect, Weapon};
use crate::npc::{self, People, PersonKind};
use crate::player::Player;
use crate::props::Props;

/// Jennifer Mui, the playable mercenary whose model the README's export
/// examples use.
pub const DEFAULT_MODEL: u32 = 0x15d8_9832;

/// Where the island opens: the first town, as in the viewer's examples.
pub const DEFAULT_SPAWN: (f32, f32) = (450.0, -1950.0);

/// Half the ground's width in metres.
const GROUND_HALF: f32 = 1000.0;
/// Metres per repeat of the grid texture.
const GRID_TILE: f32 = 4.0;

#[derive(Debug)]
pub struct Placed {
    pub model: GpuModel,
    pub world: Mat4,
    /// Skinned models play the game's locomotion clips in place.
    pub rig: Option<Rig>,
    /// Holding a moment of a clip (`--clip HASH@SECONDS`): not advanced.
    pub held: bool,
}

/// A skinned model's skeleton, the clips that fit it, and what it plays.
/// Clones share the skeleton and clips, each with its own animator.
#[derive(Clone, Debug)]
pub struct Rig {
    pub skeleton: Arc<Skeleton>,
    pub clips: Arc<Vec<BoundClip>>,
    pub locomotion: Option<Locomotion>,
    pub animator: Animator,
    pub gait: Gait,
    /// The upper body's nodes, for clips laid over the legs.
    pub upper: Option<Arc<[bool]>>,
    /// The pose holds a rifle in both hands (aiming, or at the ready).
    pub two_handed: bool,
    /// Where an aimed rifle should point, model space: the pose is turned
    /// so the barrel (rear hand to front) lies along it.
    pub aim: Option<Vec3>,
    /// (rear, front) hands of the aim pose, found once.
    pub(crate) hands: std::sync::OnceLock<Option<(usize, usize)>>,
    /// The arm that carries the rifle low, found once.
    pub(crate) arm: std::sync::OnceLock<Option<Arc<[bool]>>>,
}

/// How a rifle is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry {
    /// Low in the right hand, muzzle down and forward.
    Low,
    /// Standing at the ready (soldiers at their posts).
    Ready,
    /// Shouldered and aimed.
    Aim,
}

impl Rig {
    /// Reads model `hash`'s skeleton and every clip that fits it.
    pub fn load(
        source: &mut AssetSource<BufReader<File>>,
        hash: u32,
        gait: Gait,
    ) -> anyhow::Result<Self> {
        let started = std::time::Instant::now();
        let skeleton = Skeleton::from_nodes(&source.model(hash)?.parse()?.nodes);
        let clips = load_clips(source, &skeleton, &[""])?;
        let rig = Rig::new(skeleton, clips, gait);
        eprintln!(
            "model {hash:08x}: {} bones, {} clips fit ({:.1} s), locomotion {}",
            rig.skeleton.len(),
            rig.clips.len(),
            started.elapsed().as_secs_f32(),
            match &rig.locomotion {
                Some(l) => {
                    let mut c: Vec<_> = l
                        .clips
                        .iter()
                        .map(|(g, &i)| (*g, format!("{:08x}", rig.clips[i].hash)))
                        .collect();
                    c.sort();
                    format!("{c:?}")
                }
                None => "not found".into(),
            }
        );
        Ok(rig)
    }

    /// The clip `gait` plays, if there is one.
    pub fn gait_clip(&self, gait: Gait) -> Option<usize> {
        self.locomotion.as_ref()?.clips.get(&gait).copied()
    }

    /// Root speed (m/s) of the clip `gait` plays, if there is one.
    pub fn gait_speed(&self, gait: Gait) -> Option<f32> {
        let &i = self.locomotion.as_ref()?.clips.get(&gait)?;
        Some(self.clips[i].info.speed)
    }

    /// This rig's clips on another model's `skeleton`: shared when it has
    /// the same bones, otherwise the clips people play (locomotion, aim,
    /// fall) bound again by bone name. `None` if they don't fit.
    pub fn for_skeleton(&self, skeleton: Skeleton) -> Option<Self> {
        if skeleton.name_hashes == self.skeleton.name_hashes
            && skeleton.parents == self.skeleton.parents
        {
            return Some(Self {
                skeleton: Arc::new(skeleton),
                animator: Animator::default(),
                ..self.clone()
            });
        }
        let locomotion = self.locomotion.as_ref()?;
        let fight = npc::FightClips::find(self);
        let mut wanted: Vec<usize> = locomotion
            .clips
            .values()
            .copied()
            .chain(fight.all())
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut clips = Vec::new();
        let mut moved = std::collections::HashMap::new();
        for i in wanted {
            if let Some(b) = self.clips[i].rebind(&skeleton) {
                moved.insert(i, clips.len());
                clips.push(b);
            }
        }
        let gaits: std::collections::HashMap<Gait, usize> = locomotion
            .clips
            .iter()
            .filter_map(|(g, i)| Some((*g, *moved.get(i)?)))
            .collect();
        if gaits.len() < locomotion.clips.len() {
            return None;
        }
        Some(Self {
            upper: crate::combat::upper_body(&skeleton),
            two_handed: false,
            aim: None,
            hands: Default::default(),
            arm: Default::default(),
            skeleton: Arc::new(skeleton),
            clips: Arc::new(clips),
            locomotion: Some(Locomotion {
                clips: gaits,
                forward: locomotion.forward,
            }),
            animator: Animator::default(),
            gait: Gait::Idle,
        })
    }

    fn new(skeleton: Skeleton, clips: Vec<BoundClip>, gait: Gait) -> Self {
        let locomotion = Locomotion::choose(&clips);
        let mut rig = Self {
            upper: crate::combat::upper_body(&skeleton),
            two_handed: false,
            aim: None,
            hands: Default::default(),
            arm: Default::default(),
            skeleton: Arc::new(skeleton),
            clips: Arc::new(clips),
            locomotion,
            animator: Animator::default(),
            gait,
        };
        rig.set_gait(gait);
        rig
    }

    pub fn set_gait(&mut self, gait: Gait) {
        self.gait = gait;
        if let Some(&clip) = self.locomotion.as_ref().and_then(|l| l.clips.get(&gait)) {
            self.animator.play(clip);
        }
    }

    /// Moves at `gait` holding a rifle as `carry` says. Aiming, it is
    /// shouldered: by the whole body standing, by the upper body over the
    /// legs' gait when moving. Otherwise the plain gait carries it low in
    /// the right hand (as the game does), except that a soldier at the
    /// ready standing holds it across the chest. Falls back to the plain
    /// gait without the clips.
    pub fn hold_rifle(&mut self, gait: Gait, clips: &npc::FightClips, carry: Carry) {
        let (whole, upper) = match (gait, carry) {
            (Gait::Idle, Carry::Aim) => (clips.aim, None),
            (_, Carry::Aim) => (None, clips.aim),
            (Gait::Idle, Carry::Ready) => (clips.ready, None),
            _ => (None, None),
        };
        match whole {
            Some(clip) => {
                self.gait = gait;
                self.animator.play(clip);
            }
            None => self.set_gait(gait),
        }
        let upper = upper.zip(self.upper.clone());
        self.two_handed = whole.is_some() || upper.is_some();
        if carry != Carry::Aim {
            self.aim = None;
        }
        // Carried low on the move, the rifle arm hangs as it does standing
        // while the rest of her runs (the game's carry).
        let arm = (carry == Carry::Low && gait != Gait::Idle)
            .then(|| self.gait_clip(Gait::Idle).zip(self.carry_arm()))
            .flatten();
        self.animator.set_upper(upper.or(arm));
    }

    /// The pose turned to aim along [`Rig::aim`], when aiming.
    pub fn aimed_pose(&self) -> Option<Pose> {
        let want = self.aim.filter(|_| self.two_handed)?;
        let mut pose = self.animator.pose(&self.skeleton, &self.clips);
        self.point(&mut pose, want);
        Some(pose)
    }

    /// The (rear, front) hands of the aim pose
    /// ([`aim_hands`](crate::combat::aim_hands)).
    pub fn aim_hands(&self) -> Option<(usize, usize)> {
        *self.hands.get_or_init(|| crate::combat::aim_hands(self))
    }

    /// The arm of the hand that carries the rifle low.
    pub fn carry_arm(&self) -> Option<Arc<[bool]>> {
        self.arm
            .get_or_init(|| {
                let (right, left) = crate::combat::hands(&self.skeleton)?;
                self.skeleton.arm(right, left)
            })
            .clone()
    }

    /// The hand that carries the rifle low: her right (the aim pose holds
    /// it in whichever hand is rear, [`Rig::aim_hands`]).
    pub fn rifle_hand(&self) -> Option<usize> {
        crate::combat::hands(&self.skeleton).map(|(right, _)| right)
    }

    /// Aims along `dir` (world space) for a body turned to `facing`, or
    /// stops aiming (`None`).
    pub fn aim_along(&mut self, facing: f32, dir: Option<Vec3>) {
        let forward = self
            .locomotion
            .as_ref()
            .map_or(m2_math::glam::Vec2::Y, |l| l.forward);
        let turn = Quat::from_rotation_y(facing - forward.x.atan2(forward.y));
        self.aim = dir.map(|d| turn.inverse() * d);
    }

    /// How far the rifle in an aiming pose points off `want` (model
    /// space): (yaw, pitch) radians to turn it by, the barrel taken as the
    /// line from the rear hand to the front.
    pub fn aim_error(&self, pose: &Pose, want: Vec3) -> Option<(f32, f32)> {
        let (rh, lh) = self.aim_hands()?;
        let nodes = self.skeleton.model_matrices(pose);
        let barrel = (nodes[lh].w_axis - nodes[rh].w_axis).truncate();
        let (b, w) = (barrel.normalize_or_zero(), want.normalize_or_zero());
        if b == Vec3::ZERO || w == Vec3::ZERO {
            return None;
        }
        let yaw = |v: Vec3| v.x.atan2(v.z);
        let mut turn = (yaw(w) - yaw(b)).rem_euclid(std::f32::consts::TAU);
        if turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        }
        Some((
            turn,
            w.y.clamp(-1.0, 1.0).asin() - b.y.clamp(-1.0, 1.0).asin(),
        ))
    }

    /// Turns an aiming pose so the rifle points along `want` (model
    /// space): standing, the whole body turns to it (the aim clip stands
    /// side-on to the target); over moving legs, the upper body twists at
    /// the waist (within reason). The barrel's rise or drop is made up at
    /// the waist either way.
    fn point(&self, pose: &mut Pose, want: Vec3) {
        let Some((yaw, _)) = self.aim_error(pose, want) else {
            return;
        };
        let root = self.skeleton.root();
        let waist = self.upper.as_ref().and_then(|mask| {
            (0..mask.len()).find(|&n| mask[n] && self.skeleton.parents[n].is_none_or(|p| !mask[p]))
        });
        let turn_at = |pose: &mut Pose, node: usize, by: Quat| {
            // The node's turn in model space, made local to its parent.
            let nodes = self.skeleton.model_matrices(pose);
            let parent = self.skeleton.parents[node].map_or(Quat::IDENTITY, |p| {
                nodes[p].to_scale_rotation_translation().1
            });
            let local = &mut pose.0[node];
            local.rotation = (parent.inverse() * by * parent * local.rotation).normalize();
        };
        if self.animator.upper().is_none() {
            let local = &mut pose.0[root];
            let by = Quat::from_rotation_y(yaw);
            local.rotation = (by * local.rotation).normalize();
            local.translation = by * local.translation;
        } else if let Some(w) = waist {
            turn_at(
                pose,
                w,
                Quat::from_rotation_y(yaw.clamp(-MAX_TWIST, MAX_TWIST)),
            );
        }
        let Some(((_, pitch), w)) = self.aim_error(pose, want).zip(waist) else {
            return;
        };
        let level = want.with_y(0.0).normalize_or(Vec3::Z);
        let axis = level.cross(Vec3::Y);
        let pitch = pitch.clamp(-MAX_BEND, MAX_BEND);
        turn_at(pose, w, Quat::from_axis_angle(axis, pitch));
    }

    /// The next gait, slowest after fastest.
    pub fn cycle_gait(&mut self) {
        self.set_gait(match self.gait {
            Gait::Idle => Gait::Walk,
            Gait::Walk => Gait::Run,
            Gait::Run => Gait::Sprint,
            Gait::Sprint => Gait::Idle,
        });
    }

    /// Posed node matrices and skinning matrices, model space.
    pub fn posed(&self) -> (Vec<Mat4>, Vec<Mat4>) {
        let pose = self
            .aimed_pose()
            .unwrap_or_else(|| self.animator.pose(&self.skeleton, &self.clips));
        let nodes = self.skeleton.model_matrices(&pose);
        let skin = nodes
            .iter()
            .zip(&self.skeleton.inverse_bind)
            .map(|(m, ib)| *m * *ib)
            .collect();
        (nodes, skin)
    }
}

/// The most an aiming upper body twists at the waist from the legs,
/// radians.
const MAX_TWIST: f32 = 1.3;
/// And bends up or down.
const MAX_BEND: f32 = 0.9;

#[derive(Debug)]
pub struct Scene {
    /// The grid, drawn when there is no island.
    ground: (VertexBufferHandle, IndexBufferHandle, MaterialId),
    pub world: Option<World>,
    pub models: Vec<Placed>,
    pub materials: MaterialCache,
    pub source: Option<AssetSource<BufReader<File>>>,
    /// Where the camera should start: (eye, target).
    pub focus: (Vec3, Vec3),
    /// Jennifer, on the island.
    pub player: Option<Player>,
    /// Drivable vehicles near the spawn.
    pub vehicles: Vec<ParkedVehicle>,
    /// Soldiers and civilians.
    pub people: People,
    pub arsenal: Arsenal,
    /// Fuel tanks and barrels that blow up when hit.
    pub explosives: Vec<crate::combat::Explosive>,
    /// What the loaded state layers place (a mission's gates).
    pub props: Props,
    /// Mesh and materials for shots, flashes, fire and smoke.
    effects: Effects,
    vehicle_loads: Option<Receiver<Loaded>>,
    /// The archive's vehicle, person and weapon models, once found.
    pub catalogue: Option<Catalogue>,
    install: Option<Install>,
    /// Vehicles loaded on request ([`Scene::load_vehicle`]).
    requests: (Sender<Loaded>, Receiver<Loaded>),
}

/// A vehicle to drive: what it is, its model, and its body once placed.
#[derive(Debug)]
pub struct ParkedVehicle {
    pub spec: VehicleSpec,
    pub model: GpuModel,
    pub id: Option<VehicleId>,
    /// Blown up: can't be driven.
    pub wrecked: bool,
    /// The mission scripts' object it is, if theirs; placed where they
    /// say rather than parked.
    pub guid: Option<u32>,
    /// Where the island's layers park it (position, yaw), if the game's
    /// own: the entity and its spot.
    pub spot: Option<(u32, Vec3, f32)>,
    /// How far its turret is turned from straight ahead, radians
    /// (anticlockwise from above).
    pub turret: f32,
    /// How far its gun is raised, radians (down is negative).
    pub gun: f32,
    /// How far it has rolled, metres (backwards less), for turning a
    /// tracked vehicle's road wheels.
    pub rolled: f32,
}

/// Kinds that don't drive on wheels (or aren't vehicles to sit in).
pub const NOT_DRIVABLE: [&str; 13] = [
    "boat",
    "plane",
    "airstrike",
    "vtol",
    "towed",
    "trailer",
    "outboardmotor",
    "winch",
    "transport",
    "motorcycle",
    "helicopter",
    "jetski",
    "gunboat",
];

/// Vehicles parked by the spawn: one of each drivable kind first.
const PARKED: usize = 8;

/// The state layers' models load for props this much further away than
/// they are drawn, metres.
const PROP_MARGIN: f32 = 200.0;

/// Person models loaded: the soldiers the spawn names ask for, then
/// civilians.
const ARMY: [&str; 3] = ["vz_hum_soldier", "vz_hum_soldierelite", "vz_hum_deathsquad"];
const CIVILIAN_MODELS: usize = 6;

/// A model the loading thread has decoded.
pub enum Loaded {
    Vehicle(VehicleSpec, CpuModel),
    /// A vehicle asked for by the mission scripts' object `guid`.
    MissionVehicle(u32, VehicleSpec, CpuModel),
    /// One of the game's own parked vehicles: its layer entity, where it
    /// stands (position, yaw), its spec and model.
    WorldVehicle(u32, Vec3, f32, VehicleSpec, CpuModel),
    Catalogue(Catalogue),
    /// A person's model: a role's, or a story character's (true).
    Person(PersonModel, CpuModel, Skeleton, bool),
    /// A hand weapon: name, model, muzzle (model space).
    Weapon(&'static str, CpuModel, Vec3),
}

/// Finds the island's vehicle and person models and decodes the soldiers,
/// a few civilians and up to [`PARKED`] vehicles, on a thread (the search
/// reads the whole archive).
fn load_cast(install: Install) -> Receiver<Loaded> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let result = (|| -> anyhow::Result<()> {
            let mut source = install.open_world()?;
            // The search reads the whole archive, so its result is kept.
            let stamp = cache::stamp(&install.world_archive());
            let path = cache::default_path();
            let found = match stamp.and_then(|s| cache::read(&path, s)) {
                Some(found) => found,
                None => {
                    let found = catalogue(&mut source)?;
                    if let Some(s) = stamp
                        && let Err(e) = cache::write(&path, s, &found)
                    {
                        eprintln!("could not keep the catalogue in {}: {e}", path.display());
                    }
                    found
                }
            };
            if tx.send(Loaded::Catalogue(found.clone())).is_err() {
                return Ok(());
            }
            let people: Vec<&PersonModel> = found
                .people
                .iter()
                .filter(|p| ARMY.contains(&p.role.as_str()))
                .chain(
                    found
                        .people
                        .iter()
                        .filter(|p| !p.soldier)
                        .take(CIVILIAN_MODELS),
                )
                .collect();
            eprintln!(
                "people: {} roles; loading {}",
                found.people.len(),
                people
                    .iter()
                    .map(|p| p.role.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for p in people {
                let data = source.model(p.model)?;
                let mut cpu = CpuModel::from_data(&data, 0)?;
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                let skeleton = Skeleton::from_nodes(&data.parse()?.nodes);
                if tx
                    .send(Loaded::Person(p.clone(), cpu, skeleton, false))
                    .is_err()
                {
                    return Ok(());
                }
            }
            let mut weapons = Vec::new();
            for &(name, hash) in &found.weapons {
                let data = source.model(hash)?;
                let model = data.parse()?;
                let muzzle = m2_assets::vehicle::marker(&model, combat::MUZZLE)
                    .map_or(Vec3::new(0.0, 0.0, model.info.bounds.max[2]), Vec3::from);
                let (lo, hi) = (model.info.bounds.min, model.info.bounds.max);
                weapons.push(format!(
                    "{name} {hash:08x} ({:.2} x {:.2} x {:.2} m)",
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    hi[2] - lo[2]
                ));
                let mut cpu = CpuModel::build(hash, &model, 0);
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                if tx.send(Loaded::Weapon(name, cpu, muzzle)).is_err() {
                    return Ok(());
                }
            }
            eprintln!("weapons: {}", weapons.join(", "));
            let all = &found.vehicles;
            let drivable: Vec<&CatalogueEntry> = all
                .iter()
                .filter(|e| {
                    DRIVABLE.contains(&e.kind.as_str())
                        // `global_veh_<model>`: civilian cars and trucks,
                        // named by model rather than kind.
                        || (e.label.starts_with("global_") && !NOT_DRIVABLE.contains(&e.kind.as_str()))
                })
                .collect();
            eprintln!(
                "vehicles: {} models by texture name, {} drivable kinds ({:.1} s)",
                all.len(),
                drivable.len(),
                started.elapsed().as_secs_f32()
            );
            let rank = |e: &CatalogueEntry| {
                DRIVABLE
                    .iter()
                    .position(|k| *k == e.kind)
                    .unwrap_or(DRIVABLE.len())
            };
            let mut order = drivable.clone();
            order.sort_by_key(|e| rank(e));
            // One of each kind, then the rest, one model per name.
            let mut kinds = std::collections::HashSet::new();
            let firsts: Vec<&CatalogueEntry> = order
                .iter()
                .copied()
                .filter(|e| kinds.insert(e.kind.clone()))
                .collect();
            let mut labels = std::collections::HashSet::new();
            let mut sent = 0;
            for e in firsts.into_iter().chain(order) {
                if sent >= PARKED || !labels.insert(e.label.clone()) {
                    continue;
                }
                let data = source.model(e.model)?;
                let model = data.parse()?;
                let Some(spec) = VehicleSpec::from_model(e, &model) else {
                    labels.remove(&e.label);
                    continue;
                };
                eprintln!(
                    "vehicle {:08x}: {} ({}, {} wheels)",
                    e.model,
                    e.label,
                    e.kind,
                    spec.wheels.len()
                );
                let mut cpu = CpuModel::build(e.model, &model, 0);
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                if tx.send(Loaded::Vehicle(spec, cpu)).is_err() {
                    return Ok(());
                }
                sent += 1;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("vehicles and people: {e:#}");
        }
    });
    rx
}

impl Scene {
    pub fn new(backend: &mut dyn RenderBackend) -> anyhow::Result<Self> {
        let mut materials = MaterialCache::new();
        let ground = ground(backend, &mut materials)?;
        let effects = Effects::new(backend, &mut materials)?;
        Ok(Self {
            ground,
            world: None,
            models: Vec::new(),
            materials,
            source: None,
            focus: (Vec3::new(0.0, 2.0, 6.0), Vec3::new(0.0, 1.0, 0.0)),
            player: None,
            vehicles: Vec::new(),
            people: People::default(),
            arsenal: Arsenal::default(),
            explosives: Vec::new(),
            props: Props::default(),
            effects,
            vehicle_loads: None,
            catalogue: None,
            install: None,
            requests: channel(),
        })
    }

    /// Opens the install's world archive.
    pub fn open(&mut self, install: &Install) -> anyhow::Result<()> {
        let source = install
            .open_world()
            .with_context(|| format!("opening {}", install.world_archive().display()))?;
        self.source = Some(source);
        Ok(())
    }

    /// Loads models `hashes` and stands them in a row on the ground, 3 m
    /// apart, facing the camera (models face `+z`).
    pub fn load_models(
        &mut self,
        backend: &mut dyn RenderBackend,
        show: &crate::Showcase,
    ) -> anyhow::Result<()> {
        let source = self.source.as_mut().context("no install open")?;
        let mut x = 0.0;
        let mut widest = 2.0f32;
        let mut rigs: HashMap<u32, Rig> = HashMap::new();
        for (i, &hash) in show.models.iter().enumerate() {
            let mut held = false;
            let gpu = GpuModel::load(backend, source, &mut self.materials, hash, 0)?;
            let rig = if gpu.is_skinned() {
                let mut rig = match rigs.get(&hash) {
                    Some(r) => r.clone(),
                    None => {
                        let r = Rig::load(source, hash, show.gait)?;
                        rigs.insert(hash, r.clone());
                        r
                    }
                };
                if let Some(&(clip, at)) = show.clips.get(i) {
                    match (rig.clips.iter().position(|c| c.hash == clip), at) {
                        (Some(k), Some(t)) => {
                            let length = rig.clips[k].info.duration;
                            eprintln!(
                                "model {i}: clip {clip:08x} held {t:.2} s in (of {length:.2})"
                            );
                            rig.animator.play_once(k);
                            rig.animator.seek(t);
                            held = true;
                        }
                        (Some(k), None) => {
                            eprintln!("model {i}: clip {clip:08x}");
                            rig.animator.play(k);
                        }
                        (None, _) => eprintln!("model {i}: no clip {clip:08x} fits"),
                    }
                }
                Some(rig)
            } else {
                None
            };
            let (lo, hi) = gpu.bounds;
            eprintln!(
                "model {hash:08x}: {} draws, {:.1} x {:.1} x {:.1} m{}",
                gpu.draw_count(),
                hi.x - lo.x,
                hi.y - lo.y,
                hi.z - lo.z,
                if gpu.skipped_groups > 0 {
                    format!(
                        ", {} groups in unsupported vertex formats",
                        gpu.skipped_groups
                    )
                } else {
                    String::new()
                }
            );
            let size = hi - lo;
            widest = widest.max(size.x.max(size.z)).max(size.y);
            // Stand it on the ground, centred on x.
            let offset = Vec3::new(x - (lo.x + hi.x) * 0.5, -lo.y, -(lo.z + hi.z) * 0.5);
            let centre = Vec3::new(x, 0.0, 0.0);
            self.models.push(Placed {
                model: gpu,
                world: Mat4::from_translation(centre)
                    * Mat4::from_rotation_y(show.turn.to_radians())
                    * Mat4::from_translation(offset - centre),
                rig,
                held,
            });
            x += size.x.max(1.0) + 3.0;
        }
        let centre = Vec3::new((x - 3.0).max(0.0) * 0.5, widest * 0.5, 0.0);
        self.focus = (
            // Models face +z, so the camera stands on that side.
            centre + Vec3::new(0.0, widest * 0.3, widest * 1.6 + x * 0.5),
            centre,
        );
        Ok(())
    }

    /// Indexes the island and starts streaming it, with the camera placed
    /// above `(x, z)`.
    pub fn open_world(
        &mut self,
        backend: &mut dyn RenderBackend,
        install: &Install,
        settings: WorldSettings,
        spawn: (f32, f32),
    ) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let world = World::open(install, backend, &mut self.materials, settings)
            .context("indexing the island")?;
        eprintln!(
            "island: {} cells, {} tiles, {} placed objects ({} by name), {} template placements (indexed in {:.1} s)",
            world.index.cells.len(),
            world.index.tiles.len(),
            world.index.objects.len(),
            world.index.objects.iter().filter(|o| o.named).count(),
            world.index.templates.len(),
            started.elapsed().as_secs_f32()
        );
        self.explosives = crate::combat::explosives(&world.index);
        eprintln!("explosives: {}", self.explosives.len());
        let (x, z) = spawn;
        let y = world.index.rough_height(x, z) + 40.0;
        self.focus = (Vec3::new(x, y, z), Vec3::new(x, y - 20.0, z + 60.0));
        self.world = Some(world);

        // Jennifer, to walk the island as.
        self.open(install)?;
        let source = self.source.as_mut().context("no install open")?;
        let model = match GpuModel::load(backend, source, &mut self.materials, DEFAULT_MODEL, 0)
            .map_err(anyhow::Error::from)
            .and_then(|gpu| Ok((gpu, Rig::load(source, DEFAULT_MODEL, Gait::Idle)?)))
        {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("player model: {e:#}");
                None
            }
        };
        let clips = model
            .as_ref()
            .map(|(_, rig)| npc::FightClips::find(rig))
            .unwrap_or_default();
        self.people.clips = clips;
        let mut player = Player::new(x, z, 0.0, model);
        player.clips = clips;
        self.player = Some(player);
        self.vehicle_loads = Some(load_cast(install.clone()));
        self.install = Some(install.clone());
        Ok(())
    }

    /// Loads a vehicle for the mission scripts' object `guid` on a thread:
    /// the first of `models` that makes a drivable vehicle. It joins
    /// [`Scene::vehicles`] unplaced, with its guid.
    pub fn load_vehicle(&self, guid: u32, models: Vec<CatalogueEntry>) {
        self.load_vehicle_as(
            models,
            format!("mission vehicle {guid:#x}"),
            move |spec, cpu| Loaded::MissionVehicle(guid, spec, cpu),
        );
    }

    /// Loads the first of `models` that drives, to be parked near the
    /// player as the vehicles she starts with are.
    pub fn load_parked(&self, models: Vec<CatalogueEntry>) {
        self.load_vehicle_as(models, "parked vehicle".into(), Loaded::Vehicle);
    }

    /// Loads one of the game's own parked vehicles, layer entity `entity`
    /// standing at `at` facing `yaw`, from the first of `models` that
    /// drives.
    pub fn load_world_vehicle(&self, entity: u32, at: Vec3, yaw: f32, models: Vec<CatalogueEntry>) {
        self.load_vehicle_as(models, format!("vehicle {entity:#x}"), move |spec, cpu| {
            Loaded::WorldVehicle(entity, at, yaw, spec, cpu)
        });
    }

    /// Loads a story character's model (`Carmona`'s, by their template)
    /// on a thread; it joins the people's kinds as `template`.
    fn load_character(&self, model: u32, template: String) {
        let Some(install) = self.install.clone() else {
            return;
        };
        let tx = self.requests.0.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let mut source = install.open_world()?;
                let data = source.model(model)?;
                let cpu = CpuModel::from_data(&data, 0)?;
                let skeleton = Skeleton::from_nodes(&data.parse()?.nodes);
                let person = PersonModel {
                    model,
                    role: template.clone(),
                    soldier: false,
                };
                let _ = tx.send(Loaded::Person(person, cpu, skeleton, true));
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("{template} ({model:08x}): {e:#}");
            }
        });
    }

    /// Decodes the first of `models` that drives on a thread and sends it
    /// as `make` wraps it; `what` names it in errors.
    fn load_vehicle_as(
        &self,
        models: Vec<CatalogueEntry>,
        what: String,
        make: impl FnOnce(VehicleSpec, CpuModel) -> Loaded + Send + 'static,
    ) {
        let Some(install) = self.install.clone() else {
            return;
        };
        let tx = self.requests.0.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let mut source = install.open_world()?;
                for e in &models {
                    let data = source.model(e.model)?;
                    let model = data.parse()?;
                    if let Some(spec) = VehicleSpec::from_model(e, &model) {
                        let mut cpu = CpuModel::build(e.model, &model, 0);
                        cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                        let _ = tx.send(make(spec, cpu));
                        return Ok(());
                    }
                }
                anyhow::bail!("no drivable model")
            })();
            if let Err(e) = result {
                eprintln!("{what}: {e:#}");
            }
        });
    }

    /// Streams around `eye`.
    pub fn update(&mut self, backend: &mut dyn RenderBackend, eye: Vec3) -> anyhow::Result<()> {
        if let Some(world) = &mut self.world {
            world.update(backend, &mut self.materials, eye)?;
        }
        // At most one model a frame from each: each uploads a model.
        if let Some(rx) = &self.vehicle_loads {
            match rx.try_recv() {
                Ok(loaded) => self.take(backend, loaded)?,
                Err(TryRecvError::Disconnected) => self.vehicle_loads = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Ok(loaded) = self.requests.1.try_recv() {
            self.take(backend, loaded)?;
        }
        for (model, template) in std::mem::take(&mut self.people.wanted) {
            self.load_character(model, template);
        }
        if let (Some(world), Some(source)) = (&mut self.world, &mut self.source) {
            // As far as they are drawn, and a little more.
            let reach = world.settings.object_distance + PROP_MARGIN;
            self.props.update(
                backend,
                &mut self.materials,
                source,
                &mut world.physics,
                eye,
                reach,
            )?;
        }
        Ok(())
    }

    /// Puts up what the state layers place for `entities` (see
    /// [`Props::show`]).
    pub fn show_props(&mut self, entities: impl IntoIterator<Item = u32>) -> usize {
        match (&self.install, &self.world) {
            (Some(install), Some(world)) => self.props.show(install, &world.index, entities),
            _ => 0,
        }
    }

    /// Uploads a model the loading threads decoded and puts it to use.
    fn take(&mut self, backend: &mut dyn RenderBackend, loaded: Loaded) -> anyhow::Result<()> {
        let Some(source) = &mut self.source else {
            return Ok(());
        };
        let cpu = match &loaded {
            Loaded::Catalogue(c) => {
                self.catalogue = Some(c.clone());
                return Ok(());
            }
            Loaded::Vehicle(_, cpu)
            | Loaded::MissionVehicle(_, _, cpu)
            | Loaded::WorldVehicle(_, _, _, _, cpu)
            | Loaded::Person(_, cpu, _, _)
            | Loaded::Weapon(_, cpu, _) => cpu,
        };
        self.materials.load_model_textures(backend, source, cpu)?;
        let model = GpuModel::upload(backend, &mut self.materials, cpu)?;
        // What its template says of its handling and its gun: its own
        // (the mission's `AMX30 (Full)`), else its model's.
        let tune = |spec: VehicleSpec| {
            let t = self
                .world
                .as_ref()
                .and_then(|w| w.index.template_db.tuning_for(&spec.label, spec.model));
            spec.tuned(t)
        };
        match loaded {
            Loaded::Catalogue(_) => {}
            Loaded::Vehicle(spec, _) => self.vehicles.push(ParkedVehicle {
                spec: tune(spec),
                model,
                id: None,
                wrecked: false,
                guid: None,
                spot: None,
                turret: 0.0,
                gun: 0.0,
                rolled: 0.0,
            }),
            Loaded::MissionVehicle(guid, spec, _) => {
                eprintln!("mission vehicle {guid:#x}: {}", spec.label);
                self.vehicles.push(ParkedVehicle {
                    spec: tune(spec),
                    model,
                    id: None,
                    wrecked: false,
                    guid: Some(guid),
                    spot: None,
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                });
            }
            Loaded::WorldVehicle(entity, at, yaw, spec, _) => {
                self.vehicles.push(ParkedVehicle {
                    spec: tune(spec),
                    model,
                    id: None,
                    wrecked: false,
                    guid: None,
                    spot: Some((entity, at, yaw)),
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                });
            }
            Loaded::Weapon(name, _, muzzle) => {
                let rig = self
                    .player
                    .as_ref()
                    .and_then(|pl| pl.model.as_ref())
                    .map(|(_, rig)| rig);
                self.arsenal.add(name, model, muzzle, rig);
            }
            Loaded::Person(p, _, skeleton, named) => {
                // Her clips fit every human skeleton.
                let template = self
                    .player
                    .as_ref()
                    .and_then(|pl| pl.model.as_ref())
                    .and_then(|(_, rig)| rig.for_skeleton(skeleton));
                match template {
                    Some(rig) => {
                        eprintln!(
                            "person {}: {} bones, {} clips",
                            p.role,
                            rig.skeleton.len(),
                            rig.clips.len()
                        );
                        self.people.add_kind(PersonKind {
                            role: p.role,
                            soldier: p.soldier,
                            named,
                            model,
                            rig,
                        });
                    }
                    None => eprintln!(
                        "person {}: the clips don't fit its skeleton; left out",
                        p.role
                    ),
                }
            }
        }
        Ok(())
    }

    /// Vehicles and people are still being found and decoded.
    pub fn vehicles_loading(&self) -> bool {
        self.vehicle_loads.is_some()
    }

    /// Advances animation by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        for p in self.models.iter_mut().filter(|p| !p.held) {
            if let Some(rig) = &mut p.rig {
                rig.animator.advance(dt);
            }
        }
    }

    /// Steps every rig to its next gait.
    pub fn cycle_gaits(&mut self) {
        for rig in self.models.iter_mut().filter_map(|p| p.rig.as_mut()) {
            rig.cycle_gait();
            eprintln!("gait: {:?}", rig.gait);
        }
    }

    /// Queues the frame's draws; call inside the backend's frame, as skinned
    /// models set their palettes on it.
    pub fn draw(
        &mut self,
        backend: &mut dyn RenderBackend,
        queue: &mut DrawQueue,
        view: &View,
        fight: &combat::Combat,
    ) -> anyhow::Result<()> {
        let eye = view.eye();
        if let Some(world) = &mut self.world {
            let frustum = Frustum::from_view_proj(view.projection * view.view);
            world.draw(backend, queue, eye, &frustum)?;
            self.props.draw(queue, eye, world.settings.object_distance);
        } else {
            self.draw_ground(queue);
        }
        if let Some(world) = &self.world {
            for v in &mut self.vehicles {
                // A wreck shows its wrecked body, turret and wheels.
                v.model.wrecked = v.wrecked;
                // Tracks run as far as the road wheels roll.
                v.model.tracks = v.spec.tracked;
                v.model.track_travel = v.rolled;
                let Some(id) = v.id else { continue };
                let Some(at) = world.physics.vehicle_transform(id) else {
                    continue;
                };
                let nodes = v.spec.posed_nodes(
                    &world.physics.vehicle_wheels(id),
                    v.rolled,
                    v.turret,
                    v.gun,
                );
                v.model.draw_posed(backend, queue, at, &nodes, &[])?;
            }
        }
        if let Some(player) = &self.player
            && player.landed
            && player.driving.is_none()
            && let Some((model, rig)) = &player.model
        {
            let (nodes, skin) = rig.posed();
            let body = player.model_matrix();
            model.draw_posed(backend, queue, body, &nodes, &skin)?;
            // Throwing, her weapon is put away and the grenade is in her
            // hand until it goes.
            let weapon = match (player.throwing, fight.weapon) {
                (Some(_), _) if fight.grenade_in_hand() => &self.arsenal.grenade,
                (Some(_), _) => &None,
                (None, Weapon::Rifle) => &self.arsenal.rifle,
                (None, Weapon::Rpg) => &self.arsenal.rpg,
            };
            if !player.down
                && let Some(w) = weapon
                && let Some(at) = self.arsenal.in_hand(
                    w,
                    body,
                    &nodes,
                    (self.arsenal.hand, self.arsenal.attach),
                    rig.two_handed,
                )
            {
                w.model.draw(queue, at);
            }
        }
        for n in &self.people.npcs {
            if !n.landed || n.character.position.distance(eye) > npc::DRAW_DISTANCE {
                continue;
            }
            let model = &self.people.kinds[n.kind].model;
            // The dead lie as their limp bodies fell.
            if let Some(doll) = n.ragdoll
                && let Some(world) = &self.world
                && let Some(nodes) = world.physics.ragdoll_nodes(doll)
            {
                let skin: Vec<Mat4> = nodes
                    .iter()
                    .zip(&n.rig.skeleton.inverse_bind)
                    .map(|(m, ib)| *m * *ib)
                    .collect();
                model.draw_posed(backend, queue, Mat4::IDENTITY, &nodes, &skin)?;
                continue;
            }
            let (nodes, skin) = n.rig.posed();
            let body = n.model_matrix();
            model.draw_posed(backend, queue, body, &nodes, &skin)?;
            if n.soldier
                && n.alive
                && let Some(w) = &self.arsenal.rifle
                && let Some(at) =
                    self.arsenal
                        .in_hand(w, body, &nodes, (n.hand, n.attach), n.rig.two_handed)
            {
                w.model.draw(queue, at);
            }
        }
        for (rocket, at, velocity) in fight.projectiles() {
            let model = if rocket {
                &self.arsenal.rocket
            } else {
                &self.arsenal.grenade
            };
            if let Some(w) = model {
                // The rocket's nose (its muzzle marker) along its flight.
                let turn = Quat::from_rotation_arc(
                    w.muzzle.normalize_or(Vec3::Z),
                    velocity.normalize_or(Vec3::Z),
                );
                let spin = if rocket { turn } else { Quat::IDENTITY };
                w.model
                    .draw(queue, Mat4::from_rotation_translation(spin, at));
            }
        }
        for &(effect, left, life) in &fight.effects {
            self.effects.draw(queue, effect, 1.0 - left / life);
        }
        for p in &self.models {
            match &p.rig {
                Some(rig) => {
                    let (nodes, skin) = rig.posed();
                    p.model.draw_posed(backend, queue, p.world, &nodes, &skin)?;
                }
                None => p.model.draw(queue, p.world),
            }
        }
        Ok(())
    }

    fn draw_ground(&self, queue: &mut DrawQueue) {
        let (vb, ib, material) = self.ground;
        queue.push(Primitive {
            call: DrawCall {
                material,
                topology: Topology::TriangleList,
                vertex_buffer: vb,
                geometry: Geometry::Indexed {
                    buffer: ib,
                    first: 0,
                    count: 6,
                    base_vertex: 0,
                },
                world: Mat4::IDENTITY,
                skin: None,
                instances: None,
                uv_offset: [0.0; 2],
            },
            layer: Layer::WORLD,
            translucent: false,
            sort_origin: Vec3::ZERO,
            user: 0,
        });
    }
}

/// A 2 km square at height 0 with a grid texture.
/// A unit sphere: an octahedron split three times, 512 faces.
fn sphere() -> (Vec<Vec3>, Vec<[u16; 3]>) {
    let mut points = vec![Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    let mut faces: Vec<[u16; 3]> = vec![
        [0, 2, 4],
        [2, 1, 4],
        [1, 3, 4],
        [3, 0, 4],
        [2, 0, 5],
        [1, 2, 5],
        [3, 1, 5],
        [0, 3, 5],
    ];
    for _ in 0..3 {
        let mut middle = std::collections::HashMap::new();
        let mut mid = |a: u16, b: u16, points: &mut Vec<Vec3>| -> u16 {
            *middle.entry((a.min(b), a.max(b))).or_insert_with(|| {
                points.push((points[usize::from(a)] + points[usize::from(b)]).normalize());
                (points.len() - 1) as u16
            })
        };
        faces = faces
            .iter()
            .flat_map(|&[a, b, c]| {
                let (ab, bc, ca) = (
                    mid(a, b, &mut points),
                    mid(b, c, &mut points),
                    mid(c, a, &mut points),
                );
                [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]
            })
            .collect();
    }
    (points, faces)
}

/// A small sphere and the materials shots, flashes,
/// fire and smoke are drawn with, stretched and scaled per effect.
#[derive(Debug)]
struct Effects {
    mesh: (VertexBufferHandle, IndexBufferHandle),
    /// Indices in the mesh.
    count: u32,
    /// Tracer, muzzle flash, fire, smoke.
    light: MaterialId,
    flash: MaterialId,
    fire: MaterialId,
    smoke: MaterialId,
}

impl Effects {
    fn new(backend: &mut dyn RenderBackend, materials: &mut MaterialCache) -> anyhow::Result<Self> {
        use VertexFormat::*;
        use VertexSemantic::*;
        let layout = VertexLayout::packed(&[(Position, Float3), (Normal, Float3)])?;
        let (points, faces) = sphere();
        let vertices: Vec<u8> = points
            .iter()
            .flat_map(|p| [p.x, p.y, p.z, p.x, p.y, p.z])
            .flat_map(f32::to_le_bytes)
            .collect();
        let vb = backend.create_vertex_buffer(
            &VertexBufferDesc {
                layout,
                vertex_count: points.len() as u32,
                usage: BufferUsage::Static,
            },
            Some(&vertices),
        )?;
        let indices: Vec<u8> = faces
            .iter()
            .flatten()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let ib = backend.create_index_buffer(
            &IndexBufferDesc {
                format: IndexFormat::U16,
                index_count: faces.len() as u32 * 3,
                usage: BufferUsage::Static,
            },
            Some(&indices),
        )?;
        let mut define = |color: [f32; 4], lit: bool, blend: BlendMode| {
            materials.define(
                backend,
                &MaterialDesc {
                    color,
                    lit,
                    blend,
                    double_sided: true,
                    // Fire and smoke fade out towards their outline.
                    soft_edge: blend == BlendMode::Translucent,
                    ..MaterialDesc::default()
                },
            )
        };
        Ok(Self {
            mesh: (vb, ib),
            count: faces.len() as u32 * 3,
            light: define([1.0, 0.85, 0.45, 1.0], false, BlendMode::Opaque)?,
            flash: define([1.0, 0.6, 0.18, 0.8], false, BlendMode::Translucent)?,
            fire: define([1.0, 0.5, 0.1, 0.85], false, BlendMode::Translucent)?,
            smoke: define([0.35, 0.35, 0.35, 0.4], true, BlendMode::Translucent)?,
        })
    }

    /// Queues `effect`, `age` of the way (0 to 1) through its life.
    fn draw(&self, queue: &mut DrawQueue, effect: Effect, age: f32) {
        if let Effect::Flash { at, dir, size } = effect {
            // A narrow plume out along the barrel and a small glow at the
            // muzzle, both soft-edged, shrinking as they go.
            let dir = dir.normalize_or(Vec3::Z);
            let turn = Quat::from_rotation_arc(Vec3::Z, dir);
            let fade = 1.0 - 0.5 * age;
            let reach = size * fade;
            let plume = Mat4::from_scale_rotation_translation(
                Vec3::new(reach * 0.16, reach * 0.16, reach * 0.5),
                turn,
                at + dir * (reach * 0.5),
            );
            let glow = Mat4::from_scale_rotation_translation(
                Vec3::splat(reach * 0.14),
                turn,
                at + dir * (reach * 0.08),
            );
            self.push(queue, plume, self.flash, true);
            self.push(queue, glow, self.flash, true);
            return;
        }
        let (world, material, translucent) = match effect {
            Effect::Tracer { from, to } => {
                let along = to - from;
                let len = along.length();
                if len < 0.01 {
                    return;
                }
                let turn = Quat::from_rotation_arc(Vec3::Z, along / len);
                (
                    Mat4::from_scale_rotation_translation(
                        Vec3::new(0.02, 0.02, len * 0.5),
                        turn,
                        (from + to) * 0.5,
                    ),
                    self.light,
                    false,
                )
            }
            Effect::Flash { .. } => return,
            Effect::Fire { at, size } => (
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(size * (0.4 + age)),
                    Quat::IDENTITY,
                    at,
                ),
                self.fire,
                true,
            ),
            Effect::Smoke { at, size } => (
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(size * (0.5 + age)),
                    Quat::IDENTITY,
                    at + Vec3::Y * (age * size * 1.5),
                ),
                self.smoke,
                true,
            ),
        };
        // Smoke as nested shells, fainter outwards, for a softer edge.
        let shells: &[f32] = if material == self.smoke {
            &[0.55, 0.8, 1.0]
        } else {
            &[1.0]
        };
        for &shell in shells {
            self.push(
                queue,
                world * Mat4::from_scale(Vec3::splat(shell)),
                material,
                translucent,
            );
        }
    }

    /// Queues the sphere at `world` in `material`.
    fn push(&self, queue: &mut DrawQueue, world: Mat4, material: MaterialId, translucent: bool) {
        let (vb, ib) = self.mesh;
        queue.push(Primitive {
            call: DrawCall {
                material,
                topology: Topology::TriangleList,
                vertex_buffer: vb,
                geometry: Geometry::Indexed {
                    buffer: ib,
                    first: 0,
                    count: self.count,
                    base_vertex: 0,
                },
                world,
                skin: None,
                instances: None,
                uv_offset: [0.0; 2],
            },
            layer: Layer::WORLD,
            translucent,
            sort_origin: world.transform_point3(Vec3::ZERO),
            user: 0,
        });
    }
}

fn ground(
    backend: &mut dyn RenderBackend,
    materials: &mut MaterialCache,
) -> anyhow::Result<(VertexBufferHandle, IndexBufferHandle, MaterialId)> {
    use VertexFormat::*;
    use VertexSemantic::*;
    let layout =
        VertexLayout::packed(&[(Position, Float3), (Normal, Float3), (TexCoord(0), Float2)])?;
    let (h, t) = (GROUND_HALF, GROUND_HALF / GRID_TILE);
    let corners = [
        (-h, h, 0.0, 0.0),
        (h, h, t, 0.0),
        (h, -h, t, t),
        (-h, -h, 0.0, t),
    ];
    let vertices: Vec<u8> = corners
        .iter()
        .flat_map(|&(x, z, u, v)| [x, 0.0, z, 0.0, 1.0, 0.0, u, v])
        .flat_map(f32::to_le_bytes)
        .collect();
    let vb = backend.create_vertex_buffer(
        &VertexBufferDesc {
            layout,
            vertex_count: 4,
            usage: BufferUsage::Static,
        },
        Some(&vertices),
    )?;
    // Clockwise seen from above.
    let indices: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let ib = backend.create_index_buffer(
        &IndexBufferDesc {
            format: IndexFormat::U16,
            index_count: 6,
            usage: BufferUsage::Static,
        },
        Some(&indices),
    )?;
    const SIZE: u32 = 128;
    let texels: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let line = x < 2 || y < 2;
            if line {
                [90, 96, 90, 255]
            } else {
                [150, 160, 140, 255]
            }
        })
        .collect();
    let mips = generate_mips(SIZE, SIZE, &texels);
    let mips: Vec<&[u8]> = mips.iter().map(Vec::as_slice).collect();
    let texture = backend.create_texture(
        &TextureDesc {
            width: SIZE,
            height: SIZE,
            format: TextureFormat::Rgba8Srgb,
        },
        &mips,
    )?;
    let material = materials.define(
        backend,
        &MaterialDesc {
            texture: Some(texture),
            ..MaterialDesc::default()
        },
    )?;
    Ok((vb, ib, material))
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_render::NullBackend;

    #[test]
    fn ground_draws_through_the_null_backend() {
        let mut backend = NullBackend::new();
        let mut scene = Scene::new(&mut backend).unwrap();
        let mut queue = DrawQueue::new();
        let view = m2_render::View::default();
        backend.begin_frame(&view).unwrap();
        scene
            .draw(
                &mut backend,
                &mut queue,
                &view,
                &crate::combat::Combat::default(),
            )
            .unwrap();
        queue.assign_keys(&view);
        queue.sort().unwrap();
        assert_eq!(queue.submit(&mut backend).unwrap().draws, 1);
        backend.end_frame().unwrap();
        assert_eq!(backend.live_buffers().texture, 1);
    }
}

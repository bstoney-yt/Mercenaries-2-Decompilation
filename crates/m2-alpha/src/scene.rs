//! What is drawn: the streamed island, or (with `--model`) a ground grid
//! with models from the install standing on it.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

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
use crate::flights::{FlightModel, Flights};
use crate::hero::{self, Hero};
use crate::npc::{self, People, PersonKind};
use crate::player::Player;
use crate::props::Props;

/// Jennifer Mui, the playable mercenary whose model the README's export
/// examples use.
pub const DEFAULT_MODEL: u32 = 0x15d8_9832;

/// Where the island opens: the first town, as in the viewer's examples.
pub const DEFAULT_SPAWN: (f32, f32) = (450.0, -1950.0);
/// Where free roam starts without `--spawn`: where the scripts put the
/// player once the first contract is done (VzaCon001's flow binding
/// teleports her to the next contract's start, by the PMC's offices).
pub const FREE_ROAM_START: &str = "PmcCon001_Start1";

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
    /// The clips' events met and not yet taken: (clip, event).
    pub cues: Vec<(usize, usize)>,
    /// A hero's: the name hashes of the clips of the hero's own group.
    pub own: Arc<HashSet<u32>>,
}

/// Most clip events a rig keeps for the taking (older ones are dropped:
/// nobody took them).
const CUES_KEPT: usize = 16;

/// For `hero`: the name hashes of the clips in blocks everyone shares or
/// in the hero's own group, and of those in the own group.
fn hero_clips(source: &AssetSource<BufReader<File>>, hero: Hero) -> (HashSet<u32>, HashSet<u32>) {
    let index = source.archive().index();
    let group = hero.clip_group();
    let mut allowed = HashSet::new();
    let mut own = HashSet::new();
    for a in index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == m2_assets::animation::ANIMATION_TYPE)
    {
        let path = index.blocks()[usize::from(a.block)]
            .path
            .to_ascii_lowercase();
        if hero::block_for(&path, hero) {
            allowed.insert(a.name_hash);
        }
        if path.contains(&group) {
            own.insert(a.name_hash);
        }
    }
    (allowed, own)
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
    /// Reads model `hash`'s skeleton and every clip that fits it. For a
    /// `hero` other than Jennifer, the gaits come from the clips everyone shares and the
    /// hero's own group (not the other heroes'), and [`Rig::own`] holds
    /// the own group's clips; every clip stays loaded, for the clips
    /// picked by name.
    pub fn load(
        source: &mut AssetSource<BufReader<File>>,
        hash: u32,
        gait: Gait,
        hero: Option<Hero>,
    ) -> anyhow::Result<Self> {
        let started = std::time::Instant::now();
        let skeleton = Skeleton::from_nodes(&source.model(hash)?.parse()?.nodes);
        let clips = load_clips(source, &skeleton, &[""])?;
        let rig = match hero {
            // Jennifer's gaits and clips were matched against her footage
            // from every clip (rounds 55-153): they stay as they were.
            Some(hero) if hero != Hero::Jennifer => {
                let (allowed, own) = hero_clips(source, hero);
                let mut rig = Rig::new(skeleton, Vec::new(), gait);
                rig.locomotion =
                    Locomotion::choose_where(&clips, |i| allowed.contains(&clips[i].hash));
                rig.clips = Arc::new(clips);
                rig.own = Arc::new(own);
                rig.set_gait(gait);
                rig
            }
            _ => Rig::new(skeleton, clips, gait),
        };
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
                cues: Vec::new(),
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
            own: Default::default(),
            gait: Gait::Idle,
            cues: Vec::new(),
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
            cues: Vec::new(),
            own: Default::default(),
        };
        rig.set_gait(gait);
        rig
    }

    /// The same clips with the gaits picked from all of them and no own
    /// group: what the people's rigs are made from, whoever the hero is.
    pub fn shared(&self) -> Self {
        let mut rig = Self {
            locomotion: Locomotion::choose(&self.clips),
            own: Default::default(),
            animator: Animator::default(),
            cues: Vec::new(),
            ..self.clone()
        };
        rig.set_gait(Gait::Idle);
        rig
    }

    /// Plays on `dt` seconds, keeping the clips' events met for the
    /// taking ([`CUES_KEPT`] at most).
    pub fn advance(&mut self, dt: f32) {
        self.animator.events_ahead(dt, &self.clips, &mut self.cues);
        let over = self.cues.len().saturating_sub(CUES_KEPT);
        self.cues.drain(..over);
        self.animator.advance(dt);
    }

    pub fn set_gait(&mut self, gait: Gait) {
        self.gait = gait;
        if let Some(&clip) = self.locomotion.as_ref().and_then(|l| l.clips.get(&gait)) {
            self.animator.play(clip);
        }
    }

    /// Moves at `gait` holding a rifle as `carry` says. Aiming, it is
    /// shouldered: by the whole body standing, by the upper body over the
    /// legs' gait when moving. Running, it is held low across the body in
    /// both hands; otherwise the plain gait carries it low in the right
    /// hand, except that a soldier at the ready standing holds it across
    /// the chest. Falls back to the plain gait without the clips.
    pub fn hold_rifle(&mut self, gait: Gait, clips: &npc::FightClips, carry: Carry) {
        let (whole, upper) = match (gait, carry) {
            (Gait::Idle, Carry::Aim) => (clips.aim, None),
            (_, Carry::Aim) => (None, clips.aim),
            (Gait::Idle, Carry::Ready) => (clips.ready, None),
            (Gait::Run, _) => (clips.run_low, None),
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
        let arm = (carry == Carry::Low && gait != Gait::Idle && whole.is_none())
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

/// A sound one of the clips' events asks for (`fol_grenade_sidethrow`):
/// its name and category (`sound`, `sound_surface`, `sound_weapon`),
/// where, and whether it is the player's own.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub name: String,
    pub category: String,
    pub at: Vec3,
    pub hers: bool,
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
    /// The player on the island.
    pub player: Option<Player>,
    /// Who the player is.
    pub hero: Hero,
    /// What the people's rigs are made from: the hero's skeleton and
    /// clips, with the gaits everyone's clips give ([`Rig::shared`]).
    people_rig: Option<Rig>,
    /// Drivable vehicles near the spawn.
    pub vehicles: Vec<ParkedVehicle>,
    /// Soldiers and civilians.
    pub people: People,
    pub arsenal: Arsenal,
    /// Fuel tanks and barrels that blow up when hit.
    pub explosives: Vec<crate::combat::Explosive>,
    /// What the loaded state layers place (a mission's gates).
    pub props: Props,
    /// The mission scripts' deliveries by helicopter.
    pub flights: Flights,
    /// Air strikes' aircraft and ordnance in flight.
    pub strikes: crate::strikes::Strikes,
    /// Mesh and materials for shots, flashes, fire and smoke.
    effects: Effects,
    vehicle_loads: Option<Receiver<Loaded>>,
    /// The archive's vehicle, person and weapon models, once found.
    pub catalogue: Option<Catalogue>,
    pub install: Option<Install>,
    /// Vehicles loaded on request ([`Scene::load_vehicle`]).
    requests: (Sender<Loaded>, Receiver<Loaded>),
    /// Time spent streaming since this was last cleared: the island's
    /// streamer, vehicles and people, and the state layers' props.
    pub streamed: [Duration; 3],
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
    /// Who rides in it, as its template's `Rider` records say: each
    /// seat's name (`seat_driver`) and its rider's template name
    /// (`Guerilla Soldier`). Not drawn or seated yet.
    pub crew: Vec<(String, String)>,
    /// How far its turret is turned from straight ahead, radians
    /// (anticlockwise from above).
    pub turret: f32,
    /// How far its gun is raised, radians (down is negative).
    pub gun: f32,
    /// How far it has rolled, metres (backwards less), for turning a
    /// tracked vehicle's road wheels; a helicopter's: how far its rotors
    /// have turned, radians.
    pub rolled: f32,
    /// Shells its cannon has fired.
    pub fired: u32,
    /// Damage it has taken, of its [`most_health`](Self::most_health).
    pub damage: f32,
}

/// A vehicle's health where its template gives none.
const VEHICLE_HEALTH: f32 = 500.0;

impl ParkedVehicle {
    /// The damage it takes before it is wrecked: its template's health.
    pub fn most_health(&self) -> f32 {
        self.spec.tuning.health.unwrap_or(VEHICLE_HEALTH).max(1.0)
    }

    /// Its health of 100, as the HUD's health box shows it: none once
    /// wrecked.
    pub fn health(&self) -> f32 {
        if self.wrecked {
            return 0.0;
        }
        (100.0 * (1.0 - self.damage / self.most_health())).clamp(0.0, 100.0)
    }

    /// The shells left in its cannon, which carries as many as its
    /// template's `iClipSize` (the AMX-30's 40, which the HUD counts down
    /// from in the videos); `None` for a gun that doesn't run out.
    pub fn rounds(&self) -> Option<u32> {
        let gun = self.spec.tuning.gun.filter(|_| self.spec.cannon())?;
        Some(gun.rounds.saturating_sub(self.fired))
    }
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

/// The state layers' models load for props within this much further
/// than the island's props placed by name are drawn, metres (a prop's own
/// reach goes by its size, which is known once it has loaded).
const PROP_MARGIN: f32 = 100.0;

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
    WorldVehicle(u32, Vec3, f32, Vec<(String, String)>, VehicleSpec, CpuModel),
    Catalogue(Catalogue),
    /// A person's model: a role's, or a story character's (true).
    Person(PersonModel, CpuModel, Skeleton, bool),
    /// A hand weapon: name, model, muzzle (model space).
    Weapon(&'static str, CpuModel, Vec3),
    /// A model for the helicopters' deliveries (a helicopter's, a supply
    /// crate's): its hash, each node's place at rest, and its rotors.
    Flight(u32, CpuModel, Vec<Mat4>, Vec<m2_assets::vehicle::Rotor>),
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
            hero: Hero::default(),
            people_rig: None,
            vehicles: Vec::new(),
            people: People::default(),
            arsenal: Arsenal::default(),
            explosives: Vec::new(),
            props: Props::default(),
            flights: Flights::default(),
            strikes: crate::strikes::Strikes::default(),
            effects,
            vehicle_loads: None,
            catalogue: None,
            install: None,
            requests: channel(),
            streamed: [Duration::ZERO; 3],
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
                        let r = Rig::load(source, hash, show.gait, None)?;
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
        spawn: Option<(f32, f32)>,
    ) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let mut world = World::open(install, backend, &mut self.materials, settings)
            .context("indexing the island")?;
        // Boats float on the sea.
        world.physics.set_water(world.settings.sea_level);
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
        // Without a place asked for, where the first contract leaves her,
        // facing the way that start point does.
        let start = spawn
            .is_none()
            .then(|| world.index.start(FREE_ROAM_START))
            .flatten();
        let ((x, z), facing) = match (spawn, start) {
            (Some(at), _) => (at, 0.0),
            (None, Some(s)) => {
                let f = s.rotation * Vec3::Z;
                eprintln!(
                    "start: {} at {:.1}, {:.1}, {:.1}",
                    s.name, s.position.x, s.position.y, s.position.z
                );
                ((s.position.x, s.position.z), f.x.atan2(f.z))
            }
            (None, None) => {
                eprintln!("start: no {FREE_ROAM_START} in the layers; the first town instead");
                (DEFAULT_SPAWN, 0.0)
            }
        };
        let y = world.index.rough_height(x, z) + 40.0;
        self.focus = (Vec3::new(x, y, z), Vec3::new(x, y - 20.0, z + 60.0));
        self.world = Some(world);

        // The hero, to walk the island as.
        self.open(install)?;
        self.player = Some(Player::new(x, z, facing, None));
        self.load_hero(backend, self.hero);
        self.vehicle_loads = Some(load_cast(install.clone()));
        self.install = Some(install.clone());
        Ok(())
    }

    /// The hero's own rifle: the gun their template's `EquipmentLink`s
    /// hand them.
    pub fn hero_rifle(&self) -> Option<crate::combat::Rifle> {
        let db = &self.world.as_ref()?.index.template_db;
        let (name, gun) = db.own_rifle(self.hero.template())?;
        Some(crate::combat::Rifle::of(name, &gun))
    }

    /// What blasts and rounds do to the hero, by the game's damage table's
    /// `Hero` row (all three heroes' `ModifierKey`).
    pub fn hero_damage(&self) -> crate::combat::HeroDamage {
        let db = self.world.as_ref().map(|w| &w.index.template_db);
        let scale = |key: &str| db.and_then(|db| db.damage_scale("Hero", key));
        let known = crate::combat::HeroDamage::default();
        crate::combat::HeroDamage {
            explosion: scale("Explosion").unwrap_or(known.explosion),
            explosion_large: scale("ExplosionLarge").unwrap_or(known.explosion_large),
            bullet_large: scale("BulletLarge").unwrap_or(known.bullet_large),
        }
    }

    /// The hero's `Health` when whole, from their template.
    pub fn hero_health(&self) -> Option<f32> {
        self.world
            .as_ref()?
            .index
            .template_db
            .health(self.hero.template())
            .filter(|&h| h > 0.0)
    }

    /// Makes `hero` the player: the body and clips their template and
    /// clip group give, in place of whoever was played (where they stand
    /// and what they hold stay). Without a model the player walks unseen.
    pub fn load_hero(&mut self, backend: &mut dyn RenderBackend, hero: Hero) {
        self.hero = hero;
        let Some(source) = self.source.as_mut() else {
            return;
        };
        let template = self
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.get(hero.template()))
            .and_then(|t| t.model);
        let hash = template.unwrap_or(DEFAULT_MODEL);
        eprintln!(
            "hero: {hero}, model {hash:08x}{}",
            if template.is_some() {
                ""
            } else {
                " (no template: Jennifer's)"
            }
        );
        let model = match GpuModel::load(backend, source, &mut self.materials, hash, 0)
            .map_err(anyhow::Error::from)
            .and_then(|gpu| Ok((gpu, Rig::load(source, hash, Gait::Idle, Some(hero))?)))
        {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("player model: {e:#}");
                None
            }
        };
        let clips = model
            .as_ref()
            .map(|(_, rig)| npc::FightClips::find(rig).own_group(rig))
            .unwrap_or_default();
        self.people_rig = model.as_ref().map(|(_, rig)| rig.shared());
        if let Some((_, rig)) = &model {
            let h = |i: Option<usize>| {
                i.map_or("none".to_owned(), |i| {
                    let c = &rig.clips[i];
                    let own = if rig.own.contains(&c.hash) {
                        " own"
                    } else {
                        ""
                    };
                    format!("{:08x}{own}", c.hash)
                })
            };
            eprintln!(
                "hero: {hero} clips: aim {}, ready {}, run {}, jump {}, take-off {}/{}, land {}/{}, throw {}, bash {}/{}, die {}",
                h(clips.aim),
                h(clips.ready),
                h(clips.run_low),
                h(clips.jump),
                h(clips.takeoff),
                h(clips.takeoff_still),
                h(clips.land),
                h(clips.land_still),
                h(clips.throw.map(|t| t.0)),
                h(clips.bash.map(|t| t.0)),
                h(clips.bash_low.map(|t| t.0)),
                h(clips.die),
            );
        }
        if let Some((_, rig)) = &model {
            let named = |c: Option<(usize, f32)>| {
                c.map_or("none".to_owned(), |(i, at)| {
                    let clip = &rig.clips[i];
                    format!(
                        "{:08x} ({:.2} s, the blow at {at:.2} s)",
                        clip.hash, clip.info.duration
                    )
                })
            };
            eprintln!(
                "player bash clips: high {}, low {}",
                named(clips.bash),
                named(clips.bash_low)
            );
            // Every clip with a blow, by how near its start is to the
            // rifle held ready (for `--bash-clip`).
            for (event, list) in npc::FightClips::by_posture(rig) {
                let shown: Vec<String> = list
                    .iter()
                    .map(|&(i, at, off)| {
                        let c = &rig.clips[i];
                        format!(
                            "{:08x} ({:.2} s, {at:.2}, off {off:.2})",
                            c.hash, c.info.duration
                        )
                    })
                    .collect();
                eprintln!("player {event} clips: {}", shown.join(", "));
            }
        }
        self.people.clips = model
            .as_ref()
            .map(|(_, rig)| npc::FightClips::find(rig))
            .unwrap_or_default();
        let templates = self.world.as_ref().map(|w| &w.index.template_db);
        let speeds = templates
            .and_then(|t| {
                Some((
                    t.speeds(hero.template())?,
                    t.speeds(Hero::Jennifer.template())?,
                ))
            })
            .map_or(crate::player::SPEEDS, |(theirs, hers)| {
                Player::hero_speeds(theirs, hers)
            });
        eprintln!(
            "hero: {hero} walks {:.2}, runs {:.2}, sprints {:.2} m/s",
            speeds[0], speeds[1], speeds[2]
        );
        if let Some(player) = &mut self.player {
            player.model = model;
            player.clips = clips;
            player.speeds = speeds;
        }
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
    pub fn load_world_vehicle(
        &self,
        entity: u32,
        at: Vec3,
        yaw: f32,
        crew: Vec<(String, String)>,
        models: Vec<CatalogueEntry>,
    ) {
        self.load_vehicle_as(models, format!("vehicle {entity:#x}"), move |spec, cpu| {
            Loaded::WorldVehicle(entity, at, yaw, crew, spec, cpu)
        });
    }

    /// Loads model `hash` for the helicopters' deliveries on a thread
    /// ([`Flights::loaded`] takes it).
    fn load_flight_model(&self, hash: u32) {
        let Some(install) = self.install.clone() else {
            return;
        };
        let tx = self.requests.0.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let mut source = install.open_world()?;
                let data = source.model(hash)?;
                let model = data.parse()?;
                let rest = (0..model.nodes.len())
                    .map(|n| model.node_to_model(n as u16).unwrap_or(Mat4::IDENTITY))
                    .collect();
                let rotors = m2_assets::vehicle::rotors(&model);
                let cpu = CpuModel::build(hash, &model, 0);
                let _ = tx.send(Loaded::Flight(hash, cpu, rest, rotors));
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("delivery model {hash:08x}: {e:#}");
            }
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
        let mut lap = Instant::now();
        let mut split = |i: usize, streamed: &mut [Duration; 3]| {
            streamed[i] += lap.elapsed();
            lap = Instant::now();
        };
        if let Some(world) = &mut self.world {
            world.update(backend, &mut self.materials, eye)?;
        }
        split(0, &mut self.streamed);
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
        self.strikes.want_models(&mut self.flights);
        for hash in self.flights.take_wanted() {
            self.load_flight_model(hash);
        }
        split(1, &mut self.streamed);
        if let (Some(world), Some(source)) = (&mut self.world, &mut self.source) {
            // As far as most are drawn, and a little more.
            let reach = world.settings.prop_distance + PROP_MARGIN;
            self.props.update(
                backend,
                &mut self.materials,
                source,
                &mut world.physics,
                eye,
                reach,
            )?;
        }
        split(2, &mut self.streamed);
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
            | Loaded::WorldVehicle(_, _, _, _, _, cpu)
            | Loaded::Person(_, cpu, _, _)
            | Loaded::Weapon(_, cpu, _)
            | Loaded::Flight(_, cpu, _, _) => cpu,
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
                crew: Vec::new(),
                turret: 0.0,
                gun: 0.0,
                rolled: 0.0,
                fired: 0,
                damage: 0.0,
            }),
            Loaded::MissionVehicle(guid, spec, _) => {
                eprintln!("mission vehicle {guid:#x}: {}", spec.label);
                let crew = self
                    .world
                    .as_ref()
                    .map(|w| w.index.template_db.crew(&spec.label))
                    .unwrap_or_default();
                self.vehicles.push(ParkedVehicle {
                    spec: tune(spec),
                    model,
                    id: None,
                    wrecked: false,
                    guid: Some(guid),
                    spot: None,
                    crew,
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                    fired: 0,
                    damage: 0.0,
                });
            }
            Loaded::WorldVehicle(entity, at, yaw, crew, spec, _) => {
                self.vehicles.push(ParkedVehicle {
                    spec: tune(spec),
                    model,
                    id: None,
                    wrecked: false,
                    guid: None,
                    spot: Some((entity, at, yaw)),
                    crew,
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                    fired: 0,
                    damage: 0.0,
                });
            }
            Loaded::Flight(hash, _, rest, rotors) => {
                self.flights.loaded(
                    hash,
                    FlightModel {
                        gpu: model,
                        rest,
                        rotors,
                    },
                );
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
                // The hero's clips fit every human skeleton.
                let template = self
                    .people_rig
                    .as_ref()
                    .and_then(|rig| rig.for_skeleton(skeleton));
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

    /// The sound events the clips have met since last asked: the
    /// player's, and those of the living within `reach` of `listener`;
    /// the others' are dropped.
    pub fn take_cues(&mut self, listener: Vec3, reach: f32) -> Vec<Cue> {
        let mut out = Vec::new();
        let mut take = |rig: &mut Rig, at: Vec3, hers: bool, heard: bool| {
            for (c, e) in rig.cues.drain(..) {
                let Some(ev) = rig.clips.get(c).and_then(|c| c.clip.events.get(e)) else {
                    continue;
                };
                if heard && ev.category.to_ascii_lowercase().starts_with("sound") {
                    out.push(Cue {
                        name: ev.name.clone(),
                        category: ev.category.clone(),
                        at,
                        hers,
                    });
                }
            }
        };
        if let Some(p) = &mut self.player
            && let Some((_, rig)) = &mut p.model
        {
            take(rig, p.character.position + Vec3::Y, true, true);
        }
        for n in &mut self.people.npcs {
            let at = n.character.position + Vec3::Y;
            let heard = n.alive && at.distance(listener) <= reach;
            take(&mut n.rig, at, false, heard);
        }
        out
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
            self.props.draw(
                queue,
                &world.physics,
                eye,
                &frustum,
                world.settings.object_distance,
            );
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
                let mut nodes = v.spec.posed_nodes(
                    &world.physics.vehicle_wheels(id),
                    v.rolled,
                    v.turret,
                    v.gun,
                );
                v.spec.spin_rotors(&mut nodes, v.rolled);
                v.model.draw_posed(backend, queue, at, &nodes, &[])?;
            }
        }
        let hanging = self.flights.draw(queue, &self.vehicles);
        self.strikes.draw(queue, &self.flights);
        for (hook, load) in hanging.cables {
            self.effects.cable(queue, hook, load);
        }
        for (i, at) in hanging.hung {
            let v = &self.vehicles[i];
            let nodes = v.spec.posed_nodes(&[], v.rolled, v.turret, v.gun);
            v.model.draw_posed(backend, queue, at, &nodes, &[])?;
        }
        if let Some(player) = &self.player
            && player.landed
            && (player.driving.is_none() || player.seated)
            && let Some((model, rig)) = &player.model
        {
            let (nodes, skin) = rig.posed();
            let body = player.model_matrix();
            model.draw_posed(backend, queue, body, &nodes, &skin)?;
            // Throwing, her weapon is put away and the grenade is in her
            // hand until it goes; calling a support strike, the
            // designator is. Her rifle is her own or the carbine
            // ([`crate::combat::Rifle`]).
            let weapon = match (player.throwing, fight.weapon) {
                _ if fight.designator => self.arsenal.designator.as_ref(),
                (Some(_), _) if fight.grenade_in_hand() => self.arsenal.grenade.as_ref(),
                (Some(_), _) => None,
                (None, Weapon::Rifle) => self.arsenal.hers(&fight.rifle),
                (None, Weapon::Rpg) => self.arsenal.rpg.as_ref(),
                (None, Weapon::GrenadeLauncher) => self.arsenal.launcher.as_ref(),
            };
            if !player.down
                && !player.seated
                && let Some(w) = weapon
                && let Some(at) = self.arsenal.in_hand(
                    w,
                    body,
                    &nodes,
                    (self.arsenal.hand, self.arsenal.attach),
                    rig.two_handed && !fight.designator,
                )
            {
                w.model.draw(queue, at);
            }
            // What she carries but doesn't hold hangs on her back.
            let holding = !fight.designator && player.throwing.is_none() && !player.down;
            for (slot, slung) in fight.slung(holding).into_iter().enumerate() {
                let model = match slung {
                    Some(Weapon::Rifle) => self.arsenal.hers(&fight.rifle),
                    Some(Weapon::Rpg) => self.arsenal.rpg.as_ref(),
                    Some(Weapon::GrenadeLauncher) => self.arsenal.launcher.as_ref(),
                    None => None,
                };
                if let Some(w) = model
                    && let Some(at) = self.arsenal.on_back(w, body, &nodes, slot)
                {
                    w.model.draw(queue, at);
                }
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
        for (what, at, velocity) in fight.projectiles() {
            let model = match what {
                combat::Flying::Rocket => &self.arsenal.rocket,
                combat::Flying::Grenade => &self.arsenal.grenade,
                combat::Flying::Round => match &self.arsenal.round {
                    Some(_) => &self.arsenal.round,
                    None => &self.arsenal.grenade,
                },
            };
            if let Some(w) = model {
                // A rocket's nose (its muzzle marker) along its flight, and
                // a launched round's; a thrown grenade tumbles as it was.
                let turn = Quat::from_rotation_arc(
                    w.muzzle.normalize_or(Vec3::Z),
                    velocity.normalize_or(Vec3::Z),
                );
                let spin = if what == combat::Flying::Grenade {
                    Quat::IDENTITY
                } else {
                    turn
                };
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
    /// Tracer, muzzle flash, fire, smoke, black smoke.
    light: MaterialId,
    flash: MaterialId,
    fire: MaterialId,
    smoke: MaterialId,
    soot: MaterialId,
    /// A boat's spray and wake.
    spray: MaterialId,
    wake: MaterialId,
    /// A helicopter's winch cable.
    cable: MaterialId,
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
            soot: define([0.05, 0.05, 0.05, 0.6], true, BlendMode::Translucent)?,
            spray: define([0.93, 0.95, 0.97, 0.45], true, BlendMode::Translucent)?,
            wake: define([0.9, 0.93, 0.95, 0.35], true, BlendMode::Translucent)?,
            cable: define([0.06, 0.06, 0.06, 1.0], true, BlendMode::Opaque)?,
        })
    }

    /// Queues a winch cable from `from` to `to`.
    fn cable(&self, queue: &mut DrawQueue, from: Vec3, to: Vec3) {
        let along = to - from;
        let len = along.length();
        if len < 0.01 {
            return;
        }
        let turn = Quat::from_rotation_arc(Vec3::Z, along / len);
        let world = Mat4::from_scale_rotation_translation(
            Vec3::new(0.03, 0.03, len * 0.5),
            turn,
            (from + to) * 0.5,
        );
        self.push(queue, world, self.cable, false);
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
            Effect::Soot { at, size } => (
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(size * (0.6 + 1.4 * age)),
                    Quat::IDENTITY,
                    at + Vec3::Y * (age * size * 4.0),
                ),
                self.soot,
                true,
            ),
            // Out and up, falling back as it goes, gone at the end.
            Effect::Spray { at, drift, size } => (
                Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 0.6, 1.0) * (size * (0.4 + age) * (1.0 - age * age)),
                    Quat::IDENTITY,
                    at + drift * age - Vec3::Y * (2.0 * age * age),
                ),
                self.spray,
                true,
            ),
            Effect::Wake { at, size } => (
                Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 0.04, 1.0) * (size * (0.5 + age) * (1.0 - age.powi(3))),
                    Quat::IDENTITY,
                    at,
                ),
                self.wake,
                true,
            ),
        };
        // Smoke as nested shells, fainter outwards, for a softer edge.
        let shells: &[f32] = if material == self.smoke || material == self.soot {
            &[0.55, 0.8, 1.0]
        } else if material == self.spray {
            &[0.6, 1.0]
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

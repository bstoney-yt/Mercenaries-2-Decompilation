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
use m2_engine::vehicle_damage::PartDamage;
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
use crate::held;
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
/// Where a mission starts without `--spawn`: the first contract's own
/// spawn point, which its scripts name as they start (`SetSpawnLocations
/// VzaCon001_Start1`), by the speed boat they seat her in. From the town
/// she stayed the boat's 4 km off, hibernating (beyond 155 m), and the
/// scripts never seated her.
pub const MISSION_START: &str = "VzaCon001_Start1";

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
    /// A hero's: the name hashes of the clips only in the other heroes'
    /// own groups, which this hero never plays.
    pub foreign: Arc<HashSet<u32>>,
    /// A hero's: the clip the hero aims with ([`Rig::set_aim_clip`]),
    /// which the weapons' grip and the aim's hands are read from. `None`:
    /// the aim clip by name.
    pub aim_clip: Option<usize>,
    /// Whether an aim is turned along [`Rig::aim`] by the rifle's barrel
    /// (rear hand to front): not with a pistol up, held in one hand
    /// (round 13: turned so, the one-handed aim bent her double).
    pub barrel: bool,
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

/// The gaits' keys in the on-foot animation table: the clip of each gait
/// Jennifer's rig plays (chosen by speed from every clip, PC logs of
/// 2026-10-08: "model 15d89832 ... locomotion"), whose first row gives
/// the gait's animation handle ([`m2_assets::animation_table::character_clip`]):
/// idle `5aac9b8d`, walk `695f945a`, run `b99ad3b3` (the run's, as the
/// rifle run's), sprint `f71c4238` (PC, 2026-10-09). That those handles
/// are the gaits is inferred from these clips' speeds alone: the
/// handles' names are not known.
const GAIT_KEYS: [(Gait, u32); 4] = [
    (Gait::Idle, 0x01ca_8a69),
    (Gait::Walk, 0x067a_b57f),
    (Gait::Run, 0x1091_2665),
    (Gait::Sprint, 0x076a_6136),
];

/// What the on-foot table settled for a hero ([`table_clips`]).
#[derive(Debug, Default)]
struct TableClips {
    /// The clips it gave, which the own group's leave be.
    settled: HashSet<usize>,
    /// By slot name ([`npc::FightClips::slots`]): the row each clip it
    /// gave came from, for the log.
    from: HashMap<&'static str, String>,
}

/// The clips the on-foot animation table (`e00b080c`) gives `hero`, a
/// hero holding a rifle (`set`: the template's `HumanAnimationSet`
/// gender and character name): for each slot of `clips` and each gait
/// of `rig`, its key's handle ([`npc::FightClips::TABLE_KEYS`],
/// [`GAIT_KEYS`]), then the hero's own row for the handle with a rifle,
/// else anyone's with a rifle, else anyone's
/// ([`animation_table::character_clip`]), of the clips on `rig` and not
/// another hero's. Those replace the slots and gaits; the rest keep
/// what they were picked by (by name, by speed, or the own group's
/// after this). Jennifer's footage-matched picks stay: hers are taken
/// only where the table gives her the clip she has, and logged where it
/// doesn't. Every gait is logged here, each slot by
/// [`log_hero_clips`]: "hero clips: <hero>: <slot> <hash> from row ..."
/// or "... from fallback ...".
///
/// [`animation_table::character_clip`]: m2_assets::animation_table::character_clip
fn table_clips(
    source: &mut AssetSource<BufReader<File>>,
    set: Option<(u32, u32)>,
    hero: Hero,
    rig: &mut Rig,
    clips: &mut npc::FightClips,
) -> TableClips {
    use m2_assets::animation_table::{self as table, AnimationTable, Precedence};
    let read = source
        .asset(table::CHARACTER_ANIMATIONS, table::ANIMATION_TABLE_TYPE)
        .map_err(anyhow::Error::from)
        .and_then(|a| Ok(AnimationTable::parse(&a)?));
    let t = match read {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "hero clips: {hero}: no on-foot table {:08x}: {e:#}",
                table::CHARACTER_ANIMATIONS
            );
            return TableClips::default();
        }
    };
    let columns: Vec<String> = t
        .columns
        .iter()
        .map(|c| format!("{} {:?}", c.name, c.kind))
        .collect();
    let listed = t.column(table::ANIMATION).map_or(0, |list| {
        (1..t.rows())
            .filter(|&r| !t.list(r, list).is_empty())
            .count()
    });
    eprintln!(
        "hero clips: the on-foot table {:08x}: {} rows ({listed} listing clips), columns {}",
        table::CHARACTER_ANIMATIONS,
        t.rows(),
        columns.join(", ")
    );
    // The template's, else the values the heroes' templates hold
    // (`Hero`, the character's name; inferred from Mattias's).
    let (gender, name) = set.unwrap_or_else(|| {
        eprintln!(
            "hero clips: {hero}: no HumanAnimationSet for the template: gender Hero, character {}",
            hero.name()
        );
        (
            m2_assets::name_hash("Hero"),
            m2_assets::name_hash(hero.name()),
        )
    });
    let index = |hash: u32| {
        rig.clips
            .iter()
            .position(|c| c.hash == hash && !rig.foreign.contains(&hash))
    };
    let resolve = |key: u32| {
        table::character_clip(&t, key, (gender, name), table::RIFLE, |c| {
            index(c).is_some()
        })
    };
    let whose = match hero {
        Hero::Jennifer => "her",
        _ => "his",
    };
    let row = |p: &table::CharacterClip| {
        format!(
            "row {} (handle {:08x}, {})",
            p.row,
            p.handle,
            match p.by {
                Precedence::Own => format!("{whose} own rifle row"),
                Precedence::Equipment => "anyone's rifle row".to_owned(),
                Precedence::OwnAny => format!("{whose} own row"),
                Precedence::Handle => "anyone's row".to_owned(),
            }
        )
    };
    let jennifer = hero == Hero::Jennifer;
    let hash = |i: Option<usize>| i.map(|i| rig.clips[i].hash);
    // Each key's slot clip as picked by name, for Jennifer's to stay.
    let had: HashMap<u32, Option<u32>> = npc::FightClips::TABLE_KEYS
        .iter()
        .zip(clips.slots())
        .map(|(&(_, key), (_, i))| (key, hash(i)))
        .collect();
    let theirs = |key: u32| {
        let p = resolve(key)?;
        (!jennifer || had.get(&key).copied().flatten() == Some(p.clip)).then_some(p.clip)
    };
    let mut picked = *clips;
    let settled = picked.by_table(rig, &theirs);
    let mut from = HashMap::new();
    for (&(slot, key), (_, now)) in npc::FightClips::TABLE_KEYS.iter().zip(picked.slots()) {
        match (resolve(key), now) {
            (Some(p), Some(i)) if settled.contains(&i) && rig.clips[i].hash == p.clip => {
                from.insert(slot, row(&p));
            }
            (Some(p), _) if jennifer => eprintln!(
                "hero clips: {hero}: {slot}: the table's {} gives {:08x}: her footage-matched pick stays",
                row(&p),
                p.clip
            ),
            (Some(p), _) => eprintln!(
                "hero clips: {hero}: {slot}: the table's {} gives {:08x}, which lacks the slot's event",
                row(&p),
                p.clip
            ),
            (None, _) => eprintln!(
                "hero clips: {hero}: {slot}: the table gives nothing for {key:08x} (no row, or none on the rig)"
            ),
        }
    }
    *clips = picked;
    // The gaits: worked out first, then put in (the rig is read till then).
    let shown = |h: Option<u32>| h.map_or("none".to_owned(), |h| format!("{h:08x}"));
    let mut gaits = Vec::new();
    for (gait, key) in GAIT_KEYS {
        let g = format!("{gait:?}").to_lowercase();
        let was = rig.gait_clip(gait).map(|i| rig.clips[i].hash);
        match resolve(key).and_then(|p| Some((p, index(p.clip)?))) {
            Some((p, i)) if !jennifer || was == Some(p.clip) => {
                eprintln!("hero clips: {hero}: {g} {:08x} from {}", p.clip, row(&p));
                gaits.push((gait, i));
            }
            Some((p, _)) => eprintln!(
                "hero clips: {hero}: {g} {} from fallback (by speed; the table's {} gives {:08x}: her footage-matched pick stays)",
                shown(was),
                row(&p),
                p.clip
            ),
            None => eprintln!(
                "hero clips: {hero}: {g} {} from fallback (by speed; the table gives nothing for {key:08x})",
                shown(was)
            ),
        }
    }
    if let Some(l) = rig.locomotion.as_mut() {
        l.clips.extend(gaits);
    }
    let gait = rig.gait;
    rig.set_gait(gait);
    TableClips { settled, from }
}

/// The clips `hero` plays for each class of weapon held, a switch and a
/// pickup ([`held::resolve`]), from the on-foot and the stance and
/// action tables; the rifle's are `clips` and the rig's gaits as already
/// picked. Logged: "hero moves: <hero>: <class>: ..." a class, and
/// "hero moves: <hero>: pickup <hash>".
fn hero_moves(
    source: &mut AssetSource<BufReader<File>>,
    set: Option<(u32, u32)>,
    hero: Hero,
    rig: &Rig,
    clips: npc::FightClips,
) -> held::Moves {
    use m2_assets::animation_table::{self as table, AnimationTable};
    let mut read = |hash: u32| {
        source
            .asset(hash, table::ANIMATION_TABLE_TYPE)
            .map_err(anyhow::Error::from)
            .and_then(|a| Ok(AnimationTable::parse(&a)?))
    };
    let (t, actions) = match (read(table::CHARACTER_ANIMATIONS), read(table::ACTIONS)) {
        (Ok(t), Ok(a)) => (t, a),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("hero moves: {hero}: no animation tables: {e:#}");
            return held::Moves::default();
        }
    };
    let who = set.unwrap_or((
        m2_assets::name_hash("Hero"),
        m2_assets::name_hash(hero.name()),
    ));
    let gaits: Vec<(Gait, usize)> = rig.locomotion.as_ref().map_or_else(Vec::new, |l| {
        let mut g: Vec<_> = l.clips.iter().map(|(&g, &i)| (g, i)).collect();
        g.sort_by_key(|(g, _)| format!("{g:?}"));
        g
    });
    let moves = held::resolve(&t, &actions, who, rig, clips, &gaits, &GAIT_KEYS);
    let h =
        |i: Option<usize>| i.map_or("none".to_owned(), |i| format!("{:08x}", rig.clips[i].hash));
    for (held, name) in moves.held.iter().zip(held::CLASSES) {
        let gaits: Vec<String> = held
            .gaits
            .iter()
            .map(|(g, i)| format!("{} {}", format!("{g:?}").to_lowercase(), h(Some(*i))))
            .collect();
        let moves_of = |map: &HashMap<(held::Side, bool), usize>| {
            let mut v: Vec<String> = map
                .iter()
                .map(|(&(side, run), &i)| {
                    let s = if side == held::Side::Right {
                        "right"
                    } else {
                        "left"
                    };
                    let r = if run { "run" } else { "idle" };
                    format!("{s} {r} {}", h(Some(i)))
                })
                .collect();
            v.sort();
            v.join(", ")
        };
        eprintln!(
            "hero moves: {hero}: {name}: aim {}, ready {}, run {}, gaits {}; stow {}; take up {}",
            h(held.clips.aim),
            h(held.clips.ready),
            h(held.clips.run_low),
            gaits.join(", "),
            moves_of(&held.stow),
            moves_of(&held.equip)
        );
    }
    eprintln!("hero moves: {hero}: pickup {}", h(moves.pickup));
    moves
}

/// One line for each of `hero`'s fight clips, once the table's and the
/// own group's are in: "hero clips: <hero>: <slot> <hash> from row <n>
/// (handle <h>, <whose row>)" where the on-foot table gave it, else
/// "... from fallback (<how it was picked>)".
fn log_hero_clips(hero: Hero, rig: &Rig, clips: &npc::FightClips, table: &TableClips) {
    for (slot, i) in clips.slots() {
        let Some(i) = i else {
            eprintln!("hero clips: {hero}: {slot} none");
            continue;
        };
        let c = rig.clips[i].hash;
        match table.from.get(slot) {
            Some(row) if table.settled.contains(&i) => {
                eprintln!("hero clips: {hero}: {slot} {c:08x} from {row}");
            }
            _ => eprintln!(
                "hero clips: {hero}: {slot} {c:08x} from fallback ({})",
                if rig.own.contains(&c) {
                    "the own group's"
                } else {
                    "by name"
                }
            ),
        }
    }
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
    /// hero's own group (not the other heroes'), [`Rig::own`] holds
    /// the own group's clips and [`Rig::foreign`] the other heroes';
    /// every clip stays loaded, for the clips picked by name.
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
                rig.foreign = Arc::new(
                    clips
                        .iter()
                        .map(|c| c.hash)
                        .filter(|h| !allowed.contains(h))
                        .collect(),
                );
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
            foreign: Default::default(),
            aim_clip: None,
            barrel: true,
            gait: Gait::Idle,
            cues: Vec::new(),
        })
    }

    /// Its idle clip in words, and how far it turns on the spot; and the
    /// one picked when a turning clip could be (for checking, playtest 8).
    pub fn idle_line(&self) -> String {
        let turn = |i: usize| {
            let c = &self.clips[i];
            format!("{:08x} (turns {:.0} deg)", c.hash, c.info.turn.to_degrees())
        };
        let Some(now) = self
            .locomotion
            .as_ref()
            .and_then(|l| l.clips.get(&Gait::Idle))
        else {
            return "no idle clip".to_owned();
        };
        let any = Locomotion::choose_turning(&self.clips, |_| true, f32::INFINITY)
            .and_then(|l| l.clips.get(&Gait::Idle).copied());
        let held: Vec<String> = [crate::npc::FightClips::READY, crate::npc::FightClips::AIM]
            .iter()
            .filter_map(|&h| self.clips.iter().position(|c| c.hash == h))
            .map(turn)
            .collect();
        let side_on = self
            .clips
            .iter()
            .position(|c| c.hash == crate::npc::FightClips::AIM)
            .and_then(|i| self.aim_offset(i));
        format!(
            "idle {}; with turning clips allowed, {}; rifle ready and aimed {}; aiming turns the body {} deg with her ahead",
            turn(*now),
            any.map_or("none".to_owned(), turn),
            held.join(", "),
            side_on.map_or("?".to_owned(), |d| format!("{d:.0}"))
        )
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
            foreign: Default::default(),
            aim_clip: None,
            barrel: true,
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
            foreign: Default::default(),
            aim_clip: None,
            barrel: true,
            hands: Default::default(),
            animator: Animator::default(),
            cues: Vec::new(),
            ..self.clone()
        };
        rig.set_gait(Gait::Idle);
        eprintln!("people's rig: {}", rig.idle_line());
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
        let want = self.aim.filter(|_| self.two_handed && self.barrel)?;
        let mut pose = self.animator.pose(&self.skeleton, &self.clips);
        self.point(&mut pose, want);
        Some(pose)
    }

    /// Aims with clip `clip` (the hero's own pick, which for Mattias and
    /// Chris can be the on-foot table's rather than the aim clip by name):
    /// the aim's hands are found again from it.
    pub fn set_aim_clip(&mut self, clip: Option<usize>) {
        self.aim_clip = clip;
        self.hands = Default::default();
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

    /// How far the aim clip `clip`'s rifle points off the way the body
    /// faces, degrees (the clip stands side-on): what the aim turns the
    /// body by with her straight ahead.
    pub fn aim_offset(&self, clip: usize) -> Option<f32> {
        let forward = self
            .locomotion
            .as_ref()
            .map_or(m2_math::glam::Vec2::Y, |l| l.forward);
        let pose = self.clips[clip].sample(&self.skeleton, 0.0);
        let ahead = Vec3::new(forward.x, 0.0, forward.y);
        self.aim_error(&pose, ahead)
            .map(|(yaw, _)| yaw.to_degrees())
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
    /// The vehicles (the mission scripts' objects, the island's entities)
    /// that are ruins of one ([`crate::mission::is_ruin`]): they stand
    /// wrecked when they load.
    pub ruins: std::collections::HashSet<u32>,
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
    /// What the people's rigs are made from, whoever the hero is:
    /// Jennifer's skeleton and every clip that fits it, with the gaits
    /// everyone's clips give ([`Rig::shared`]). Made once: switching hero
    /// leaves the people be.
    people_rig: Option<Rig>,
    /// Drivable vehicles near the spawn.
    pub vehicles: Vec<ParkedVehicle>,
    /// Whether up to [`PARKED`] of the catalogue's vehicles are parked by
    /// her as she starts: not the game's, so not with a mission (the
    /// plain launch's included), where only those the layers and scripts
    /// place stand.
    pub park: bool,
    /// The people the layers place.
    pub people: People,
    /// The vehicles' riders, drawn in their seats
    /// ([`crate::hijack::Rider`]).
    pub riders: Vec<crate::hijack::Rider>,
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
    /// The game's own particle effects, once the island is loaded; the
    /// spheres stand in for fire and smoke without them.
    pub particles: Option<crate::particles::ParticleDraw>,
    /// Draw no fire or smoke at all (`--no-fx`, to measure what they cover).
    pub hide_fx: bool,
    vehicle_loads: Option<Receiver<Loaded>>,
    /// The archive's vehicle, person and weapon models, once found.
    pub catalogue: Option<Catalogue>,
    pub install: Option<Install>,
    /// Vehicles loaded on request ([`Scene::load_vehicle`]).
    requests: (Sender<Loaded>, Receiver<Loaded>),
    /// Chunks wrecks have thrown, flying or lying ([`Chunk`]).
    pub chunks: Vec<Chunk>,
    /// The chunks' models by hash, `None` while one loads.
    chunk_models: HashMap<u32, Option<GpuModel>>,
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
    /// (`Guerilla Soldier`). Drawn in their seats near her
    /// ([`crate::hijack::Rider`]).
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
    /// Rounds its machine gun has fired.
    pub mg_fired: u32,
    /// Damage it has taken, of its [`most_health`](Self::most_health).
    pub damage: f32,
    /// Its parts as they stand: dented, broken, thrown off
    /// ([`m2_engine::vehicle_damage`]), if its model and template say.
    pub parts: Option<PartDamage>,
    /// The parts it has thrown off, falling and lying where they fell.
    pub loose: Vec<LoosePart>,
    /// Whether its wreck has thrown its chunks ([`Chunk`]); a vehicle
    /// loaded wrecked never does.
    pub debris_thrown: bool,
}

/// A part a vehicle threw off (a door, the hood): drawn from the
/// vehicle's model, its own node's pieces alone, where it flew.
#[derive(Clone, Debug, PartialEq)]
pub struct LoosePart {
    /// The vehicle's place when the part came off.
    pub from: Mat4,
    /// The part's middle then, world space.
    pub centre: Vec3,
    /// How far it has moved from there, and how fast it goes.
    pub offset: Vec3,
    pub velocity: Vec3,
    /// What it turns about, and how far it has turned (radians).
    pub axis: Vec3,
    pub angle: f32,
    /// How fast it turns, radians a second.
    pub spin: f32,
    /// The height it comes to rest at (its middle).
    pub ground: f32,
    /// Per node of the vehicle's model, whether it isn't the part's.
    pub hidden: Vec<bool>,
    pub age: f32,
}

/// How a thrown-off part flies: out from the vehicle's middle and up,
/// m/s, turning so many radians a second, and how long it lies before
/// it is gone, seconds. Invented: the game throws it as a loose object
/// of its own (`CreateObject`), with physics this alpha doesn't have.
const LOOSE_OUT: f32 = 2.5;
const LOOSE_UP: f32 = 3.5;
const LOOSE_SPIN: f32 = 5.0;
const LOOSE_LASTS: f32 = 120.0;
/// Thrown-off parts kept a vehicle at most.
const LOOSE_MOST: usize = 12;

impl LoosePart {
    /// Its place now.
    pub fn world(&self) -> Mat4 {
        Mat4::from_translation(self.centre + self.offset)
            * Mat4::from_axis_angle(self.axis, self.angle)
            * Mat4::from_translation(-self.centre)
            * self.from
    }

    /// Flies `dt` on: falls until its middle is down on the ground, then
    /// lies there.
    pub fn step(&mut self, dt: f32) {
        self.age += dt;
        if self.velocity == Vec3::ZERO {
            return;
        }
        self.velocity.y -= 9.81 * dt;
        self.offset += self.velocity * dt;
        self.angle += self.spin * dt;
        if self.centre.y + self.offset.y <= self.ground {
            self.offset.y = self.ground - self.centre.y;
            self.velocity = Vec3::ZERO;
        }
    }
}

/// A chunk a wreck threw (its template's `DebrisEffect`: `Chunk_aa`'s
/// model), flying, then lying where it fell until its time is up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    /// Its template's model.
    pub model: u32,
    pub at: Vec3,
    pub velocity: Vec3,
    /// How it turns as it flies: about `axis`, `spin` radians a second.
    pub axis: Vec3,
    pub angle: f32,
    pub spin: f32,
    pub age: f32,
    /// How long it lasts, seconds (between its record's `MinChunkTime`
    /// and `MaxChunkTime`).
    pub lasts: f32,
}

/// The chunks kept at most, all wrecks' together, the oldest going first
/// (invented: a bound on what is drawn, not the game's).
const CHUNKS_MOST: usize = 64;
/// How steeply chunks leave, radians above level, least and most
/// (invented: the records give a speed, not a direction).
const CHUNK_RISE: (f32, f32) = (0.5, 1.1);

impl Chunk {
    /// Its place now.
    pub fn place(&self) -> Mat4 {
        Mat4::from_translation(self.at) * Mat4::from_axis_angle(self.axis, self.angle)
    }

    /// Flies `dt` on, falling; `ground` gives how far below a point the
    /// ground is within a reach, if it is: there it stops and lies.
    pub fn step(&mut self, dt: f32, ground: impl Fn(Vec3, f32) -> Option<f32>) {
        self.age += dt;
        if self.velocity == Vec3::ZERO {
            return;
        }
        self.velocity.y -= 9.81 * dt;
        let step = self.velocity * dt;
        if step.y < 0.0
            && let Some(d) = ground(self.at, -step.y)
        {
            self.at += step.with_y(-d);
            self.velocity = Vec3::ZERO;
            return;
        }
        self.at += step;
        self.angle += self.spin * dt;
    }
}

/// The chunks a wreck standing at `body` (its box `bounds`, model space)
/// throws, as its template's `DebrisEffect` records say: per record
/// between `iMinChunks` and `iMaxChunks` of its chunk, from the
/// vehicle's middle, leaving at between `MinChunkVel` and `MaxChunkVel`
/// m/s and lasting between `MinChunkTime` and `MaxChunkTime` seconds.
/// Which way each goes ([`CHUNK_RISE`], any heading) and how it turns
/// ([`LOOSE_SPIN`]) are invented; `seed` picks within the ranges.
/// Records whose chunk has no model throw nothing (a fuel can, which is
/// a pickup).
pub fn throw_chunks(
    debris: &[m2_engine::vehicle_damage::Debris],
    body: Mat4,
    bounds: (Vec3, Vec3),
    seed: u32,
) -> Vec<Chunk> {
    // Near seeds spread apart before xorshift32 takes over.
    let mut state = (seed ^ 0x2545_f491).wrapping_mul(0x9e37_79b9) | 1;
    let mut random = move || {
        // xorshift32, in [0, 1).
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / (1u32 << 24) as f32
    };
    for _ in 0..4 {
        random();
    }
    let between = |r: f32, (lo, hi): (f32, f32)| lo + (hi - lo) * r;
    let middle = body.transform_point3((bounds.0 + bounds.1) * 0.5);
    let mut out = Vec::new();
    for d in debris {
        let Some(model) = d.model else { continue };
        let (lo, hi) = d.count;
        let n = lo + ((hi - lo + 1) as f32 * random()) as u32;
        for _ in 0..n.min(hi) {
            let heading = random() * std::f32::consts::TAU;
            let rise = between(random(), CHUNK_RISE);
            let way = Vec3::new(
                heading.cos() * rise.cos(),
                rise.sin(),
                heading.sin() * rise.cos(),
            );
            let axis = Vec3::new(random() - 0.5, 0.5, random() - 0.5).normalize();
            out.push(Chunk {
                model,
                at: middle,
                velocity: way * between(random(), d.speed),
                axis,
                angle: 0.0,
                spin: LOOSE_SPIN,
                age: 0.0,
                lasts: between(random(), d.lasts),
            });
        }
    }
    out
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

    /// A round of `damage` struck it at `at` (world), its place `body`:
    /// the part there takes it.
    pub fn hit_parts(&mut self, body: Mat4, at: Vec3, damage: f32) {
        let (Some(dm), Some(parts)) = (self.spec.damage.as_deref(), self.parts.as_mut()) else {
            return;
        };
        if !self.wrecked {
            parts.hit(dm, body.inverse().transform_point3(at), damage);
        }
    }

    /// A blast at `at` (world), its place `body`, reaching `reach` metres:
    /// its parts in reach take `damage_at` their distance from its heart.
    pub fn blast_parts(
        &mut self,
        body: Mat4,
        at: Vec3,
        reach: f32,
        damage_at: impl Fn(f32) -> f32,
    ) {
        let (Some(dm), Some(parts)) = (self.spec.damage.as_deref(), self.parts.as_mut()) else {
            return;
        };
        if !self.wrecked {
            parts.blast(dm, body.inverse().transform_point3(at), reach, damage_at);
        }
    }

    /// The `ModifierKey` of its own of the part a hit at `at` (world),
    /// its place `body`, strikes (a weak point's), if that part has one.
    pub fn struck_key(&self, body: Mat4, at: Vec3) -> Option<u32> {
        let (dm, parts) = (self.spec.damage.as_deref()?, self.parts.as_ref()?);
        let k = dm.part_at(
            body.inverse().transform_point3(at),
            m2_engine::vehicle_damage::HIT_SLACK,
        )?;
        parts.key(k)
    }

    /// Its body's state as its health stands: damaged (burning, the hood
    /// on fire) at its `TickDamage` health, destroyed once wrecked.
    pub fn follow_health(&mut self) {
        let burns = self
            .spec
            .tuning
            .burns_below
            .is_some_and(|b| self.damage > 0.0 && self.most_health() - self.damage <= b);
        let (Some(dm), Some(parts)) = (self.spec.damage.as_deref(), self.parts.as_mut()) else {
            return;
        };
        if burns || self.wrecked {
            parts.body_damaged(dm);
        }
        if self.wrecked {
            parts.body_destroyed(dm);
        }
    }

    /// Throws off the parts its state machine let go since last asked,
    /// from where it stands (`body`).
    pub fn throw_loose(&mut self, body: Mat4) {
        let (Some(dm), Some(parts)) = (self.spec.damage.as_deref(), self.parts.as_mut()) else {
            return;
        };
        let (lo, hi) = self.spec.bounds;
        let middle = body.transform_point3((lo + hi) * 0.5);
        let ground = body.transform_point3(Vec3::new(0.0, lo.y, 0.0)).y + 0.15;
        for d in parts.take_detached() {
            let centre = body.transform_point3(d.centre);
            let out = (centre - middle).with_y(0.0).normalize_or(Vec3::X);
            // A turn of its own, by which part it is.
            let k = d.part as f32;
            let axis = Vec3::new(k.sin(), 0.5, k.cos()).normalize();
            let mut hidden = vec![true; dm.nodes.len()];
            for (i, h) in hidden.iter_mut().enumerate() {
                let mut at = Some(i as u16);
                for _ in 0..=dm.nodes.len() {
                    match at {
                        Some(n) if n == d.node => {
                            *h = false;
                            break;
                        }
                        Some(n) => at = dm.nodes[usize::from(n)].parent,
                        None => break,
                    }
                }
            }
            if self.loose.len() >= LOOSE_MOST {
                self.loose.remove(0);
            }
            self.loose.push(LoosePart {
                from: body,
                centre,
                offset: Vec3::ZERO,
                velocity: out * LOOSE_OUT + Vec3::Y * LOOSE_UP,
                axis,
                angle: 0.0,
                spin: LOOSE_SPIN,
                ground,
                hidden,
                age: 0.0,
            });
        }
    }

    /// Its thrown-off parts fly `dt` on; those that have lain long enough
    /// are gone.
    pub fn step_loose(&mut self, dt: f32) {
        for l in &mut self.loose {
            l.step(dt);
        }
        self.loose.retain(|l| l.age < LOOSE_LASTS);
    }

    /// The shells left in its cannon, which carries as many as its
    /// template's `iClipSize` (the AMX-30's 40, which the HUD counts down
    /// from in the videos); `None` for a gun that doesn't run out.
    pub fn rounds(&self) -> Option<u32> {
        let gun = self.spec.tuning.gun.filter(|_| self.spec.cannon())?;
        Some(gun.rounds.saturating_sub(self.fired))
    }

    /// The machine gun a tank mounts beside its cannon (as
    /// [`m2_engine::weapons::machine_gun`] finds it: the AMX-30's "Mounted
    /// Heavy MG (Tank) (VZ)"), and the rounds left in it, of its template's
    /// `iClipSize` (300, no reserve), as the gun box counts them down;
    /// `None` without a cannon or such a gun.
    pub fn machine_gun(&self) -> Option<(&m2_engine::weapons::WeaponSpec, u32)> {
        if !self.spec.cannon() {
            return None;
        }
        match m2_engine::weapons::machine_gun(&self.spec.armed) {
            m2_engine::weapons::MachineGun::Mounted(w) if w.clip > 0 => {
                Some((w, w.clip.saturating_sub(self.mg_fired)))
            }
            _ => None,
        }
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

/// The catalogue's person models loaded up front: those of the roles
/// soldiers' names ask for, for soldiers whose template has no model of
/// its own (the rest stand in their template's).
const ARMY: [&str; 3] = ["vz_hum_soldier", "vz_hum_soldierelite", "vz_hum_deathsquad"];

/// A model the loading thread has decoded.
pub enum Loaded {
    Vehicle(VehicleSpec, CpuModel),
    /// A vehicle asked for by the mission scripts' object `guid`.
    MissionVehicle(u32, VehicleSpec, CpuModel),
    /// One of the game's own parked vehicles: its layer entity, where it
    /// stands (position, yaw), its spec and model.
    WorldVehicle(u32, Vec3, f32, Vec<(String, String)>, VehicleSpec, CpuModel),
    Catalogue(Catalogue),
    /// A person's model: a role's, or a template's own (true).
    Person(PersonModel, CpuModel, Skeleton, bool),
    /// A template's own person model that could not be read: model and
    /// template.
    NoPerson(u32, String),
    /// A hand weapon: name, model, muzzle and the node spent cases fly
    /// from (model space).
    Weapon(&'static str, CpuModel, Vec3, Option<Vec3>),
    /// Any other hand weapon's model the templates name: its name's hash,
    /// model, muzzle, case node.
    HandModel(u32, CpuModel, Vec3, Option<Vec3>),
    /// A model for the helicopters' deliveries (a helicopter's, a supply
    /// crate's): its hash, each node's place at rest, and its rotors.
    Flight(u32, CpuModel, Vec<Mat4>, Vec<m2_assets::vehicle::Rotor>),
    /// A wreck's chunk's model ([`Chunk`]): its hash and model.
    Chunk(u32, CpuModel),
}

/// Finds the island's vehicle and person models and decodes the soldiers'
/// role models ([`ARMY`]) and, when `park`, up to [`PARKED`] vehicles, on
/// a thread (the search reads the whole archive).
fn load_cast(install: Install, hand: Vec<u32>, park: bool) -> Receiver<Loaded> {
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
                let eject = combat::eject_node(&model);
                let (lo, hi) = (model.info.bounds.min, model.info.bounds.max);
                weapons.push(format!(
                    "{name} {hash:08x} ({:.2} x {:.2} x {:.2} m)",
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    hi[2] - lo[2]
                ));
                let mut cpu = CpuModel::build(hash, &model, 0);
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                if tx.send(Loaded::Weapon(name, cpu, muzzle, eject)).is_err() {
                    return Ok(());
                }
            }
            eprintln!("weapons: {}", weapons.join(", "));
            // Every other gun and explosive the templates name, where the
            // archive has its model.
            let mut more = 0;
            for hash in hand {
                if found.weapons.iter().any(|&(_, h)| h == hash) {
                    continue;
                }
                let Ok(data) = source.model(hash) else {
                    continue;
                };
                let Ok(model) = data.parse() else { continue };
                let muzzle = m2_assets::vehicle::marker(&model, combat::MUZZLE)
                    .map_or(Vec3::new(0.0, 0.0, model.info.bounds.max[2]), Vec3::from);
                let eject = combat::eject_node(&model);
                let cpu = CpuModel::build(hash, &model, 0);
                if tx
                    .send(Loaded::HandModel(hash, cpu, muzzle, eject))
                    .is_err()
                {
                    return Ok(());
                }
                more += 1;
            }
            eprintln!("hand weapons from the templates: {more} more models");
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
            if !park {
                eprintln!(
                    "vehicles: none of the catalogue's parked (a mission: only those the layers and scripts place)"
                );
                return Ok(());
            }
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
            ruins: Default::default(),
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
            park: true,
            people: People::default(),
            riders: Vec::new(),
            arsenal: Arsenal::default(),
            explosives: Vec::new(),
            props: Props::default(),
            flights: Flights::default(),
            strikes: crate::strikes::Strikes::default(),
            effects,
            particles: None,
            hide_fx: false,
            vehicle_loads: None,
            catalogue: None,
            install: None,
            requests: channel(),
            chunks: Vec::new(),
            chunk_models: HashMap::new(),
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
    /// above `spawn`, or else at the start point named `start_name`
    /// ([`MISSION_START`], [`FREE_ROAM_START`]).
    pub fn open_world(
        &mut self,
        backend: &mut dyn RenderBackend,
        install: &Install,
        settings: WorldSettings,
        spawn: Option<(f32, f32)>,
        start_name: &str,
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
        // Without a place asked for, the start point named (where the first
        // contract starts her, or where it leaves her), facing its way.
        let start = spawn
            .is_none()
            .then(|| world.index.start(start_name))
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
                eprintln!("start: no {start_name} in the layers; the first town instead");
                (DEFAULT_SPAWN, 0.0)
            }
        };
        let y = world.index.rough_height(x, z) + 40.0;
        self.focus = (Vec3::new(x, y, z), Vec3::new(x, y - 20.0, z + 60.0));
        self.props.physics_props = world.index.template_db.prop_physics.clone();
        self.people.mixer_profiles = world.index.template_db.mixer_profiles.clone();
        self.world = Some(world);

        // The hero, to walk the island as.
        self.open(install)?;
        if let (Some(source), Some(world)) = (self.source.as_mut(), self.world.as_ref()) {
            match crate::particles::ParticleDraw::load(
                backend,
                &mut self.materials,
                source,
                &world.index.template_db,
            ) {
                Ok(p) => self.particles = Some(p),
                Err(e) => eprintln!("particles: {e:#}; fire and smoke drawn as spheres"),
            }
        }
        self.player = Some(Player::new(x, z, facing, None));
        self.load_hero(backend, self.hero);
        let hand = self
            .world
            .as_ref()
            .map(|w| w.index.template_db.arms.hand_models())
            .unwrap_or_default();
        self.vehicle_loads = Some(load_cast(install.clone(), hand, self.park));
        self.install = Some(install.clone());
        Ok(())
    }

    /// The hero's own rifle: the gun their template's `EquipmentLink`s
    /// hand them.
    pub fn hero_rifle(&self) -> Option<crate::combat::Rifle> {
        let db = &self.world.as_ref()?.index.template_db;
        let (name, gun) = db.own_rifle(self.hero.template())?;
        Some(match db.arms.weapons.get(&gun.template) {
            Some(spec) => crate::combat::Rifle::from_spec(spec),
            None => crate::combat::Rifle::of(name, &gun),
        })
    }

    /// Every gun, thrown weapon and vehicle armament the templates give.
    pub fn arms(&self) -> Option<&m2_engine::weapons::Arms> {
        Some(&self.world.as_ref()?.index.template_db.arms)
    }

    /// The game's damage table, if found.
    pub fn damage_table(&self) -> Option<m2_engine::tuning::DamageTable> {
        self.world.as_ref()?.index.template_db.damage.clone()
    }

    /// The thrown weapons the hero's template's `EquipmentLink`s hand
    /// them (free roam), each with how many they carry.
    pub fn hero_explosives(&self) -> Vec<(String, u32)> {
        let Some(w) = &self.world else {
            return Vec::new();
        };
        let db = &w.index.template_db;
        db.ids(self.hero.template())
            .iter()
            .filter_map(|id| db.equipment.get(id))
            .flatten()
            .filter_map(|t| db.arms.throwables.get(t))
            .map(|t| (t.name.clone(), t.max))
            .collect()
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
        let mut model = match GpuModel::load(backend, source, &mut self.materials, hash, 0)
            .map_err(anyhow::Error::from)
            .and_then(|gpu| Ok((gpu, Rig::load(source, hash, Gait::Idle, Some(hero))?)))
        {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("player model: {e:#}");
                None
            }
        };
        if self.people_rig.is_none() {
            // Jennifer's own rig is loaded as no hero's is (every clip
            // that fits her): hers will do when she is the hero. Taken
            // before the hero's clips are picked, so nothing done to her
            // rig for her reaches the people's.
            self.people_rig = match &model {
                Some((_, rig)) if hero == Hero::Jennifer && hash == DEFAULT_MODEL => {
                    Some(rig.shared())
                }
                _ => match Rig::load(source, DEFAULT_MODEL, Gait::Idle, None) {
                    Ok(rig) => Some(rig.shared()),
                    Err(e) => {
                        eprintln!("people's rig: {e:#}; the hero's instead");
                        model.as_ref().map(|(_, rig)| rig.shared())
                    }
                },
            };
        }
        // The clips by name, then as the on-foot table gives them the
        // hero, then the hero's own group's for those it doesn't.
        let set = self
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.animation_set(hero.template()));
        let mut clips = model
            .as_ref()
            .map(|(_, rig)| npc::FightClips::find(rig))
            .unwrap_or_default();
        let from_table = match &mut model {
            Some((_, rig)) => table_clips(source, set, hero, rig, &mut clips),
            None => TableClips::default(),
        };
        let settled = &from_table.settled;
        let clips = match &model {
            Some((_, rig)) => {
                let clips = clips.own_group(rig, settled);
                log_hero_clips(hero, rig, &clips, &from_table);
                clips
            }
            None => clips,
        };
        if let Some((_, rig)) = &model {
            let h = |i: Option<usize>| {
                i.map_or("none".to_owned(), |i| {
                    let c = &rig.clips[i];
                    let own = if rig.own.contains(&c.hash) {
                        " own"
                    } else {
                        ""
                    };
                    let table = if settled.contains(&i) { " table" } else { "" };
                    format!("{:08x}{own}{table}", c.hash)
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
        // The weapons already loaded were held by the last body's
        // skeleton.
        if let Some((_, rig)) = &mut model {
            rig.set_aim_clip(clips.aim);
            self.arsenal.rehold(rig);
        }
        let moves = match &model {
            Some((_, rig)) => hero_moves(source, set, hero, rig, clips),
            None => held::Moves::default(),
        };
        if let Some(player) = &mut self.player {
            player.set_body(model, clips, moves);
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

    /// Loads a wreck's chunk's model `hash` on a thread.
    fn load_chunk_model(&self, hash: u32) {
        let Some(install) = self.install.clone() else {
            return;
        };
        let tx = self.requests.0.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let mut source = install.open_world()?;
                let data = source.model(hash)?;
                let mut cpu = CpuModel::from_data(&data, 0)?;
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
                let _ = tx.send(Loaded::Chunk(hash, cpu));
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("chunk model {hash:08x}: {e:#}");
            }
        });
    }

    /// Loads a template's own person model (`Carmona`'s, a `VZ Soldier`'s)
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
                let mut cpu = CpuModel::from_data(&data, 0)?;
                cpu.decode_textures(&mut source, MODEL_TEXTURE_WIDTH);
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
                let _ = tx.send(Loaded::NoPerson(model, template));
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
            Loaded::NoPerson(model, template) => {
                self.people.refuse(*model, template.clone());
                return Ok(());
            }
            Loaded::Vehicle(_, cpu)
            | Loaded::MissionVehicle(_, _, cpu)
            | Loaded::WorldVehicle(_, _, _, _, _, cpu)
            | Loaded::Person(_, cpu, _, _)
            | Loaded::Weapon(_, cpu, _, _)
            | Loaded::HandModel(_, cpu, _, _)
            | Loaded::Flight(_, cpu, _, _)
            | Loaded::Chunk(_, cpu) => cpu,
        };
        let uploaded = self
            .materials
            .load_model_textures(backend, source, cpu)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(GpuModel::upload(backend, &mut self.materials, cpu)?));
        let model = match uploaded {
            Ok(model) => model,
            Err(e) => {
                // A template's own person model that won't go up: its
                // spawns stand as their name's role, or not at all,
                // rather than wait for it.
                if let Loaded::Person(p, _, _, true) = &loaded {
                    eprintln!("person {} ({:08x}): {e:#}; left out", p.role, p.model);
                    self.people.refuse(p.model, p.role.clone());
                }
                return Err(e);
            }
        };
        // What its template says of its handling and its gun: its own
        // (the mission's `AMX30 (Full)`), else its model's.
        let tune = |spec: VehicleSpec| {
            let Some(db) = self.world.as_ref().map(|w| &w.index.template_db) else {
                return spec.tuned(None);
            };
            let t = db.tuning_for(&spec.label, spec.model);
            let arm = db.armament_for(&spec.label, spec.model);
            spec.tuned(t).armed_with(arm, &db.arms)
        };
        match loaded {
            Loaded::Catalogue(_) | Loaded::NoPerson(..) => {}
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
                mg_fired: 0,
                damage: 0.0,
                parts: None,
                loose: Vec::new(),
                debris_thrown: false,
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
                    wrecked: self.ruins.contains(&guid),
                    guid: Some(guid),
                    spot: None,
                    crew,
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                    fired: 0,
                    mg_fired: 0,
                    damage: 0.0,
                    parts: None,
                    loose: Vec::new(),
                    debris_thrown: self.ruins.contains(&guid),
                });
            }
            Loaded::WorldVehicle(entity, at, yaw, crew, spec, _) => {
                self.vehicles.push(ParkedVehicle {
                    spec: tune(spec),
                    model,
                    id: None,
                    wrecked: self.ruins.contains(&entity),
                    guid: None,
                    spot: Some((entity, at, yaw)),
                    crew,
                    turret: 0.0,
                    gun: 0.0,
                    rolled: 0.0,
                    fired: 0,
                    mg_fired: 0,
                    damage: 0.0,
                    parts: None,
                    loose: Vec::new(),
                    debris_thrown: self.ruins.contains(&entity),
                });
            }
            Loaded::Chunk(hash, _) => {
                let (lying, near) = (
                    self.chunks
                        .iter()
                        .filter(|c| c.velocity == Vec3::ZERO)
                        .count(),
                    self.chunks.iter().filter(|c| c.model == hash).count(),
                );
                eprintln!(
                    "chunk model {hash:08x} up: {near} of {} chunks its, {lying} lying",
                    self.chunks.len()
                );
                self.chunk_models.insert(hash, Some(model));
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
            Loaded::Weapon(name, _, muzzle, eject) => {
                let rig = self
                    .player
                    .as_ref()
                    .and_then(|pl| pl.model.as_ref())
                    .map(|(_, rig)| rig);
                self.arsenal.add(name, model, (muzzle, eject), rig);
            }
            Loaded::HandModel(hash, _, muzzle, eject) => {
                let rig = self
                    .player
                    .as_ref()
                    .and_then(|pl| pl.model.as_ref())
                    .map(|(_, rig)| rig);
                self.arsenal.add_model(hash, model, (muzzle, eject), rig);
            }
            Loaded::Person(p, _, skeleton, named) => {
                // The people's clips fit every human skeleton.
                let template = self
                    .people_rig
                    .as_ref()
                    .and_then(|rig| rig.for_skeleton(skeleton));
                // A template's own gun (`EquipmentLink`); a role's none.
                let db = self.world.as_ref().map(|w| &w.index.template_db);
                let gun = db
                    .filter(|_| named)
                    .and_then(|db| db.own_rifle(&p.role))
                    .map(|(name, gun)| (name.to_owned(), gun));
                match template {
                    Some(rig) => {
                        eprintln!(
                            "person {} ({:08x}): {} bones, {} clips, gun {}",
                            p.role,
                            p.model,
                            rig.skeleton.len(),
                            rig.clips.len(),
                            gun.as_ref()
                                .map_or("none of its own".to_owned(), |(name, g)| {
                                    format!(
                                        "{name} (model {:08x}, {:.0} a minute, {} a round)",
                                        g.model.unwrap_or(0),
                                        g.rate_of_fire,
                                        g.damage
                                    )
                                })
                        );
                        self.people.add_kind(PersonKind {
                            role: p.role,
                            named,
                            gun: gun.map(|(_, g)| g),
                            model,
                            rig,
                        });
                    }
                    None => {
                        eprintln!(
                            "person {}: the clips don't fit its skeleton; left out",
                            p.role
                        );
                        if named {
                            self.people.refuse(p.model, p.role);
                        }
                    }
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
        self.tick_vehicle_parts(dt);
    }

    /// Vehicles' parts: those just placed get their parts' health from
    /// their template; parts let go are thrown off; thrown-off parts fly.
    fn tick_vehicle_parts(&mut self, dt: f32) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let db = &world.index.template_db;
        let mut thrown = Vec::new();
        for (i, v) in self.vehicles.iter_mut().enumerate() {
            if v.parts.is_none()
                && let Some(dm) = v.spec.damage.as_deref()
            {
                let healths = db
                    .part_healths_for(&v.spec.label, v.spec.model)
                    .cloned()
                    .unwrap_or_default();
                let parts = PartDamage::new(dm, &healths);
                eprintln!(
                    "vehicle parts: {} {} parts, {} with health",
                    v.spec.label,
                    dm.states.machines.len(),
                    parts.with_health()
                );
                v.parts = Some(parts);
                // A wreck as it was loaded stands as its wreck, cold.
                if v.wrecked
                    && let Some(p) = v.parts.as_mut()
                {
                    p.start_destroyed(dm);
                }
            }
            if let Some(body) = v.id.and_then(|id| world.physics.vehicle_transform(id)) {
                v.follow_health();
                v.throw_loose(body);
                if v.wrecked && !v.debris_thrown {
                    v.debris_thrown = true;
                    let debris = db
                        .part_healths_for(&v.spec.label, v.spec.model)
                        .map(|h| h.debris.as_slice())
                        .unwrap_or_default();
                    let seed = v.spec.model ^ (i as u32 + 1).wrapping_mul(0x9e37_79b9);
                    let chunks = throw_chunks(debris, body, v.spec.bounds, seed);
                    eprintln!(
                        "wreck debris: {} throws {} chunks ({} records)",
                        v.spec.label,
                        chunks.len(),
                        debris.len()
                    );
                    thrown.extend(chunks);
                }
            }
            v.step_loose(dt);
        }
        let physics = &world.physics;
        let ground = |at: Vec3, reach: f32| physics.cast_ray(at, Vec3::NEG_Y, reach + 0.05);
        for c in &mut self.chunks {
            c.step(dt, |at, reach| ground(at, reach).map(|(d, _)| d));
        }
        self.chunks.retain(|c| c.age < c.lasts);
        self.chunks.extend(thrown);
        if self.chunks.len() > CHUNKS_MOST {
            let over = self.chunks.len() - CHUNKS_MOST;
            self.chunks.drain(..over);
        }
        let wanted: Vec<u32> = self
            .chunks
            .iter()
            .map(|c| c.model)
            .filter(|m| !self.chunk_models.contains_key(m))
            .collect();
        for m in wanted {
            if let std::collections::hash_map::Entry::Vacant(e) = self.chunk_models.entry(m) {
                e.insert(None);
                self.load_chunk_model(m);
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
                // A wreck shows its wrecked body, turret and wheels; else
                // its parts as they stand, once any has been hurt.
                v.model.wrecked = v.wrecked;
                v.model.parts_hidden = match (&v.parts, v.spec.damage.as_deref()) {
                    (Some(p), Some(dm)) if !p.untouched() => Some(p.hidden(dm)),
                    _ => None,
                };
                for l in &v.loose {
                    v.model.draw_hiding(queue, l.world(), &l.hidden);
                }
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
        for c in &self.chunks {
            if let Some(Some(m)) = self.chunk_models.get(&c.model) {
                m.draw(queue, c.place());
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
            && player.binoculars.is_none()
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
                (None, Weapon::Rifle) => self.arsenal.of_gun(&fight.rifle),
                (None, Weapon::Rpg) => self.arsenal.of_gun(&fight.launcher),
                (None, Weapon::GrenadeLauncher) => self.arsenal.of_gun(&fight.launcher),
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
            // What she carries but doesn't hold hangs on her back (not
            // seated: it would go through the seat's back, and the one
            // she holds isn't drawn there either).
            let holding = !fight.designator && player.throwing.is_none() && !player.down;
            let slung = if player.seated {
                [None; 2]
            } else {
                fight.slung(holding)
            };
            for (slot, slung) in slung.into_iter().enumerate() {
                let model = match slung {
                    Some(Weapon::Rifle) => self.arsenal.of_gun(&fight.rifle),
                    Some(Weapon::Rpg) => self.arsenal.of_gun(&fight.launcher),
                    Some(Weapon::GrenadeLauncher) => self.arsenal.of_gun(&fight.launcher),
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
            // Each in their own look, the dead too.
            let outfit = Some(&n.outfit[..]).filter(|o| !o.is_empty());
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
                model.draw_posed_as(backend, queue, Mat4::IDENTITY, &nodes, &skin, outfit)?;
                continue;
            }
            let (nodes, skin) = n.rig.posed();
            let body = n.model_matrix();
            model.draw_posed_as(backend, queue, body, &nodes, &skin, outfit)?;
            if n.soldier
                && n.alive
                && let Some(w) = self.arsenal.npc_gun(n.gun)
                && let Some(at) =
                    self.arsenal
                        .in_hand(w, body, &nodes, (n.hand, n.attach), n.rig.two_handed)
            {
                w.model.draw(queue, at);
            }
        }
        for r in &self.riders {
            let Some(k) = self.people.kinds.get(r.kind) else {
                continue;
            };
            if r.frame.transform_point3(Vec3::ZERO).distance(eye) > npc::DRAW_DISTANCE {
                continue;
            }
            let outfit = Some(&r.outfit[..]).filter(|o| !o.is_empty());
            let (nodes, skin) = r.rig.posed();
            k.model
                .draw_posed_as(backend, queue, r.frame, &nodes, &skin, outfit)?;
        }
        for (what, at, velocity) in fight.projectiles() {
            let model = match what {
                combat::Flying::Charge(m) => m.and_then(|m| self.arsenal.by_model.get(&m)),
                combat::Flying::Rocket => self.arsenal.rocket.as_ref(),
                combat::Flying::Grenade => self.arsenal.grenade.as_ref(),
                combat::Flying::Round => self
                    .arsenal
                    .round
                    .as_ref()
                    .or(self.arsenal.grenade.as_ref()),
            };
            if let Some(w) = model {
                // A rocket's nose (its muzzle marker) along its flight, and
                // a launched round's; a thrown grenade tumbles as it was.
                let turn = Quat::from_rotation_arc(
                    w.muzzle.normalize_or(Vec3::Z),
                    velocity.normalize_or(Vec3::Z),
                );
                let spin = if matches!(what, combat::Flying::Grenade | combat::Flying::Charge(_)) {
                    Quat::IDENTITY
                } else {
                    turn
                };
                w.model
                    .draw(queue, Mat4::from_rotation_translation(spin, at));
            }
        }
        for &(effect, left, life) in &fight.effects {
            // The game's own effects draw fire and smoke when they loaded.
            let standing_in = matches!(
                effect,
                combat::Effect::Fire { .. }
                    | combat::Effect::Smoke { .. }
                    | combat::Effect::Soot { .. }
            );
            if standing_in && (self.particles.is_some() || self.hide_fx) {
                continue;
            }
            self.effects.draw(queue, effect, 1.0 - left / life);
        }
        let hide = self.hide_fx;
        if let Some(p) = self.particles.as_mut().filter(|_| !hide) {
            p.draw(backend, queue, view)?;
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
    fn a_thrown_off_part_flies_out_and_lies_where_it_lands() {
        let mut l = LoosePart {
            from: Mat4::IDENTITY,
            centre: Vec3::new(1.0, 1.0, 0.0),
            offset: Vec3::ZERO,
            velocity: Vec3::new(LOOSE_OUT, LOOSE_UP, 0.0),
            axis: Vec3::Y,
            angle: 0.0,
            spin: LOOSE_SPIN,
            ground: 0.2,
            hidden: Vec::new(),
            age: 0.0,
        };
        // Up first, then down.
        l.step(0.1);
        assert!(l.offset.y > 0.0 && l.offset.x > 0.0);
        for _ in 0..100 {
            l.step(0.1);
        }
        // At rest with its middle on the ground, out from where it was.
        assert_eq!(l.velocity, Vec3::ZERO);
        assert!((l.centre.y + l.offset.y - 0.2).abs() < 1e-5);
        let resting = l.offset;
        l.step(0.1);
        assert_eq!(l.offset, resting);
        // Its pieces go with it: its middle is where it flew to.
        let at = l.world().transform_point3(l.centre);
        assert!(at.distance(l.centre + l.offset) < 1e-4);
    }

    #[test]
    fn a_heros_aim_clip_finds_the_hands_again_and_stays_the_heros() {
        let mut rig = crate::npc::tests::rig();
        let _ = rig.hands.set(Some((1, 2)));
        rig.set_aim_clip(Some(0));
        assert_eq!(rig.aim_clip, Some(0));
        assert!(rig.hands.get().is_none(), "the old aim's hands are kept");
        // The people's rigs aim with the clip by name, not the hero's.
        let _ = rig.hands.set(Some((1, 2)));
        let people = rig.shared();
        assert_eq!(people.aim_clip, None);
        assert!(people.hands.get().is_none());
    }

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

    /// The Valiant's `Chunk_ae` record (2 to 4, 15 to 30 m/s, 5 to 20 s)
    /// and its fuel can, which has no model.
    fn valiant_debris() -> Vec<m2_engine::vehicle_damage::Debris> {
        use m2_engine::vehicle_damage::Debris;
        vec![
            Debris {
                template: 0x8000_6396,
                model: Some(0x5d2e_359b),
                hardpoint: None,
                count: (2, 4),
                speed: (15.0, 30.0),
                lasts: (5.0, 20.0),
            },
            Debris {
                template: 0x8000_8608,
                model: None,
                hardpoint: None,
                count: (1, 1),
                speed: (0.1, 0.15),
                lasts: (0.0, 0.2),
            },
        ]
    }

    #[test]
    fn a_wreck_throws_its_records_chunks_within_their_ranges() {
        let bounds = (Vec3::new(-1.0, 0.0, -2.5), Vec3::new(1.0, 1.5, 2.5));
        let body = Mat4::from_translation(Vec3::new(10.0, 2.0, 0.0));
        let mut counts = HashSet::new();
        for seed in 0..64 {
            let chunks = throw_chunks(&valiant_debris(), body, bounds, seed);
            assert!((2..=4).contains(&chunks.len()), "{}", chunks.len());
            counts.insert(chunks.len());
            for c in &chunks {
                assert_eq!(c.model, 0x5d2e_359b);
                assert_eq!(c.at, Vec3::new(10.0, 2.75, 0.0));
                let speed = c.velocity.length();
                assert!((15.0..=30.0).contains(&speed), "{speed}");
                assert!(c.velocity.y > 0.0);
                assert!((5.0..=20.0).contains(&c.lasts));
            }
        }
        // Every count the record allows comes up.
        assert_eq!(counts.len(), 3);
        // The same seed throws the same.
        assert_eq!(
            throw_chunks(&valiant_debris(), body, bounds, 7),
            throw_chunks(&valiant_debris(), body, bounds, 7)
        );
    }

    #[test]
    fn a_chunk_falls_to_the_ground_and_lies_there() {
        let mut c = Chunk {
            model: 1,
            at: Vec3::new(0.0, 5.0, 0.0),
            velocity: Vec3::new(4.0, 6.0, 0.0),
            axis: Vec3::Y,
            angle: 0.0,
            spin: 5.0,
            age: 0.0,
            lasts: 10.0,
        };
        // Flat ground at y 0.
        let ground = |at: Vec3, reach: f32| (at.y <= reach).then_some(at.y);
        for _ in 0..300 {
            c.step(1.0 / 60.0, ground);
        }
        assert_eq!(c.velocity, Vec3::ZERO);
        assert!(c.at.y.abs() < 1e-4, "{}", c.at.y);
        assert!(c.at.x > 4.0);
        let lying = c;
        c.step(1.0, ground);
        assert_eq!(c.at, lying.at);
        assert!((c.age - 6.0).abs() < 1e-3);
    }
}

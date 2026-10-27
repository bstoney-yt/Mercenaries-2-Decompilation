//! The mission played by itself (`--autoplay`, for testing): she heads for
//! the markers the scripts put up, shoots the soldiers and blows up the
//! things they mark, gets into the vehicles they bring and drives on, all
//! with a player's moves: an [`Intent`] a tick, as the keyboard and mouse
//! give. She goes the way planned round walls, the sea and ground too
//! steep ([`Route`]). What she does is logged, so a run shows whether the
//! mission can be played through and where she got stuck. Where she makes
//! no headway for a while she hops ahead (logged too): the run goes on to
//! the later objectives, and the hops show where the way is blocked.

use m2_engine::physics::{FLOAT_DEPTH, StaticKey};
use m2_engine::road_graph::RoadGraph;
use m2_engine::{Physics, Templates, VehicleControls, VehicleId, VehicleSpec};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3};

use crate::combat::{Combat, Trigger, Weapon};
use crate::game::{Game, can_get_out, get_out, place};
use crate::mission::{Mark, WATERLINE};
use crate::player::{Intent, Player};
use crate::route::{Mover, Route};
use crate::support::{Aim, Entry, PDA_CLOSE, PDA_OPEN, Phase};

/// How fast she turns the camera, radians a second (a quick hand on the
/// mouse).
const TURN: f32 = 5.0;
/// The camera's pitch while she runs or drives, radians.
const CRUISE_PITCH: f32 = -0.2;
/// With nothing to do, she looks at what the scripts wait for her to
/// see if it is within this far (metres), from at least this far off
/// (invented: the camera rides behind her).
const LOOK_NEAR: f32 = 60.0;
const LOOK_FROM: f32 = 6.0;
/// Close enough to a place to be there, metres. Waiting there longer than
/// this (seconds), she walks round it this far out: the scripts' region
/// may lie to a side of their marker.
const ARRIVE: f32 = 2.5;
/// An action objective's mark (`MrxTaskObjectiveAction`): she goes this
/// close (metres, flat; invented, inside the 2 m reach the scripts put its
/// prompt on with), and takes its prompt again after this long (seconds)
/// while it is still offered.
const ACTION_KIND: &str = "action";
const ACTION_ARRIVE: f32 = 1.0;
const ACTION_AGAIN: f32 = 5.0;
/// Driving, she gets out within this of an action's object (metres, flat;
/// invented).
const ACTION_GET_OUT: f32 = 15.0;
/// Further above or below its object than this (metres; the alpha's
/// `CONTEXT_RISE`, a floor's height) an action's prompt isn't offered.
const ACTION_RISE: f32 = 2.5;
/// Riding in the scripts' helicopter, she gets out once it has stood still
/// this long (seconds; invented, longer than the transit's fade) and her
/// next place is elsewhere.
const RIDE_STILL: f32 = 20.0;
/// Still marked this long after she got in (seconds; invented), she gets
/// out and in again.
const RIDE_AGAIN: f32 = 10.0;
const CIRCLE_AFTER: f32 = 4.0;
const CIRCLE: f32 = 6.0;
/// At a place this long (seconds) with nothing happening, she takes one
/// of the scripts' vehicles standing within this far of it (metres): a
/// vehicle they brought her.
const TAKE_AFTER: f32 = 8.0;
const TAKE_NEAR: f32 = 40.0;
/// She shoots soldiers from within this far (metres), and blows things up
/// from between these.
const SHOOT_RANGE: f32 = 35.0;
/// Shooting a target with her rifle she fires this long (seconds) of
/// every so long (invented).
const TAP_FOR: f32 = 0.12;
const TAP_EVERY: f32 = 0.5;
/// She fires on a target with her rifle once her crosshair is within this
/// much of its middle, metres (invented: half a statue's width).
const SNIPE_WIDTH: f32 = 0.4;
/// Of spots to see a target from she tries this many, then passes it over
/// for now (invented). (Going down off PmcCon031's MG platform to the
/// columns below its rail, she was caught in its yard 180 s, round 31 of
/// its thread.)
const PERCH_TRIES: usize = 4;
/// A target an emplaced gun's round that blows up flies at is left this
/// long past its flight (seconds) before another at it (invented).
const LANDS: f32 = 0.5;
/// Nothing may stand this near an emplaced gun's muzzle (metres) on its
/// way to a target, for a round that blows up (invented: past its blast).
const MUZZLE_CLEAR: f32 = 8.0;
/// From an emplaced gun she holds her aim within this of the target's
/// middle, metres across (invented: a car's width).
const MANNED_WIDTH: f32 = 1.0;
/// An emplaced gun covers a target its turn and tilt bring within this
/// (a cosine: about 1 degree).
const COVERS: f32 = 0.9998;
/// A gun fires this far under a target's top (metres), when it can't tilt
/// to its middle (invented).
const TOP_IN: f32 = 0.5;
/// A target still standing after this long of her rifle (seconds, in
/// reach) is blown up instead (invented).
const RIFLE_TRY: f32 = 20.0;
/// ...or this long, of a kind her rounds have broken before (invented:
/// PmcCon031's columns break to about a second of them).
const RIFLE_RETRY: f32 = 5.0;
/// Targets that rounds break she shoots from within this far (metres;
/// invented, the ranges' furthest statues from their towers: PmcCon034's
/// Easy9, 151 m from where she shoots on the HQ's roof).
const SNIPE_RANGE: f32 = 170.0;
/// Shooting a target with her rifle she fires this long (seconds) of
/// every so long (invented). TAP_FOR: f32 = 0.12; TAP_EVERY: f32 = 0.5;
const BLAST_RANGE: (f32, f32) = (15.0, 45.0);
/// With nothing to blow things up with, she mans an emplaced gun this
/// near her (metres) that is this near the target (invented).
const MAN_SEEK: f32 = 80.0;
/// Whether target `g` is a prop that rounds break (its template's damage
/// row takes a `Bullet`, with health to wear: the ranges' targets): shot
/// with her rifle rather than rockets or C4 (PmcCon033 round 11: C4 on
/// each pistol-range target, 116 s against its 90 s to beat).
fn takes_rounds(game: &Game, g: u32) -> bool {
    let (Some((p, _)), Some(w)) = (game.scene.props.get(g), game.scene.world.as_ref()) else {
        return false;
    };
    let db = &w.index.template_db;
    !game.scene.props.sturdy.contains(&g)
        && db
            .durability
            .get(&p.template)
            .is_some_and(|d| !d.invincible && d.health > 0.0)
        && crate::breakables::Makeup::of(db, p.template).bullet >= ROUNDS_TAKEN
}

/// A target takes at least this much of a `Bullet`'s damage for her to
/// shoot it rather than blow it up (invented: PmcCon031's columns take a
/// little of rounds and stood through 240 s of her rifle, round 12; the
/// pistol range's targets take all of them).
const ROUNDS_TAKEN: f32 = 0.5;

/// She keeps to the target she is blowing up unless another is this much
/// nearer, metres (invented).
const KEEP_TARGET_BY: f32 = 10.0;
const MANNED_REACH: f32 = 90.0;
/// An emplaced gun this near her (metres) is manned whatever she carries
/// (invented).
const GUN_BY: f32 = 8.0;
/// ...if its rounds blow up and the target is this near it, metres
/// (invented: PmcCon031's flung cars stand up to 150 m from its recoilless
/// rifle, and she ran after them on foot, round 26 of its thread).
const BLAST_GUN_REACH: f32 = 170.0;
/// With the grenade launcher, whose rounds reach about 40 m.
const LOB_RANGE: (f32, f32) = (12.0, 32.0);
/// What only a support strike brings down (the first contract's gates)
/// she calls the bomb on from between these (metres): out of its frag's
/// reach, and near enough that the satellite's view (5 m/s) is soon over
/// it; she confirms the view within this of the target (metres).
const CALL_FROM: (f32, f32) = (crate::combat::BOMB_FRAG.0 + 8.0, 90.0);
const ON_SPOT: f32 = 0.5;
/// A tank's cannon reaches this far, metres.
const CANNON_RANGE: f32 = 90.0;
/// She fires once aimed this long (seconds) and this close (radians); a
/// turret must be this close to the camera's heading too.
const STEADY: f32 = 0.35;
const ON_TARGET: f32 = 0.03;
const TURRET_ON: f32 = 0.05;
/// Soldiers fighting her within this far are dealt with first, metres.
const DEFEND: f32 = 30.0;
/// With a countdown of the scripts' running she fights back only within
/// this (metres), and hurries this long after its last tick (seconds; the
/// countdown cues a tick each minute, from the logs). Invented.
const DEFEND_HURRIED: f32 = 12.0;
/// Places to deliver to that she leaves until the others are done.
/// OilCon002's posts: its 15 minutes start at the second, and post C, the
/// nearest the kidnappers' vans, last beat them in every round (6 of 6);
/// post B last lost them (round 5 here, 657 s on its leg). Invented: the
/// order is hers to choose.
const SAVE_FOR_LAST: &[&str] = &["oilcon002_loc_postC"];
const HURRY_FOR: f32 = 65.0;
/// Aimed this long (seconds) with something in the way, she goes where she
/// can see her target from: on rings this far out (metres), no higher or
/// lower than she stands by more than this if she can (metres).
const BLOCKED: f32 = 4.0;
const VANTAGE_RINGS: [f32; 5] = [6.0, 10.0, 15.0, 20.0, 26.0];
/// A soldier she fought back with no clear shot and nowhere better to see
/// them from is left be this long, seconds, as are those within this of
/// where they stood, metres (round 3 here: 600 s aiming at one 21 m down a
/// cliff from her). Invented.
const LEAVE_UNSEEN: f32 = 60.0;
const UNSEEN_NEAR: f32 = 3.0;
/// Blowing something up, in range this long (seconds) without a shot
/// (something in the way, or her aim never settling), she goes where she
/// can fire from: on rings this far out (metres), inside the rockets' and
/// the grenades' ranges. Invented.
const BLAST_BLOCKED: f32 = 8.0;
const BLAST_RINGS: [f32; 4] = [20.0, 26.0, 32.0, 38.0];
const LOB_RINGS: [f32; 3] = [16.0, 22.0, 28.0];
/// A shot meeting something within this of what she is blowing up meets
/// the thing itself (metres; a truck's half length and more). Invented.
const BLAST_BODY: f32 = 6.0;
/// Out of rockets, she sets C4 on what she is blowing up: thrown from
/// within this (metres), set off from at least this far (metres) or once
/// she has been going this long (seconds). Invented.
const PLANT_FROM: f32 = 4.0;
const PLANT_SAFE: f32 = 18.0;
const PLANT_AWAY: f32 = 12.0;
/// A prompt at her goal is taken again after this long (seconds) if it is
/// still offered. Invented.
const PRESS_AGAIN: f32 = 3.0;
/// A spot to fire from more than this above or below her once she is
/// over it is one she can't get to (metres). Invented.
const SPOT_RISE: f32 = 2.5;
/// Rings (metres) round her looked on for a way out of the water.
/// Invented.
const LANDING_RINGS: [f32; 6] = [6.0, 12.0, 24.0, 48.0, 96.0, 192.0];
/// Ground this far (metres) over the water is out of it. Invented.
const LANDING_DRY: f32 = 0.3;
/// How far (metres) short of a way out, towards her, the water must be
/// shallow enough to stand in. Invented.
const LANDING_WADE: f32 = 2.0;
/// Ways out she doesn't get out at before she is hopped out. Invented.
const STRANDED_HOP: u32 = 2;
/// A thing to blow up lower than this (metres) over the waterline is on
/// the water. Invented.
const AFLOAT: f32 = 3.0;
/// Ground this far (metres) below her feet a step ahead is a drop she
/// doesn't step closer over. Invented.
const LEDGE_DROP: f32 = 3.0;
/// Seconds with no headway towards a spot to fire from before she tries
/// another (short of the hop's). Invented.
const SPOT_GIVE_UP: f32 = 20.0;
/// Searches finding nowhere to fire from before she goes for another of
/// the things to blow up, and for how long (seconds). Invented.
const PASS_AFTER: u32 = 3;
const PASS_FOR: f32 = 180.0;
/// A vehicle's cannon in range of her goal this long (seconds) without
/// bringing it down, she gets out. Invented.
const CANNON_GIVE_UP: f32 = 30.0;
/// Sooner (seconds) when the gun is at its stop with the goal beyond it.
/// Invented.
const CANNON_BEYOND: f32 = 4.0;
const VANTAGE_CLIMB: f32 = 3.0;
/// Driving: she eases off within this far of a place (metres) above this
/// speed (m/s); stuck this long (seconds), she backs out for this long.
/// How far ahead a driver looks for a clear way: this much, plus this
/// many seconds at its speed, at most this (metres); how far either side
/// of the heading it looks (radians); and how flat a surface must be
/// (its normal's height) not to be a wall.
const WAY_AHEAD: f32 = 6.0;
const WAY_TIME: f32 = 1.5;
const WAY_MOST: f32 = 25.0;
const WAY_FAN: f32 = 1.4;
const WALL: f32 = 0.8;
const SLOW_NEAR: f32 = 30.0;
const SLOW: f32 = 7.0;
const STALL: f32 = 2.5;
const REVERSE: f32 = 1.5;
/// No headway (a metre closer) for this long, seconds: she jumps and
/// sidesteps; for this long, she hops this far ahead (metres).
const STUCK: f32 = 3.0;
const SIDESTEP: f32 = 1.2;
const HOP_AFTER: f32 = 25.0;
const HOP_AFTER_BOAT: f32 = 8.0;
/// On foot this near the place she is after (metres), she hops as soon as
/// a boat does (invented).
const HOP_NEAR: f32 = 40.0;
/// A place more than this over the sea is ashore, for a boat, which goes
/// for the water this far short of it, metres (invented).
const ASHORE: f32 = 1.5;
const SHORT_OF_SHORE: f32 = 6.0;
const HOP: f32 = 25.0;
/// Coming closer counts towards the hop only once she has also gone this
/// far (metres) from where she last did: the way's length wobbles by a
/// few metres as it is planned again while she stands still (round 98's
/// 1 to 6 m put her hop off by up to a minute twice).
const HEADWAY_MOVE: f32 = 4.0;
/// A hop along her way goes at least this far, metres; a way ending
/// nearer than that is passed over for the place itself.
const HOP_LEAST: f32 = 8.0;
/// On foot this long (seconds) without coming [`WANDER_NEARER`] metres
/// nearer where she walks to, as the crow flies, she hops on, however much
/// she walks meanwhile; a place that moves less than [`WANDER_SAME`] is
/// the same place (invented).
const WANDER: f32 = 20.0;
const WANDER_NEARER: f32 = 3.0;
const WANDER_SAME: f32 = 5.0;
/// A hop lands on the ground found from this far above the way's height
/// there, down to this far below it, metres; else on the highest there.
const HOP_ABOVE: f32 = 3.0;
const HOP_BELOW: f32 = 10.0;
/// A ride she has had is taken again for places further than this when it
/// stands within the second distance, metres.
const RIDE_FOR: f32 = 120.0;
const RIDE_NEAR: f32 = 40.0;
/// One of the scripts' vehicles she hasn't driven is taken from this far,
/// metres: they brought it for her.
const RIDE_FRESH: f32 = 150.0;
/// A vehicle coming the other way within this far ahead (metres) and this
/// far either side of her line (metres) has her swerve this far to the
/// right of her way (radians) and ease off (invented).
const ONCOMING_AHEAD: f32 = 35.0;
/// On foot she fights no nearer than this to a vehicle with goods riding
/// on it, metres (invented: past a soldier's grenade thrown at her
/// landing a few metres off).
const CARGO_CLEAR: f32 = 15.0;
/// What a vehicle stuck on is kept out of with this much room round it,
/// metres; not what is wider than this (a building's whole batch).
const AVOID_ROOM: f32 = 2.0;
const AVOID_LARGEST: f32 = 40.0;
/// Held by the ground alone, she keeps out of this square (half its side,
/// metres) this far ahead of her vehicle's middle (invented).
const TERRAIN_HALF: f32 = 3.0;
const TERRAIN_AHEAD: f32 = 4.0;
/// Vehicles going slower than this, m/s, within this of her, metres, are
/// kept out of on her ways (invented).
const PARKED_SPEED: f32 = 1.0;
const PARKED_NEAR: f32 = 400.0;
/// With goods on board she gives way to a vehicle coming within this of
/// hers, metres, within this many seconds (invented).
const CROSS_CLOSE: f32 = 5.0;
const CROSS_SECONDS: f32 = 3.0;
/// Soldiers fighting her within this of a truck with goods on it, itself
/// within [`CARGO_NEAR`] of her on foot, are dealt with before she drives
/// it, metres (invented).
const CARGO_GUARD: f32 = 60.0;
const CARGO_NEAR: f32 = 100.0;
/// She clears them for at most this long, seconds (invented).
const GUARD_MOST: f32 = 90.0;
const ONCOMING_SIDE: f32 = 3.0;
const ONCOMING_SECONDS: f32 = 3.0;
/// In a boat, as far as she goes in this long, and what goes slower than
/// this (m/s) counts as standing (invented).
const BOAT_AHEAD_SECONDS: f32 = 5.0;
const CRAWLING: f32 = 3.0;
/// She steers round a vehicle standing within this ahead, metres
/// (invented).
const AHEAD_STANDING: f32 = 20.0;
const SWERVE: f32 = 0.35;
/// Waiting this long (seconds) in a vehicle she drove to a place to
/// deliver, she gets out of it.
const EXIT_AFTER: f32 = 4.0;
/// With goods riding on her vehicle she drives no faster than this, m/s
/// (invented: about 40 km/h).
const LADEN_SPEED: f32 = 11.0;
/// A vehicle of no more health than this is driven as if laden. Invented.
const FRAIL: f32 = 60.0;
/// And no faster than this in one, m/s: round 21 ran the 50-health
/// Devastator into a UP truck at 11 m/s (62 damage). Invented.
const FRAIL_SPEED: f32 = 6.0;
/// She takes an action the scripts offer within this of its object,
/// metres (a context action's reach is a few metres; within 1.5 m she
/// stalled against the body, GurCon002 round 14).
const USE_FROM: f32 = 2.5;
/// Driving, she gets out this near an outpost's flag (metres), to walk
/// the rest.
const FLAG_ON_FOOT: f32 = 15.0;
/// Out of a vehicle without a cannon this far from a target, metres.
const GET_OUT: f32 = 40.0;
/// Within this far of a vehicle's door she presses to get in, metres;
/// presses are this far apart, seconds (a press gets her in or out).
const DOOR: f32 = 2.5;
/// Within this of a crewed vehicle's middle she starts its hijack,
/// metres (invented; m2-alpha's `HIJACK_REACH` is 6).
const HIJACK_NEAR: f32 = 5.0;
const PRESS_GAP: f32 = 1.5;
/// Fallen this far under the ground (metres: a hole where the island
/// hadn't streamed in yet), she is put back on it.
const UNDER: f32 = 5.0;
/// From further than this from a place (metres), she goes the way planned
/// to it ([`Route`]): in cells this size on foot and driving (metres),
/// over ground up to this steep on foot, on wheels and on tracks; planned
/// again every this long (seconds: the island streams in as she goes),
/// when the place moves this far or she strays this far off it (metres).
/// She heads for a point this far on along it, plus this many seconds at
/// her speed, at most this (metres); taken to run at this speed (m/s).
const PLAN_FROM: f32 = 25.0;
const FOOT_CELL: f32 = 2.0;
const DRIVE_CELL: f32 = 4.0;
const ON_FOOT: f32 = 40f32.to_radians();
const ON_WHEELS: f32 = 22f32.to_radians();
const ON_TRACKS: f32 = 27f32.to_radians();
const REPLAN: f32 = 20.0;
/// And when a vehicle comes to stand within this of her, metres, so long
/// as this long has gone by, seconds (invented).
const STANDING_FRESH: f32 = 150.0;
const REPLAN_SOONEST: f32 = 2.0;
const REPLAN_MOVED: f32 = 5.0;
/// A place farther than this, flat, is gone to by road, metres: she heads
/// for a road point on the way ([`m2_engine::road_graph`]) this far ahead
/// of her, getting on and off within the reach; the way is found again
/// once the place moves or she strays this far (metres). PmcCon004's base
/// is 4.6 km from the contract's start, over hills the ground's way never
/// crossed (its ground way ended 1.7-2.2 km short). Invented numbers.
const ROAD_FROM: f32 = 800.0;
const ROAD_REACH: f32 = 400.0;
const ROAD_AHEAD: f32 = 120.0;
const ROAD_REPLAN: f32 = 250.0;
const STRAYED: f32 = 15.0;
const LOOK_AHEAD: f32 = 5.0;
const LOOK_TIME: f32 = 0.6;
const LOOK_MOST: f32 = 20.0;
const RUN: f32 = 6.0;
/// A boat's way keeps this much water under its keel, metres. Stuck, a
/// boat is put afloat over water that deep: on its way, back towards it
/// in steps this long, or this far to either side of it (metres).
const UNDER_KEEL: f32 = 0.6;
const AFLOAT_STEP: f32 = 5.0;
const AFLOAT_SIDES: [f32; 5] = [0.0, -8.0, 8.0, -16.0, 16.0];
/// Rays looking for open water start this far over the sea, metres (over
/// rocks standing out of it).
const OVER_WATER: f32 = 30.0;
/// A way planned this much longer than the straight one is logged, at
/// most every this long (seconds).
const ROUNDABOUT: f32 = 1.15;
const LOG_WAY: f32 = 30.0;
/// A supply crate down within this far (metres) is opened before going on,
/// standing this far from its side (with a bash's 1.8 m reach), turned to
/// it within this much (radians); one not open after this many seconds is
/// left. The crates are the deliveries' and land where the scripts call
/// her: in the real game's first contract she smashes the weapon crate
/// (1:18) before the beach guards (1:26); here the guards can draw her
/// 100 m up the beach before it lands (from the recordings; she still
/// fights back first when they're within [`DEFEND`]).
const OPEN_NEAR: f32 = 250.0;
/// With the launcher empty, rockets lying within this far (metres) are
/// fetched, those the layers placed too. Invented.
const RESTOCK_NEAR: f32 = 150.0;
/// With things still to blow up, other pickups are fetched only within
/// this (metres); one she stands over this long (seconds) that doesn't
/// come with the key is left. Invented.
const BUSY_NEAR: f32 = 30.0;
const TAKE_AT: f32 = 10.0;
/// A vehicle she goes for and hasn't got into in this long (seconds) is
/// left. Invented.
const BOARD_FOR: f32 = 90.0;
const OPEN_FROM: f32 = 0.9;
const OPEN_FACING: f32 = 0.2;
const OPEN_FOR: f32 = 45.0;
/// A vehicle being brought to a drop point this near (metres) is waited
/// for this far from it (metres).
const AWAIT_REACH: f32 = 300.0;
const AWAIT_FROM: f32 = 15.0;
/// She looks at her PDA at least this long before shutting it, at most
/// this long (seconds).
const PDA_LOOK: f32 = 2.0;
const PDA_MOST: f32 = 15.0;
/// She opens it this long after the scripts begin waiting for that
/// (seconds): they put its prompt up two seconds after taking the last
/// line down (round 96), and she reads it first.
const PDA_PROMPT: f32 = 3.0;
/// Seconds between lines saying how she is getting on.
const REPORT: f32 = 15.0;
/// Under the place she is after but this far below it (metres: a prisoner
/// on a roof, ChiCon001's party official 56 m up), for this long
/// (seconds), she hops up there, logged as a hop: a player would land a
/// helicopter or grapple up.
const UNDER_BY: f32 = 4.0;
/// A building to destroy this near (metres, flat) under one she is to
/// reach is left standing meanwhile.
const HOLDS_UP: f32 = 20.0;
/// ...when they stand this much higher than its foot, metres (invented:
/// PmcCon033 round 5, she stood 0.1 m above the range's targets on the
/// same floor and left them standing as if on their roof).
const HOLDS_UP_RISE: f32 = 2.0;
/// She stands on a building to destroy when her feet are within this of
/// its box, metres (invented).
const STANDS_ON: f32 = 1.0;
/// One following her left this far behind (metres) this long (seconds)
/// is brought to her: a test driver's shortcut, logged, for ways the
/// alpha has no means of (a VIP off a roof, which a player lifts off by
/// helicopter).
const LEFT_BEHIND: f32 = 30.0;
const LEFT_FOR: f32 = 20.0;
/// And so is one this far off once she has waited at a place this long
/// (metres, seconds): OilCon002's hostage, wanted within 15 m of the
/// rescue site, stood 25 m back past a wall she hopped (round 8 here,
/// 2300 s). Invented.
const LEFT_AT_PLACE: f32 = 8.0;
const WAITED_FOR: f32 = 10.0;
const UNDER_FOR: f32 = 6.0;
/// An outpost to capture (`HUD_objective_outpost` on its capture point):
/// she goes within this far (metres) and calls in the soldier delivery
/// she holds (the outpost gives her one of its attackers' free,
/// `Outpost.GetFreebieName`), at most once in this long (seconds), and
/// stands by while their soldiers take the point.
const OUTPOST_NEAR: f32 = 20.0;
const DELIVERY_AGAIN: f32 = 60.0;
/// She throws a delivery's designator once facing where it is to land
/// within this, radians. Invented.
const FACE_THROW: f32 = 0.15;
/// She calls in a delivery for a place this near it (its designator
/// thrown from here lands well inside a 30 m drop zone), and keeps at the
/// call while this near (round 5 walked 20 m off and dropped it), metres:
/// facing it, a smoke thrown from 12 m landed 1.7 m off, and post C's spot
/// lies in a crate she stops 23 m short of (round 16). Invented.
const DELIVER_NEAR: f32 = 25.0;
const DELIVER_KEEP: f32 = 40.0;
/// After calling a delivery in, she waits this long for the scripts to
/// hand her its designator (round 7: the call went out, and she walked off
/// the frame after, the smoke not yet in her hand), seconds. Invented.
const DESIGNATOR_WAIT: f32 = 3.0;
/// Driving, she drives up to within this of a scripts' vehicle she is to
/// get into before getting out for it, metres. Invented.
const BOARD_DRIVE: f32 = 25.0;

/// What she is after.
#[derive(Clone, Debug, PartialEq)]
enum Goal {
    /// Get to the scripts' object `target`, at `at`.
    Reach { target: u32, at: Vec3 },
    /// Shoot person `npcs[i]`.
    Shoot(usize),
    /// Blow up the scripts' object `target`, aiming at `at`.
    Blast { target: u32, at: Vec3 },
    /// Get into vehicle `vehicles[i]`.
    Board(usize),
    /// Break open the supply crate `target` (the scripts' object) with a
    /// bash.
    Open(u32),
    /// Take one thing a supply crate left (walking up to it), by its
    /// [`Pickup::key`](crate::flights::Pickup::key).
    Take((u32, u32)),
    /// Call in the scripts' support item `key` she holds (a soldier
    /// delivery, by an outpost): the support menu, then its designator.
    Deliver(String),
    /// Get out of the vehicle she is driving, by the place she is after
    /// (an outpost's flag).
    GetOut,
    /// Take the action the scripts offer on their object `target` (a
    /// verified HVT's body), standing at `at`.
    Use { target: u32, at: Vec3 },
}

impl Goal {
    /// The same goal, though its target may have moved.
    fn same(&self, other: &Goal) -> bool {
        match (self, other) {
            (Goal::Reach { target: a, .. }, Goal::Reach { target: b, .. })
            | (Goal::Blast { target: a, .. }, Goal::Blast { target: b, .. }) => a == b,
            (Goal::Shoot(a), Goal::Shoot(b)) | (Goal::Board(a), Goal::Board(b)) => a == b,
            (Goal::Open(a), Goal::Open(b)) => a == b,
            (Goal::Take(a), Goal::Take(b)) => a == b,
            (Goal::Deliver(a), Goal::Deliver(b)) => a == b,
            (Goal::GetOut, Goal::GetOut) => true,
            (Goal::Use { target: a, .. }, Goal::Use { target: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// The way she is going ([`Route`]): when it was planned, and whether
/// for driving.
#[derive(Debug)]
struct Way {
    route: Route,
    planned: f32,
    driving: bool,
    /// The vehicles standing near her when it was planned (indices).
    standing: Vec<usize>,
}

/// A region she must not leave (the scripts end the contract when she
/// does): its outline, placed at `centre` (x, z) turned by `yaw`.
#[derive(Debug, Clone, PartialEq)]
pub struct Keep {
    pub region: m2_assets::region::Region,
    pub centre: [f32; 2],
    pub yaw: f32,
}

impl Keep {
    /// Whether `p` is within it (it has no height).
    pub fn holds(&self, p: Vec3) -> bool {
        self.region.contains(self.centre, self.yaw, p.x, p.z)
    }
}

/// A round of hers that would strike something this much short of the
/// target (metres) has no clear shot at it (invented: more than a
/// statue's depth).
const SHOT_SHORT: f32 = 3.0;
/// Within the scripts' bounds, what she can't shoot from where she is is
/// passed over for this long (seconds; invented: short of PmcCon034's
/// 90 s for all its targets).
const KEEP_PASS: f32 = 15.0;
/// With no clear shot she looks for a spot to shoot from on rings this
/// far round her (metres), on ground no more than this above or below
/// hers (her roof), the way to it sampled this often (metres), and is
/// there within this (all invented).
const PERCH_RINGS: [f32; 8] = [3.0, 6.0, 9.0, 12.0, 16.0, 20.0, 25.0, 30.0];
const PERCH_LEVEL: f32 = 1.5;
const PERCH_STRIDE: f32 = 1.0;
const PERCH_ARRIVE: f32 = 1.0;

/// The player playing herself.
#[derive(Debug, Default)]
pub struct Autoplay {
    /// The regions she must stay within (set each tick:
    /// [`crate::mission::MissionLink::kept_within`]).
    pub keep: Vec<Keep>,
    /// What the scripts wait for her to see (`Event.ObjectIsVisible`:
    /// PmcCon001's hall column, which brings the tank through the wall):
    /// where each is, set each tick ([`crate::mission::MissionLink::
    /// sight_targets`]).
    pub sights: Vec<Vec3>,
    /// The current contract's active deliveries (set each tick:
    /// [`crate::mission::MissionLink::deliveries`]).
    pub deliveries: Vec<Delivery>,
    /// The delivery she last got out at, and when: getting back into a
    /// vehicle while it still stands is a failed delivery, logged.
    pub delivery_exit: Option<(u32, f32)>,
    /// The actions the scripts offer: on what, and where it is (set each
    /// tick before [`step`](Self::step)).
    pub offers: Vec<(u32, Vec3)>,
    /// Seconds played.
    clock: f32,
    goal: Option<Goal>,
    /// What the goal is, for the log.
    what: String,
    /// The closest she has come to the goal (metres), and when.
    best: (f32, f32),
    /// Where she was when she last came closer having gone
    /// [`HEADWAY_MOVE`] from the time before, and when: the hop waits on
    /// this.
    moved: Option<(Vec3, f32)>,
    /// On foot: where she walks to, the nearest she has been to it (flat),
    /// and when she came that near ([`WANDER`]).
    wander: Option<(Vec3, f32, f32)>,
    /// The target the guns by her were last logged for.
    gun_logged: Option<u32>,
    /// Where the emplaced guns stand she fired dry (they are passed over).
    dry_guns: Vec<Vec3>,
    /// The rounds left in the emplaced gun she mans, as last seen.
    manned_rounds: Option<u32>,
    /// Targets an emplaced gun she manned couldn't turn to.
    uncovered: std::collections::HashSet<u32>,
    /// When she got to the place she is after, if she has.
    arrived: Option<f32>,
    /// The place she waits at (the scripts' object), and since when:
    /// kept while she goes for a vehicle standing by.
    waiting: Option<(u32, f32)>,
    /// Seconds she has been aiming steadily.
    steady: f32,
    /// Seconds she has been aimed with something in the way.
    blocked: f32,
    /// Where she goes to see the person she is shooting (`npcs[i]`, the
    /// spot), and the spots that turned out no better.
    vantage: Option<(usize, Vec3)>,
    tried: Vec<Vec3>,
    /// Soldiers she had no clear shot at and nowhere better to see from:
    /// where they stood, and until when she leaves them be
    /// ([`LEAVE_UNSEEN`]).
    unseen: Vec<(Vec3, f32)>,
    /// Where she goes to fire at what she is blowing up (its object, the
    /// spot); the spots no better go in `tried` too.
    blast_spot: Option<(u32, Vec3)>,
    /// The target last logged as one she has nothing to blow up with.
    no_gun_logged: Option<u32>,
    /// The target she last shot at with her rifle, and for how long
    /// within reach (seconds).
    rifle_on: Option<(u32, f32)>,
    /// The templates of targets that stood through her rifle that long.
    rifle_failed: std::collections::HashSet<u32>,
    /// The templates of what she has broken with her rifle, and that of
    /// what she shoots at now.
    rifle_broke: std::collections::HashSet<u32>,
    rifle_kind: Option<u32>,
    /// The target whose shot she judges by her eye's line, not her aim's.
    trust_sight: Option<u32>,
    /// Whether her rifle, last aimed, had no clear shot (and nowhere on
    /// her level to see the target from).
    blocked_logged: bool,
    /// Where she goes to shoot the target her rifle is on from (a spot on
    /// her own level, [`perch`]), and since when; the spots no better.
    perch: Option<(Vec3, f32)>,
    perch_tried: Vec<Vec3>,
    /// Where she is swimming to climb out of the water (see [`landing`]).
    landing: Option<Vec3>,
    /// Searches in a row for a spot to fire from that found none, and the
    /// things to blow up she passed over for now (object, until when).
    nowhere: u32,
    passed: Vec<(u32, f32)>,
    /// Ways out of the water in a row she didn't get out at.
    stranded: u32,
    /// With the launcher empty, the C4 she set on what she is blowing up
    /// and when, while she gets clear to set it off.
    planted: Option<f32>,
    /// When she last took a prompt of the scripts' at her goal.
    context_pressed: f32,
    /// Seconds a vehicle's cannon has been in range of her goal.
    gunning: f32,
    /// The vehicle she is going for and since when, and those she gave up
    /// going for.
    boarding: Option<(usize, f32)>,
    left_rides: Vec<usize>,
    /// Hops she has had driving to the goal she is after.
    drive_hops: u32,
    /// Sidestepping (-1 left, 1 right), seconds left.
    sidestep: Option<(f32, f32)>,
    /// Which way the next sidestep goes.
    side: f32,
    /// Backing a vehicle out: steering, seconds left.
    reversing: Option<(f32, f32)>,
    /// Seconds a vehicle has been stuck.
    stalled: f32,
    /// The vehicle she drove last, and all she has driven (indices into
    /// the scene's).
    ride: Option<usize>,
    ridden: Vec<usize>,
    /// Seconds since she last pressed to get in or out.
    pressed: f32,
    /// Getting out of the vehicle she is stuck in, to go on on foot.
    leave: bool,
    /// The way she is going, and when a roundabout one was last logged.
    way: Option<Way>,
    /// What her vehicles stuck on (each box's least and most corner, x
    /// and z, with room round it), kept out of on the ways she drives.
    avoid: Vec<(Vec2, Vec2)>,
    /// Until when (her clock) she clears the soldiers round a truck with
    /// goods on it before driving it ([`CARGO_GUARD`]), once begun.
    guard_until: std::cell::Cell<Option<f32>>,
    /// Where the scripts' delivery she calls in is to land (a deliverable
    /// place): she faces it to throw the designator.
    deliver_at: std::cell::Cell<Option<Vec3>>,
    /// Until when she hurries: a countdown of the scripts' ticked lately.
    hurry_until: f32,
    /// When the vehicles near her boat were last logged.
    near_logged: f32,
    way_logged: f32,
    /// The island's roads as one graph, made the first time a place is
    /// far ([`Autoplay::by_road`]).
    roads: Option<RoadGraph>,
    /// The way by road to a far place: the place, and the road points.
    road_way: Option<(Vec3, Vec<Vec3>)>,
    /// Hops: from, to.
    pub hops: Vec<(Vec3, Vec3)>,
    /// The objectives last logged.
    objectives: Vec<String>,
    next_report: f32,
    /// The supply crate she is opening and since when, and those she
    /// gave up on.
    opening: Option<(u32, f32)>,
    left_shut: Vec<u32>,
    /// The crate whose pickup she is going for and since when, and those
    /// whose pickups she gave up on.
    taking: Option<((u32, u32), f32)>,
    left_lying: Vec<(u32, u32)>,
    /// Fire as last pressed at the support (it takes presses, not holds),
    /// and the support's step last logged.
    tapped: bool,
    support_logged: &'static str,
    /// When she opened her PDA, while it is open; seconds since she last
    /// pressed its key; when the scripts began waiting for it to open,
    /// while they wait.
    pda_since: Option<f32>,
    pda_pressed: f32,
    pda_asked: Option<f32>,
    /// The action objective's object whose prompt she last took, and
    /// when; and the one she was last logged at without its prompt.
    acted: Option<(u32, f32)>,
    short_logged: Option<u32>,
    /// The helicopter she rides in, where it stood still and since when.
    ride_still: Option<(u32, Vec3, f32)>,
    /// The helicopter she got into, and when.
    boarded: Option<(u32, f32)>,
    /// When she last called in a soldier delivery by an outpost.
    delivered: Option<f32>,
    /// Seconds one following her has been left behind.
    left_behind: f32,
}

impl Autoplay {
    /// One tick: how she plays it, from the scripts' markers. `objectives`
    /// (as shown) are logged as they change. May hop her ahead, or put
    /// her back on the ground.
    pub fn step(
        &mut self,
        game: &mut Game,
        marks: &[Mark],
        objectives: &[String],
        awaited: &[String],
        dt: f32,
    ) -> Intent {
        self.clock += dt;
        if std::mem::take(&mut game.countdown_tick) {
            self.hurry_until = self.clock + HURRY_FOR;
        }
        self.pressed += dt;
        self.pda_pressed += dt;
        if objectives != self.objectives.as_slice() {
            eprintln!("autoplay {:.1} s: objectives {objectives:?}", self.clock);
            self.objectives = objectives.to_vec();
        }
        let Some(p) = &game.scene.player else {
            return Intent::default();
        };
        if !game.on_foot || !p.landed || game.combat.down > 0.0 {
            return Intent::default();
        }
        match p.driving {
            Some(i) => {
                // Shot to a wreck under her: out, on foot or into another.
                if !self.leave && game.scene.vehicles.get(i).is_some_and(|v| v.wrecked) {
                    eprintln!(
                        "autoplay {:.1} s: the {} is wrecked: getting out",
                        self.clock, game.scene.vehicles[i].spec.label
                    );
                    self.leave = true;
                    self.fresh_headway();
                }
                self.ride = Some(i);
                if !self.ridden.contains(&i) {
                    self.ridden.push(i);
                }
            }
            None => self.leave = false,
        }
        self.ground(game);
        // Those following her who fall behind (on foot, or while she
        // drives): brought to her after a while (a test shortcut).
        let here = game.focus();
        let waited = self
            .waiting
            .is_some_and(|(_, since)| self.clock - since > WAITED_FOR);
        let behind = game.scene.people.followers().iter().any(|f| {
            let d = f.with_y(0.0).distance(here.with_y(0.0));
            d > LEFT_BEHIND
                || (waited && d > LEFT_AT_PLACE)
                || (f.y - here.y).abs() > UNDER_BY * 3.0
        });
        self.left_behind = if behind { self.left_behind + dt } else { 0.0 };
        if self.left_behind > LEFT_FOR
            && game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_none())
        {
            self.left_behind = 0.0;
            let n = game.scene.people.bring_followers(here);
            eprintln!(
                "autoplay {:.1} s: {n} following her left behind: brought to her at {here:.1} (test shortcut)",
                self.clock
            );
        }
        // The scripts wait for her to look at her PDA (the first contract,
        // at the first gate): she opens it, and shuts it again.
        let wants = |name: &str| awaited.iter().any(|a| a == name);
        let asked = wants(PDA_OPEN)
            && game.support.phase == Phase::Idle
            && game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_none());
        self.pda_asked = asked.then(|| self.pda_asked.unwrap_or(self.clock));
        if !game.support.pda
            && self
                .pda_asked
                .is_some_and(|since| self.clock - since < PDA_PROMPT)
        {
            // Its prompt isn't up yet: she waits for it.
            return Intent::default();
        }
        if game.support.pda || asked {
            return self.pda(game.support.pda, wants(PDA_CLOSE));
        }
        // The scripts offer their context action on what a marker of
        // theirs is on, and she stands at it (a prisoner to release,
        // `[ContextAction.ReleasePrisoner]`): she takes it with the use key.
        if let Some((g, text)) = game.context.clone()
            && marks.iter().any(|m| m.alive && m.target == g)
        {
            // As the use key does in play (`Game::tick`, which her
            // intents don't pass through), as `--context` does too.
            if self.pressed > PRESS_GAP && !game.input_locked {
                self.pressed = 0.0;
                eprintln!("autoplay {:.1} s: taking {text:?} at {g:#x}", self.clock);
                game.context_taken = Some(g);
            }
            return Intent::default();
        }
        let chosen = self.choose(game, marks);
        let changed = match (&chosen, &self.goal) {
            (Some((a, _)), Some(b)) => !a.same(b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.what = chosen
                .as_ref()
                .map_or_else(|| "nothing to do".to_owned(), |c| c.1.clone());
            eprintln!(
                "autoplay {:.1} s: {}, from {:.1}",
                self.clock,
                self.what,
                game.focus()
            );
            self.fresh_headway();
            self.arrived = None;
            self.steady = 0.0;
            self.blocked = 0.0;
            self.sidestep = None;
            self.reversing = None;
            self.stalled = 0.0;
            self.way = None;
            self.gunning = 0.0;
            self.drive_hops = 0;
        }
        self.goal = chosen.map(|c| c.0);
        // Got out to deliver; back into a vehicle while the delivery still
        // stands is a delivery that failed (b3i: 1,985 times), logged.
        if changed {
            let standing = |g: u32| self.deliveries.iter().any(|d| d.dest == g);
            if matches!(self.goal, Some(Goal::GetOut))
                && let Some((g, _)) = self.waiting
                && standing(g)
            {
                self.delivery_exit = Some((g, self.clock));
            } else if self.what.starts_with("getting into")
                && let Some((g, at)) = self.delivery_exit.take()
                && standing(g)
            {
                eprintln!(
                    "autoplay {:.1} s: delivery failed: got out at {g:#x} {:.1} s ago and it still \
                     stands: {}",
                    self.clock,
                    self.clock - at,
                    self.what
                );
            }
        }
        // The scripts' prompt on what she went to (a verify on a body, a
        // starter's talk) is taken with the use key, as a player would.
        if let (Some(Goal::Reach { target, .. }), Some((g, text))) = (&self.goal, &game.context)
            && g == target
            && self.clock - self.context_pressed > PRESS_AGAIN
        {
            eprintln!(
                "autoplay {:.1} s: taking {text:?} at {}",
                self.clock, self.what
            );
            self.context_pressed = self.clock;
            game.context_taken = Some(*g);
        }
        // At a body to verify still driving: out, as the scripts offer the
        // prompt on foot (ChiCon003's HVT, shot from the M1A2: she sat on
        // him 1,400 s, the prompt never offered).
        if let (Some(Goal::Reach { target, .. }), Some(_)) = (&self.goal, self.arrived)
            && !self.leave
            && game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_some())
            && game
                .scene
                .people
                .npcs
                .iter()
                .any(|n| n.spawn == Some(*target) && !n.alive)
        {
            eprintln!(
                "autoplay {:.1} s: at the body to verify, driving: getting out",
                self.clock
            );
            self.leave = true;
        }
        if let (Some(Goal::Reach { target, .. }), Some(t)) = (&self.goal, self.arrived)
            && self.waiting.is_none_or(|w| w.0 != *target)
        {
            self.waiting = Some((*target, t));
        }
        if self.clock >= self.next_report {
            self.next_report = self.clock + REPORT;
            self.report(game);
        }
        // In a helicopter she can't fly (round 38's AllCon002: boarding at
        // the HQ she got the Coanda by the tank, which climbed to 62 km):
        // out, and not that one again.
        if let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving)
            && game.scene.vehicles[i].spec.is_helicopter()
            && !self.leave
        {
            eprintln!(
                "autoplay {:.1} s: in the {}, which she doesn't fly: getting out",
                self.clock, game.scene.vehicles[i].spec.label
            );
            self.leave = true;
            if !self.left_rides.contains(&i) {
                self.left_rides.push(i);
            }
        }
        let sturdy = |target: &u32| game.scene.props.sturdy.contains(target);
        // Her launcher spent (round 13: three tanks took all her rockets,
        // and she stood aimed at the fourth for 2100 s), a strike brings
        // down what she would have blown up.
        let spent = game.combat.launcher_rounds.is_some() && game.combat.launcher_empty();
        // Unless an emplaced gun in reach of it does instead (`blast`).
        let dry = self.dry_guns.clone();
        let gun = |at: &Vec3| {
            game.combat.manned.is_some()
                || game.scene.player.as_ref().is_some_and(|p| {
                    game.placed
                        .emplacement_for(
                            &game.scene.props,
                            p.character.position,
                            *at,
                            MAN_SEEK,
                            MANNED_REACH,
                            |g| {
                                !dry.iter().any(|d| {
                                    d.with_y(0.0).distance(g.with_y(0.0)) < crate::placed::MAN_REACH
                                })
                            },
                        )
                        .is_some()
                })
        };
        // Nor while she drives something with a cannon, which fires on it
        // (PmcCon016 round 4: out of the race's Panhard to call a bomb on a
        // pylon, and the race ran out).
        let cannon = game
            .scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| game.scene.vehicles.get(i))
            .is_some_and(|v| v.spec.cannon());
        // (Not a sturdy one first: PmcCon031 round 11 tried her rifle on
        // each of its 50 columns in turn and brought none down.)
        let rifle = matches!(&self.goal, Some(Goal::Blast { target, at })
            if takes_rounds(game, *target)
                && Self::rifle_ready(game)
                // Not with an emplaced gun by her whose rounds blow up and
                // reach it (`blast`).
                && !game
                    .scene
                    .player
                    .as_ref()
                    .is_some_and(|p| self.gun_by(game, p.character.position, *target, *at).is_some())
                // Nor manning one (the last car out of its reach).
                && game.combat.manned.is_none()
                // Not one that has stood through her rounds a while: blown
                // up instead (PmcCon031 round 14: its columns stood through
                // 240 rounds from the MG platform).
                // Nor any other of its kind after that.
                && !game
                    .scene
                    .props
                    .get(*target)
                    .is_some_and(|(p, _)| self.rifle_failed.contains(&p.template))
                && self
                    .rifle_on
                    .is_none_or(|(t, spent)| t != *target || spent < self.rifle_try(game, t)));
        // One of a kind her rounds have broken before is only passed over
        // for now: the rest of its kind still fall to them (PmcCon031 round
        // 28: one column of 50, behind the MG platform's rail, and she put
        // her rockets into all the rest).
        if let Some((t, spent)) = self.rifle_on
            && spent >= self.rifle_try(game, t)
            && let Some((p, _)) = game.scene.props.get(t)
            && self.rifle_broke.contains(&p.template)
        {
            eprintln!(
                "autoplay {:.1} s: {t:#x} stood through {spent:.0} s of her rifle: passed over \
                 for {KEEP_PASS:.0} s (her rounds broke others of its kind)",
                self.clock
            );
            self.passed.push((t, self.clock + KEEP_PASS));
            self.rifle_on = None;
        } else if let Some((t, spent)) = self.rifle_on
            && spent >= RIFLE_TRY
            && let Some((p, _)) = game.scene.props.get(t)
            && self.rifle_failed.insert(p.template)
        {
            eprintln!(
                "autoplay {:.1} s: {t:#x} stood through {spent:.0} s of her rifle: its kind \
                 ({:08x}) is blown up from now on",
                self.clock, p.template
            );
        }
        let striking = !rifle
            && matches!(&self.goal, Some(Goal::Blast { target, at }) if (sturdy(target) && !cannon) || (spent && !gun(at) && !cannon))
            || matches!(self.goal, Some(Goal::Deliver(_)));
        // A call she is no longer after is put away: the designator goes
        // with a weapon taken up (twice from the satellite's view).
        if !striking && (game.support.busy() || game.support.holding()) {
            return Intent {
                trigger: Trigger {
                    select: Some(Weapon::Rifle),
                    ..Trigger::default()
                },
                ..Intent::default()
            };
        }
        // Riding in the scripts' helicopter (the Lucky Lady): she sits
        // until it has stood still a while and what she is after is
        // elsewhere, then gets out.
        if let Some(h) = game.aboard {
            let here = game.focus();
            let since = match self.ride_still {
                Some((g, at, t)) if g == h && at.distance(here) < 1.0 => t,
                _ => {
                    self.ride_still = Some((h, here, self.clock));
                    self.clock
                }
            };
            let after_it = matches!(&self.goal, Some(Goal::Reach { target, .. }) if *target == h);
            // Still marked a while after she got in: the objective that
            // waits for her getting in came after she did (PmcCon003's
            // comes with Ewan's last line, round 4): out, and in again.
            let boarded = match self.boarded {
                Some(b) if b.0 == h => b,
                _ => *self.boarded.insert((h, self.clock)),
            };
            if after_it && self.clock - boarded.1 > RIDE_AGAIN && game.context_taken.is_none() {
                eprintln!(
                    "autoplay {:.1} s: {h:#x} still marked {RIDE_AGAIN:.0} s after getting in: out and in again",
                    self.clock
                );
                game.context_taken = Some(h);
                self.boarded = None;
                self.acted = Some((h, self.clock));
                return Intent::default();
            }
            if !after_it && self.clock - since > RIDE_STILL && game.context_taken.is_none() {
                eprintln!(
                    "autoplay {:.1} s: {h:#x} still for {RIDE_STILL:.0} s at {here:.1}: getting out",
                    self.clock
                );
                game.context_taken = Some(h);
                self.ride_still = None;
                self.boarded = None;
            }
            return Intent::default();
        }
        let acting = match &self.goal {
            Some(Goal::Reach { target, .. }) => marks
                .iter()
                .find(|m| m.target == *target && m.kind == ACTION_KIND)
                .map(|m| (m.target, m.at)),
            _ => None,
        };
        if let Some((target, at)) = acting {
            // Prompts are taken on foot: driving up to it she gets out
            // (round 30 sat in a tank 6 m from PlayerLocation_PMC for
            // 3700 s).
            let driving = game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_some());
            if driving && !self.leave && (at - game.focus()).with_y(0.0).length() < ACTION_GET_OUT {
                eprintln!(
                    "autoplay {:.1} s: by {target:#x}'s prompt in a vehicle: getting out",
                    self.clock
                );
                self.leave = true;
            }
            self.act(game, target, at);
        }
        // A place to get herself to (a delivery of her, on foot) is gone
        // to as close as an action's: its distance can be under her own
        // arrival's (PmcCon033 round 7: 2.3 m from the scripts, and she
        // stood 450 s 3 m off the pistol range's point).
        let close = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_none())
            && matches!(&self.goal, Some(Goal::Reach { target, .. }) if marks
                .iter()
                .any(|m| m.target == *target && m.kind == "deliverable"));
        // Its rounds spent: off it, for good (PmcCon031 round 27: she stood
        // 190 s aimed at a column with the grenade launcher's 50 gone).
        if let Some(m) = &game.combat.manned
            && m.rounds == 0
        {
            if !self.fired_dry(m.pivot) {
                eprintln!(
                    "autoplay {:.1} s: the emplaced gun {:#x} is out of rounds: off it",
                    self.clock, m.entity
                );
                self.dry_guns.push(m.pivot);
            }
            return Intent {
                use_pressed: true,
                ..Intent::default()
            };
        }
        // Manning an emplaced gun, with nothing left for it to fire on:
        // off it (the use key again), to go on. With nothing at all to do
        // she stays on it: PmcCon031's cars come one at a time, up to 10 s
        // apart, each to be hit within 9 s (from the scripts).
        if game.combat.manned.is_some() && !matches!(self.goal, Some(Goal::Blast { .. }) | None) {
            return Intent {
                use_pressed: true,
                ..Intent::default()
            };
        }
        match self.goal.clone() {
            None => self.idle(game, dt),
            Some(Goal::Reach { at, .. }) if acting.is_some() || close => {
                self.travel(game, at, ACTION_ARRIVE, dt)
            }
            Some(Goal::Reach { at, .. }) => self.travel(game, at, ARRIVE, dt),
            Some(Goal::Shoot(i)) => self.shoot(game, i, dt),
            Some(Goal::Blast { target, at }) if rifle => self.rifle_at(game, target, at, dt),
            Some(Goal::Blast { at, .. }) if striking => self.strike(game, at, dt),
            Some(Goal::Blast { target, at }) => self.blast(game, target, at, dt),
            Some(Goal::Board(i)) => self.board(game, i, dt),
            Some(Goal::Open(target)) => self.open(game, target, dt),
            Some(Goal::Take(key)) => self.take(game, key, dt),
            Some(Goal::Deliver(key)) => self.deliver(game, &key, dt),
            Some(Goal::Use { target, at }) => {
                let here = game.focus();
                let driving = game.scene.player.as_ref().and_then(|p| p.driving);
                if driving.is_none() && here.with_y(0.0).distance(at.with_y(0.0)) < USE_FROM {
                    // The action the game offers her there is taken with
                    // the use key, as the keyboard's use key takes it
                    // (`Game::tick`), a press at a time.
                    let offered = game.context.as_ref().is_some_and(|c| c.0 == target);
                    if offered && self.pressed > PRESS_GAP {
                        self.pressed = 0.0;
                        eprintln!(
                            "autoplay {:.1} s: taking the action at {target:#x}",
                            self.clock
                        );
                        game.context_taken = Some(target);
                    }
                    Intent::default()
                } else {
                    self.travel(game, at, USE_FROM * 0.6, dt)
                }
            }
            Some(Goal::GetOut) => match game.scene.player.as_ref().and_then(|p| p.driving) {
                Some(i) => {
                    let speed = vehicle_state(game, i).map_or(0.0, |s| s.1);
                    self.press(speed)
                }
                None => self.idle(game, dt),
            },
        }
    }

    /// The emplaced gun by `here` ([`GUN_BY`]) that breaks `target` (at
    /// `at`): one whose rounds blow up, or any if rounds break it
    /// ([`takes_rounds`]); that reaches and turns to it
    /// ([`BLAST_GUN_REACH`]) and that she hasn't fired dry: where it stands.
    /// (PmcCon031's heavy MG broke its last three columns in a second, her
    /// rifle's 240 rounds spent on the rest, round 36 of its thread.)
    fn gun_by(&self, game: &Game, here: Vec3, target: u32, at: Vec3) -> Option<Vec3> {
        // Not for one a gun here couldn't turn to: the two heavy MGs on
        // PmcCon031's platform stand 1.5 m apart, and she walked to one and
        // manned the other, on and off it 5 times a tick (round 38).
        if self.uncovered.contains(&target) {
            return None;
        }
        let w = game.scene.world.as_ref()?;
        let props = &game.scene.props;
        let rounds = takes_rounds(game, target);
        game.placed
            .emplacement_for(props, here, at, GUN_BY, BLAST_GUN_REACH, |g| {
                !self.fired_dry(g)
                    && game
                        .placed
                        .emplacement_near(props, &w.index.template_db, g)
                        .is_some_and(|m| {
                            (rounds || m.weapon.ammo.blast.is_some())
                                && gun_point(game, &m, target, at).is_some()
                        })
            })
    }

    /// How long (seconds, with a clear shot) she gives her rifle on
    /// `target` before passing it over: less for a kind it has broken.
    /// Only near: far off, her rounds take longer (PmcCon034's statues at
    /// 150 m, passed over after 5 s, round 39 of PmcCon031's thread).
    fn rifle_try(&self, game: &Game, target: u32) -> f32 {
        let here = game.focus();
        match game.scene.props.get(target) {
            Some((p, _))
                if self.rifle_broke.contains(&p.template)
                    && p.position.distance(here) < SHOOT_RANGE =>
            {
                RIFLE_RETRY
            }
            _ => RIFLE_TRY,
        }
    }

    /// Whether the emplaced gun standing at `g` is one she fired dry.
    fn fired_dry(&self, g: Vec3) -> bool {
        self.dry_guns
            .iter()
            .any(|d| d.with_y(0.0).distance(g.with_y(0.0)) < crate::placed::MAN_REACH)
    }

    /// At an action objective's object `target` (`MrxTaskObjectiveAction`:
    /// "Enter office", "Board the Lucky Lady"), the prompt the scripts put
    /// on it (`Pg.AddContextAction`) is taken with the use key, as a player
    /// would; again after [`ACTION_AGAIN`] while it is still offered. There
    /// and not offered it (above or below her), she says so once.
    fn act(&mut self, game: &mut Game, target: u32, at: Vec3) {
        let offered = game.context.as_ref().is_some_and(|(g, _)| *g == target);
        let again = self
            .acted
            .is_none_or(|(g, t)| g != target || self.clock - t > ACTION_AGAIN);
        if offered && again && !game.input_locked && game.context_taken.is_none() {
            eprintln!(
                "autoplay {:.1} s: taking the prompt at {target:#x}",
                self.clock
            );
            game.context_taken = Some(target);
            self.acted = Some((target, self.clock));
        } else if !offered && self.arrived.is_some() && self.short_logged != Some(target) {
            let here = game.focus();
            eprintln!(
                "autoplay {:.1} s: at {target:#x} ({at:.1}) from {here:.1}, no prompt offered",
                self.clock
            );
            self.short_logged = Some(target);
            // Under or over it (the oil rig's office, a deck 43 m over
            // the ground she walked to: round 3), with no way up found:
            // she hops onto its floor, as a stuck way does.
            if (at.y - here.y).abs() > ACTION_RISE {
                let still = self.since_moved();
                self.hop(game, at, still);
                self.arrived = None;
                self.fresh_headway();
            }
        }
    }

    /// Calls in the support item `key` she holds, as a player does: her
    /// PDA shut, the item chosen in the support menu and taken, its
    /// designator thrown (or aimed and confirmed).
    fn deliver(&mut self, game: &mut Game, key: &str, dt: f32) -> Intent {
        let mut intent = Intent::default();
        if game.support.pda {
            self.tapped = !self.tapped;
            intent.trigger.pda = self.tapped;
            return intent;
        }
        // The menu open again with the smoke already handed her (round 12
        // reopened it the frame after the call, its item not yet taken off
        // her stock, and stood 3300 s with the menu up and the smoke
        // unthrown): a weapon taken up shuts it, then the throw.
        if game.combat.to_throw && game.support.phase == Phase::Menu {
            intent.trigger.select = Some(Weapon::Rifle);
            return intent;
        }
        if game.combat.to_throw {
            // Towards where it is to land, if that is known: a throw the
            // other way can fall outside a drop zone's reach.
            if let (Some(at), Some(p)) = (self.deliver_at.get(), game.scene.player.as_ref()) {
                let to = (at - p.character.position).with_y(0.0);
                let yaw = to.x.atan2(to.z);
                if wrap(yaw - p.yaw).abs() > FACE_THROW {
                    intent.look = turn_to(p, yaw, CRUISE_PITCH, dt);
                    return intent;
                }
            }
            intent.trigger.throw = true;
            return intent;
        }
        match game.support.phase {
            // Not again while the call just made is answered.
            Phase::Idle
                if self
                    .delivered
                    .is_some_and(|t| self.clock - t < DESIGNATOR_WAIT) => {}
            Phase::Idle => {
                if let Some(i) = game
                    .support
                    .entries()
                    .iter()
                    .position(|e| matches!(e, Entry::Stocked(s) if s.key == key))
                {
                    game.support.selected = i;
                    intent.trigger.support = true;
                }
            }
            Phase::Menu => {
                intent.use_pressed = true;
                self.delivered = Some(self.clock);
                eprintln!("autoplay {:.1} s: calling in {key}", self.clock);
            }
            Phase::Designating { aim, .. } => match aim {
                Aim::Satellite => {
                    self.tapped = !self.tapped;
                    intent.trigger.fire = self.tapped;
                }
                Aim::Laser => intent.trigger.fire = true,
            },
            _ => {}
        }
        intent
    }

    /// What to do next, and words for the log.
    fn choose(&self, game: &Game, marks: &[Mark]) -> Option<(Goal, String)> {
        // A countdown of the scripts' running (OilCon002's 15 minutes from
        // the second post; rounds 16 and 18 lost it to fights and pickups on
        // the way): only close fights, no detours for crates or pickups.
        let hurry = self.clock < self.hurry_until;
        let p = game.scene.player.as_ref()?;
        let here = game.focus();
        let npcs = &game.scene.people.npcs;
        let vehicles = &game.scene.vehicles;
        let driving = p.driving;
        // Soldiers fighting her close by, while she's on foot; and those
        // round a truck with goods on it near her, before she drives it
        // off (PirCon003 round 61: three ran after the parrot truck and
        // a grenade broke its boxes when it caught on a wall).
        // For at most [`GUARD_MOST`]: a base keeps sending more
        // (PirCon004 round 75: 1000 s went on its soldiers and their
        // drops before she took the truck).
        let guarding = self
            .guard_until
            .get()
            .is_none_or(|until| self.clock < until);
        let guarded: Vec<Vec3> = (0..vehicles.len())
            .filter(|_| guarding)
            .filter(|&j| game.scene.props.carrying(j))
            .filter_map(|j| vehicle_at(game, j))
            .map(|t| t.w_axis.truncate())
            .filter(|c| c.distance(here) < CARGO_NEAR)
            .collect();
        // Not while she swims: she can't fire (OilCon002 round 3), and
        // getting on to dry ground is the way out.
        // Nor with a delivery's smoke in her hand: thrown first, a second's
        // work (else it is put away and the call is lost).
        let throwing = matches!(self.goal, Some(Goal::Deliver(_)))
            && (game.combat.to_throw
                || self
                    .delivered
                    .is_some_and(|t| self.clock - t < DESIGNATOR_WAIT));
        if driving.is_none()
            && !p.swimming()
            && !throwing
            && let Some((i, d)) = npcs
                .iter()
                .enumerate()
                .filter(|(_, n)| n.alive && n.soldier && n.alert)
                .filter(|(_, n)| {
                    !self.unseen.iter().any(|&(at, until)| {
                        until > self.clock && at.distance(n.character.position) < UNSEEN_NEAR
                    })
                })
                .map(|(i, n)| {
                    let at = n.character.position;
                    (
                        i,
                        at.distance(here),
                        guarded.iter().any(|c| c.distance(at) < CARGO_GUARD),
                    )
                })
                .filter(|&(_, d, guard)| d < if hurry { DEFEND_HURRIED } else { DEFEND } || guard)
                .inspect(|&(_, d, _)| {
                    if d >= DEFEND && self.guard_until.get().is_none() {
                        self.guard_until.set(Some(self.clock + GUARD_MOST));
                    }
                })
                .map(|(i, d, _)| (i, d))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Shoot(i),
                format!("fighting back a soldier {d:.0} m off"),
            ));
        }
        let mut live: Vec<&Mark> = marks.iter().filter(|m| m.alive).collect();
        // A place saved for last waits while others to deliver to are open.
        if live
            .iter()
            .any(|m| m.kind == "deliverable" && !SAVE_FOR_LAST.contains(&m.name.as_str()))
        {
            live.retain(|m| !SAVE_FOR_LAST.contains(&m.name.as_str()));
        }
        fn nearest(ms: Vec<&Mark>, here: Vec3) -> Option<&Mark> {
            ms.into_iter()
                .min_by(|a, b| a.at.distance(here).total_cmp(&b.at.distance(here)))
        }
        // An outpost to capture: there, the soldier delivery she holds
        // called in (its soldiers take the point), then she stands by.
        let outposts: Vec<&Mark> = live
            .iter()
            .copied()
            .filter(|m| m.kind == "outpost")
            .collect();
        if let Some(m) = nearest(outposts, here) {
            let d = m.at.distance(here);
            let held = game.support.entries().into_iter().find_map(|e| match e {
                Entry::Stocked(s) if s.key.starts_with("SoldierDelivery") => Some(s.key.clone()),
                _ => None,
            });
            let again = self
                .delivered
                .is_none_or(|t| self.clock - t > DELIVERY_AGAIN);
            if d < OUTPOST_NEAR
                && driving.is_none()
                && let Some(key) = held.filter(|_| again || game.support.phase != Phase::Idle)
            {
                self.deliver_at.set(None);
                return Some((
                    Goal::Deliver(key.clone()),
                    format!(
                        "calling in {key} by the outpost {} ({:#x})",
                        m.name, m.target
                    ),
                ));
            }
            return Some((
                Goal::Reach {
                    target: m.target,
                    at: m.at,
                },
                format!(
                    "taking the outpost {} ({:#x}), {d:.0} m off",
                    m.name, m.target
                ),
            ));
        }
        // A supply crate down near her or marked (or the one she set out
        // to open): broken open first, for what is inside.
        if driving.is_none()
            && let Some((target, d)) = game
                .scene
                .flights
                .crates_standing()
                .filter(|g| !self.left_shut.contains(g))
                .filter_map(|g| {
                    let (at, _) = game.scene.flights.crate_standing(g)?;
                    let d = at.distance(here);
                    let wanted = live.iter().any(|m| m.target == g)
                        || self.opening.is_some_and(|o| o.0 == g)
                        || (!hurry && d < OPEN_NEAR);
                    wanted.then_some((g, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Open(target),
                format!("opening the supply crate {target:#x}, {d:.0} m off"),
            ));
        }
        // What a crate she broke open left for her: taken before going on.
        let none = Templates::default();
        let templates = game
            .scene
            .world
            .as_ref()
            .map_or(&none, |w| &w.index.template_db);
        // Places to deliver to count too: OilCon002's last post has 15 minutes
        // (round 11 went 240 m back for grenades and missed it by 30 s).
        let destroying = live
            .iter()
            .any(|m| m.kind == "destroy" || m.kind == "deliverable");
        let rockets_out = game.combat.second == Some(Weapon::Rpg)
            && game.combat.launcher_rounds.is_none_or(|r| r.total() == 0)
            && live.iter().any(|m| m.kind == "destroy");
        let rifle_out = game.combat.rifle_rounds.total() == 0
            && live
                .iter()
                .any(|m| m.kind == "destroy" && takes_rounds(game, m.target));
        if driving.is_none()
            && let Some((from, d)) = game
                .scene
                .flights
                .pickups()
                .iter()
                // What the layers place she leaves to the player: going out
                // of her way for each would keep her from the contract.
                // (Cheapest first: there are hundreds placed.)
                // Except rockets with the launcher empty and something
                // left to blow up (round 4's AllCon002: the boats).
                .filter(|p| {
                    let d = p.at.distance(here);
                    // Her rifle empty too, with targets left that rounds
                    // break (PmcCon032 round 14: 13 of 21 statues, then
                    // no rounds left). (Looked up only then: there are
                    // over a thousand pickups, and this runs each tick.)
                    let restock = (rockets_out || rifle_out) && d < RESTOCK_NEAR && {
                        let refill = crate::mission::refill(templates, &game.combat, &p.what);
                        rockets_out && refill.launcher > 0 || rifle_out && refill.rifle > 0
                    };
                    // With the contract's things still to blow up, only
                    // what lies close is worth the detour (round 16's
                    // AllCon002 lost the boats' race to pickups).
                    let reach = if destroying { BUSY_NEAR } else { OPEN_NEAR };
                    // Nor what lies out of her depth: she can't fight
                    // swimming (OilCon002 round 17 swam 1100 s by the first
                    // post after a dead soldier's rifle in the harbour).
                    let sunk = game.scene.world.as_ref().is_some_and(|w| {
                        w.physics
                            .water_at(p.at.x, p.at.z)
                            .is_some_and(|s| s - p.at.y > crate::player::SWIM_START)
                    });
                    (restock || (!hurry && !game.placed.is_pickup(p.from) && d < reach))
                        && !sunk
                        && !self.left_lying.contains(&p.key())
                        && wanted(templates, &game.combat, p)
                })
                .map(|p| (p.key(), p.at.distance(here)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Take(from),
                format!(
                    "taking the {:#x} left by {:#x}, {d:.1} m off",
                    from.1, from.0
                ),
            ));
        }
        // A person to verify (AllCon003's General Peng): shot down once
        // she is near (a ride first if far), then she goes to the body,
        // where the scripts offer their verify prompt (taken in `step`).
        let verify: Vec<&Mark> = marks.iter().filter(|m| m.kind == "verify").collect();
        if let Some(m) = nearest(verify, here)
            && let Some(i) = npcs.iter().position(|n| n.spawn == Some(m.target))
        {
            let at = npcs[i].character.position;
            let d = at.distance(here);
            // Out of a rifle's reach, she goes up to them first (jobs round
            // 5: GurJob002's seventh stood 51 m off, and she aimed at him
            // there for 700 s).
            if npcs[i].alive && (SHOOT_RANGE..RIDE_FOR).contains(&d) {
                return Some((
                    Goal::Reach {
                        target: m.target,
                        at,
                    },
                    format!(
                        "closing on {} ({:#x}) to verify, {d:.0} m off",
                        m.name, m.target
                    ),
                ));
            }
            if npcs[i].alive && d < RIDE_FOR {
                return Some((
                    Goal::Shoot(i),
                    format!(
                        "shooting {} ({:#x}) to verify, {d:.0} m off",
                        m.name, m.target
                    ),
                ));
            }
            if !npcs[i].alive {
                return Some((
                    Goal::Reach {
                        target: m.target,
                        at,
                    },
                    format!("verifying {} ({:#x}), {d:.0} m off", m.name, m.target),
                ));
            }
        }
        // What to destroy, nearest first: soldiers are shot, things blown up.
        // An HVT to verify, once down: the scripts offer an action on the
        // body a few seconds on (`MrxTaskObjectiveVerify._SetupVerify`),
        // which she takes there.
        if let Some(&(target, at)) = self
            .offers
            .iter()
            .find(|(g, _)| marks.iter().any(|m| m.target == *g && m.kind == "verify"))
        {
            return Some((
                Goal::Use { target, at },
                format!(
                    "verifying {target:#x}: the action at {at:.1}, {:.0} m off",
                    at.distance(here)
                ),
            ));
        }
        // Not what someone she is to reach stands on (ChiCon001's party
        // official on a rooftop, among the bonus objective's buildings): it
        // waits until they are reached.
        // Nor what one following her stands on (him, released), nor what
        // she stands on herself (that roof, as she releases him).
        // Nor anything a strike would bring down near one following her
        // (the bomb's blast killed him, ChiCon001).
        let followers = game.scene.people.followers();
        let held_up = |m: &Mark| {
            live.iter()
                .filter(|n| matches!(n.kind.as_str(), "action" | "deliverable"))
                .map(|n| n.at)
                .chain(followers.iter().copied())
                .any(|at| {
                    at.y > m.at.y + HOLDS_UP_RISE
                        && at.with_y(0.0).distance(m.at.with_y(0.0)) < HOLDS_UP
                })
                // Herself only on it, her feet at its box: from PmcCon031's
                // MG platform, 4 m over its columns, those within 20 m were
                // passed over, and 12 stood (round 24 of its thread).
                || game
                    .scene
                    .props
                    .touches(m.target, here - Vec3::Y * STANDS_ON, STANDS_ON)
                    .unwrap_or(false)
                || followers.iter().any(|f| {
                    f.with_y(0.0).distance(m.at.with_y(0.0)) < crate::combat::BOMB_FRAG.0 + HOLDS_UP
                })
        };
        // What to destroy, and HVTs to verify still standing: shot first.
        let mut destroy: Vec<&Mark> = live
            .iter()
            .copied()
            .filter(|m| {
                (m.kind == "destroy" && !held_up(m))
                    || (m.kind == "verify"
                        && npcs.iter().any(|n| n.spawn == Some(m.target) && n.alive))
            })
            .collect();
        // Not one she found nowhere to fire at for now, while others are
        // left (round 31's AllCon002: the second boat under its bank).
        let passed = |m: &&Mark| {
            self.passed
                .iter()
                .any(|&(t, until)| t == m.target && self.clock < until)
        };
        if destroy.iter().any(|m| !passed(m)) {
            destroy.retain(|m| !passed(m));
        }
        // The one she is on keeps her unless another is much nearer: of a
        // range's targets side by side the nearest changed as she stepped,
        // and she never fired (PmcCon033 round 8: five targets 20 m off,
        // a new one each 0.2 s for 470 s).
        let current = match &self.goal {
            Some(Goal::Blast { target, .. }) => {
                destroy.iter().copied().find(|m| m.target == *target)
            }
            _ => None,
        };
        let pick = match (current, nearest(destroy.clone(), here)) {
            (Some(c), Some(n)) if n.at.distance(here) + KEEP_TARGET_BY >= c.at.distance(here) => {
                Some(c)
            }
            (_, n) => n,
        };
        // On an emplaced gun with rounds left, what is out of its reach
        // waits: between PmcCon031's flung cars she went off after a
        // treasure 204 m away, and the next car came (round 29 of its
        // thread).
        if let Some(g) = &game.combat.manned
            && g.rounds > 0
            && pick.is_some_and(|m| m.at.distance(g.pivot) > BLAST_GUN_REACH)
        {
            return None;
        }
        if let Some(m) = pick {
            // A vehicle to destroy is blown up, not its crew shot: the
            // crew aboard come into play under its guid (PmcCon003's
            // AMX30s: round 8 aimed at a tank's commander inside it,
            // with never a clear shot, for 1900 s).
            let a_vehicle = vehicles.iter().any(|v| v.guid == Some(m.target));
            if !a_vehicle
                && let Some(i) = npcs
                    .iter()
                    .position(|n| n.spawn == Some(m.target) && n.alive)
            {
                return Some((
                    Goal::Shoot(i),
                    format!("shooting {} ({:#x})", m.name, m.target),
                ));
            }
            let spawn = !a_vehicle
                && game
                    .scene
                    .world
                    .as_ref()
                    .is_some_and(|w| w.index.spawns.iter().any(|s| s.entity == m.target));
            if spawn {
                return Some((
                    Goal::Reach {
                        target: m.target,
                        at: m.at,
                    },
                    format!(
                        "heading for {} ({:#x}), {:.0} m off, nobody standing there yet",
                        m.name,
                        m.target,
                        m.at.distance(here)
                    ),
                ));
            }
            let at = vehicles
                .iter()
                .position(|v| v.guid == Some(m.target))
                .and_then(|i| vehicle_at(game, i))
                .map(|t| t.transform_point3(Vec3::Y))
                .or_else(|| game.scene.props.centre(m.target))
                .unwrap_or(m.at + Vec3::Y);
            let sturdy = game.scene.props.sturdy.contains(&m.target);
            // A satellite bomb is called on the ground: by the middle of a
            // tall building (ChiCon003's Target04, 80 m up) no way is found.
            let at = match game.scene.props.get(m.target) {
                Some((prop, _)) if sturdy => at.with_y(prop.position.y),
                _ => at,
            };
            let how = if sturdy {
                "calling a satellite bomb on"
            } else {
                "blowing up"
            };
            // Far off, on foot, with a ride close: driven there (round
            // 4's AllCon002 walked 600 m to the boats while Caracas fell).
            // Not within the scripts' bounds, where she shoots from where
            // she is (PmcCon034 round 9 of its thread: off the HQ's roof
            // for the Phoenix by it, statues 122 m off).
            // Nor from an emplaced gun that blows it up (031's flung cars).
            if driving.is_none()
                && self.keep.is_empty()
                && at.distance(here) > RIDE_FOR
                && game.combat.manned.is_none()
                && self.gun_by(game, here, m.target, at).is_none()
                && let Some(i) = self.ride_near(game, here, &live)
            {
                return Some((
                    Goal::Board(i),
                    format!(
                        "taking the {} for {} ({:.0} m off)",
                        vehicles[i].spec.label,
                        m.name,
                        at.distance(here)
                    ),
                ));
            }
            return Some((
                Goal::Blast {
                    target: m.target,
                    at,
                },
                format!(
                    "{how} {} ({:#x}) at {at:.1}, {:.0} m off",
                    m.name,
                    m.target,
                    at.distance(here)
                ),
            ));
        }
        // The scripts' vehicles they mark: in, or by where it will be.
        // (Not one marked to destroy: PmcCon031 round 16 got into the
        // car she was blowing up.)
        for m in live.iter().filter(|m| m.kind != "destroy") {
            let Some(i) = vehicles
                .iter()
                .position(|v| v.guid == Some(m.target) && !v.wrecked)
            else {
                continue;
            };
            if driving == Some(i) {
                continue;
            }
            // Driving, and it far off: driven up to first (OilCon002 round
            // 13 got out 250 m short of the armoured truck to hijack, and
            // its timer ran out with her on foot).
            let at = vehicle_at(game, i).map_or(m.at, |t| t.w_axis.truncate());
            if driving.is_some() && vehicles[i].id.is_some() && at.distance(here) > BOARD_DRIVE {
                return Some((
                    Goal::Reach {
                        target: m.target,
                        at,
                    },
                    format!(
                        "driving up to {} ({}), {:.0} m off",
                        m.name,
                        vehicles[i].spec.label,
                        at.distance(here)
                    ),
                ));
            }
            return Some(if vehicles[i].id.is_some() {
                (
                    Goal::Board(i),
                    format!("getting into {} ({})", m.name, vehicles[i].spec.label),
                )
            } else {
                (
                    Goal::Reach {
                        target: m.target,
                        at: m.at,
                    },
                    format!("waiting by {} for it to come", m.name),
                )
            });
        }
        // A vehicle on its way to her (hanging from a delivery's
        // helicopter): on foot she waits near where it is to be put down,
        // out from under it, and takes it once it is (the tank brought for
        // the last leg; a ride of hers would carry her past it).
        if driving.is_none()
            && let Some((load, drop)) = game
                .scene
                .flights
                .bringing()
                .into_iter()
                .filter(|(_, to)| to.distance(here) < AWAIT_REACH)
                .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)))
        {
            let at = drop + (here - drop).with_y(0.0).normalize_or(Vec3::X) * AWAIT_FROM;
            return Some((
                Goal::Reach { target: load, at },
                format!(
                    "waiting for the vehicle {load:#x} being brought to {drop:.0}, {:.0} m off",
                    drop.distance(here)
                ),
            ));
        }
        // A place to deliver to, with the scripts' delivery for it held
        // (OilCon002's listening posts, "OilCon002_Delivery", given her by
        // each location: its designator must land within 30 m of it, from
        // `MrxOilCon002Delivery`'s drop-zone check): called in there, and
        // kept at until the designator is thrown (round 4 called it in,
        // then went for a truck and put the smoke away).
        let calling = match &self.goal {
            Some(Goal::Deliver(k))
                if !k.starts_with("SoldierDelivery")
                    && (game.support.phase != Phase::Idle
                        || game.combat.to_throw
                        || self
                            .delivered
                            .is_some_and(|t| self.clock - t < DESIGNATOR_WAIT)) =>
            {
                Some(k.clone())
            }
            _ => None,
        };
        if driving.is_none()
            && let Some(key) = calling.clone().or_else(|| {
                game.support.entries().into_iter().find_map(|e| match e {
                    Entry::Stocked(s)
                        if s.key.ends_with("Delivery") && !s.key.starts_with("SoldierDelivery") =>
                    {
                        Some(s.key.clone())
                    }
                    _ => None,
                })
            })
            && let Some(m) = nearest(
                live.iter()
                    .copied()
                    .filter(|m| m.kind == "deliverable")
                    .collect(),
                here,
            )
            && m.at.distance(here)
                < if calling.is_some() {
                    DELIVER_KEEP
                } else {
                    DELIVER_NEAR
                }
            && (calling.is_some()
                || self
                    .delivered
                    .is_none_or(|t| self.clock - t > DELIVERY_AGAIN))
        {
            self.deliver_at.set(Some(m.at));
            return Some((
                Goal::Deliver(key.clone()),
                format!("calling in {key} at {} ({:#x})", m.name, m.target),
            ));
        }
        // Places to get to (not the vehicle she is in): those marked, else
        // the objectives' on the map or radar.
        let riding = driving.and_then(|i| vehicles.get(i)?.guid);
        let unmarked = |m: &Mark| m.kind == "objective" || m.kind == "radar";
        let marked = live
            .iter()
            .any(|m| !unmarked(m) && Some(m.target) != riding);
        let places: Vec<&Mark> = live
            .iter()
            .copied()
            // A thing to destroy left standing for now (under someone she
            // is to reach) is no place to go.
            .filter(|m| m.kind != "destroy")
            .filter(|m| Some(m.target) != riding && (!marked || !unmarked(m)))
            .collect();
        let m = nearest(places, here)?;
        if driving.is_none() {
            if m.at.distance(here) > RIDE_FOR
                && let Some(i) = self.ride_near(game, here, &live)
            {
                return Some((
                    Goal::Board(i),
                    format!(
                        "taking the {} for {} ({:.0} m off)",
                        vehicles[i].spec.label,
                        m.name,
                        m.at.distance(here)
                    ),
                ));
            }
            // There a while and nothing happening: the scripts' vehicle
            // standing by is hers. Not with someone following her to a place
            // to deliver them (OilCon001 round 23: in and out of the EXT 8 m
            // from the drop-off 3400 s, the executive wanted within 6 m).
            if let Some((_, since)) = self.waiting.filter(|w| w.0 == m.target)
                && !(m.kind == "deliverable" && !followers.is_empty())
                && self.clock - since > TAKE_AFTER
                && let Some(i) = vehicles
                    .iter()
                    .enumerate()
                    // Not one she brought goods there in: that is the
                    // delivery, done by leaving it (PirCon002 round 46).
                    .filter(|&(i, v)| {
                        v.guid.is_some()
                            && !v.wrecked
                            && !v.spec.is_boat()
                            && !game.scene.props.carrying(i)
                    })
                    .filter_map(|(i, _)| {
                        let d = vehicle_at(game, i)?.w_axis.truncate().distance(m.at);
                        (d < TAKE_NEAR).then_some((i, d))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            {
                return Some((
                    Goal::Board(i),
                    format!(
                        "nothing at {}: getting into the {} standing by",
                        m.name, vehicles[i].spec.label
                    ),
                ));
            }
        }
        // Driven to a place to deliver it and nothing comes of it: the
        // scripts want her out of it there (`MrxTaskObjectiveDeliver`'s
        // `bStop` delivery of a vehicle shows "[objective.Deliver.
        // vehicleExit]" while she sits in it: PirCon002's rum truck). Only
        // once what she brings is inside its destination, by the
        // delivery's own region or radius ([`Delivery::arrived`]); short of
        // it she drives on in (story run b3i: stopped 15 m from OilCon020's
        // gun drop, its radius 15 m, she got out and back in 1,939 times).
        if driving.is_some()
            && m.kind == "deliverable"
            && self
                .waiting
                .is_some_and(|(g, since)| g == m.target && self.clock - since > EXIT_AFTER)
        {
            match self.deliveries.iter().find(|d| d.dest == m.target) {
                Some(d) if !d.arrived() => {
                    return Some((
                        Goal::Reach {
                            target: m.target,
                            at: d.at,
                        },
                        format!(
                            "delivery: what she brings is {} from {}, outside it: driving on in",
                            d.describe(),
                            m.name
                        ),
                    ));
                }
                found => {
                    let inside = found.map_or_else(
                        || "no delivery read".to_owned(),
                        |d| format!("{} off, inside", d.describe()),
                    );
                    return Some((
                        Goal::GetOut,
                        format!(
                            "getting out at {}, to deliver what she drove there ({inside})",
                            m.name
                        ),
                    ));
                }
            }
        }
        // An outpost's flag is taken standing by it: the scripts count
        // those who get there as its rushers (inferred: the outpost's
        // rusher check asks whether each is in a seat).
        if driving.is_some() && m.kind == "outpost" && m.at.distance(here) < FLAG_ON_FOOT {
            return Some((
                Goal::GetOut,
                format!("getting out by {}, to take it on foot", m.name),
            ));
        }
        Some((
            Goal::Reach {
                target: m.target,
                at: m.at,
            },
            format!(
                "going to {} ({:#x}, {}), {:.0} m off",
                m.name,
                m.target,
                m.kind,
                m.at.distance(here)
            ),
        ))
    }

    /// A ride close to `here`, for somewhere far: the scripts' vehicle
    /// they brought last that she hasn't driven (the tank delivered for
    /// the next leg), else hers from before or another of theirs.
    fn ride_near(&self, game: &Game, here: Vec3, live: &[&Mark]) -> Option<usize> {
        let vehicles = &game.scene.vehicles;
        let near = |i: usize, within: f32| {
            vehicles
                .get(i)
                .is_some_and(|v| !v.wrecked && !v.spec.is_boat() && !v.spec.is_helicopter())
                && !self.left_rides.contains(&i)
                // Not one she is to destroy (PmcCon031's flung cars: she
                // drove off in its targets, round 16).
                && !vehicles[i].guid.is_some_and(|g| {
                    live.iter().any(|m| m.target == g && m.kind == "destroy")
                })
                && vehicle_at(game, i).is_some_and(|t| t.w_axis.truncate().distance(here) < within)
        };
        // The one she set out for while it is still close (round 6 turned
        // from vehicle to vehicle of the Chinese camp as she passed them).
        if let Some(Goal::Board(i)) = self.goal
            && near(i, RIDE_FRESH)
        {
            return Some(i);
        }
        (0..vehicles.len())
            .rev()
            // Not one with its own crew aboard (the enemy's).
            .filter(|&i| {
                vehicles[i].guid.is_some()
                    && vehicles[i].crew.is_empty()
                    && !self.ridden.contains(&i)
            })
            .find(|&i| near(i, RIDE_FRESH))
            .or_else(|| {
                self.ride
                    .into_iter()
                    .chain((0..vehicles.len()).filter(|&i| vehicles[i].guid.is_some()))
                    .find(|&i| near(i, RIDE_NEAR))
            })
    }

    /// Opening supply crate `target`: to its side, turned to it, and a
    /// bash (low, at a crate on the ground); left shut if not open after
    /// [`OPEN_FOR`] seconds.
    fn open(&mut self, game: &mut Game, target: u32, dt: f32) -> Intent {
        let since = match self.opening {
            Some((g, t)) if g == target => t,
            _ => {
                self.opening = Some((target, self.clock));
                self.clock
            }
        };
        let (Some(p), Some((at, half))) = (
            game.scene.player.as_ref(),
            game.scene.flights.crate_standing(target),
        ) else {
            return Intent::default();
        };
        if self.clock - since > OPEN_FOR {
            eprintln!(
                "autoplay {:.1} s: the supply crate {target:#x} at {at:.1} would not open: left shut",
                self.clock
            );
            self.left_shut.push(target);
            return Intent::default();
        }
        let here = p.character.position;
        let to = (at - here).with_y(0.0);
        if to.length() > half + OPEN_FROM {
            return self.walk(game, at, half + OPEN_FROM, dt);
        }
        // Standing, she turns only to aim: so she aims round to it, and
        // bashes once she faces it.
        let yaw = to.x.atan2(to.z);
        let facing = wrap(yaw - p.facing).abs() < OPEN_FACING;
        Intent {
            look: turn_to(p, yaw, CRUISE_PITCH, dt),
            trigger: Trigger {
                aim: true,
                melee: facing,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Taking what a supply crate left (`key`, [`Pickup::key`]): walking
    /// onto it and, within its reach, pressing the use key; left lying if
    /// not taken after [`OPEN_FOR`] seconds.
    ///
    /// [`Pickup::key`]: crate::flights::Pickup::key
    fn take(&mut self, game: &mut Game, key: (u32, u32), dt: f32) -> Intent {
        let since = match self.taking {
            Some((k, t)) if k == key => t,
            _ => {
                self.taking = Some((key, self.clock));
                self.clock
            }
        };
        let Some(p) = game
            .scene
            .flights
            .pickups()
            .iter()
            .find(|p| p.key() == key)
            .copied()
        else {
            return Intent::default();
        };
        if self.clock - since > OPEN_FOR {
            eprintln!(
                "autoplay {:.1} s: the {:#x} the supply crate {:#x} left at {:.1} was not taken: left lying",
                self.clock, key.1, key.0, p.at
            );
            self.left_lying.push(key);
            return Intent::default();
        }
        let here = game
            .scene
            .player
            .as_ref()
            .map_or(p.at, |pl| pl.character.position);
        let close = (p.at - here).with_y(0.0).length();
        // Over it a while and it never goes (round 16's AllCon002: 45 s
        // stood 1.2 m from one while the boats went by): left lying.
        if close <= p.what.reach && self.clock - since > TAKE_AT {
            eprintln!(
                "autoplay {:.1} s: the {:#x} at {:.1} doesn't come with the key: left lying",
                self.clock, key.1, p.at
            );
            self.left_lying.push(key);
            return Intent::default();
        }
        if close <= p.what.reach * 0.8 {
            let press = self.pressed > PRESS_GAP;
            if press {
                self.pressed = 0.0;
            }
            // A rifle lying there is taken for its rounds ("Take Ammo"),
            // not taken up in place of hers ([`wanted`]).
            let templates = game.scene.world.as_ref().map(|w| &w.index.template_db);
            let equips = |q: &crate::flights::Pickup| {
                q.what.weapon == Some(Weapon::Rifle)
                    && templates.is_none_or(|t| {
                        crate::mission::offer(t, &game.combat, &q.what)
                            == crate::mission::Offer::Equip
                    })
            };
            let swap = equips(&p);
            // A gun lying within reach beside it would be taken up with
            // the key instead, in place of hers (PmcCon001's villa: a
            // dead soldier's grenade and rifle together, and she swapped
            // rifles back and forth for minutes): left lying.
            let beside = game.scene.flights.pickups().iter().any(|q| {
                q.key() != key && (q.at - here).with_y(0.0).length() <= q.what.reach && equips(q)
            });
            if beside && !swap {
                eprintln!(
                    "autoplay {:.1} s: the {:#x} at {:.1} lies by a gun the key would take up                      in place of hers: left lying",
                    self.clock, key.1, p.at
                );
                self.left_lying.push(key);
                return Intent::default();
            }
            return Intent {
                use_pressed: press && !swap,
                ..Intent::default()
            };
        }
        self.walk(game, p.at, p.what.reach * 0.5, dt)
    }

    /// Nothing to do: stand, or stop the vehicle.
    fn idle(&mut self, game: &mut Game, dt: f32) -> Intent {
        let driving = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_some());
        // Nothing marked, and the scripts wait for her to look at
        // something near: she looks at it, from a few metres off.
        if !driving
            && let Some(p) = game.scene.player.as_ref()
            && let Some(at) = self
                .sights
                .iter()
                .copied()
                .filter(|s| s.distance(p.character.position) < LOOK_NEAR)
                .min_by(|a, b| {
                    let here = p.character.position;
                    a.distance(here).total_cmp(&b.distance(here))
                })
        {
            let here = p.character.position;
            if (at - here).with_y(0.0).length() < LOOK_FROM {
                let away = here + (here - at).with_y(0.0).normalize_or(Vec3::X) * LOOK_FROM;
                return self.walk(game, away, 1.0, dt);
            }
            let (look, _) = aim(p, at, dt);
            return Intent {
                look,
                ..Intent::default()
            };
        }
        Intent {
            hold_jump: driving,
            ..Intent::default()
        }
    }

    /// Gets to `at`, on foot or in the vehicle she is in.
    fn travel(&mut self, game: &mut Game, at: Vec3, arrive: f32, dt: f32) -> Intent {
        match game.scene.player.as_ref().and_then(|p| p.driving) {
            Some(i) => self.drive(game, i, at, arrive, dt),
            None => self.walk(game, at, arrive, dt),
        }
    }

    /// On foot to within `arrive` of `at` (then round it, as said at
    /// [`CIRCLE_AFTER`]).
    fn walk(&mut self, game: &mut Game, at: Vec3, arrive: f32, dt: f32) -> Intent {
        let Some(here) = game.scene.player.as_ref().map(|p| p.character.position) else {
            return Intent::default();
        };
        let d = (at - here).with_y(0.0).length();
        let mut target = self.waypoint(game, None, here, at, RUN);
        if self.headway(game, at, self.left_to_go(here, at)) {
            return Intent::default();
        }
        // Walking all the while and no nearer it as the crow flies, her
        // ways replanned round and round (PmcCon031 round 25: 200 s in the
        // MG range's yard, 95 m from the car range's point): hopped on.
        let wander = match self.wander {
            Some((was, best, since)) if was.distance(at) < WANDER_SAME => {
                if d < best - WANDER_NEARER {
                    (at, d, self.clock)
                } else {
                    (was, best, since)
                }
            }
            _ => (at, d, self.clock),
        };
        self.wander = Some(wander);
        if d >= arrive.max(ARRIVE) * 2.0 && self.clock - wander.2 > WANDER {
            let to = self
                .way
                .as_mut()
                .map(|w| w.route.steer(here, HOP).0)
                .filter(|to| (*to - here).with_y(0.0).length() >= HOP_LEAST)
                .unwrap_or(at);
            eprintln!(
                "autoplay {:.1} s: no nearer {at:.0} in {WANDER:.0} s on foot ({d:.0} m off)",
                self.clock
            );
            self.wander = None;
            self.hop(game, to, WANDER);
            return Intent::default();
        }
        if d < arrive || self.arrived.is_some() {
            let since = *self.arrived.get_or_insert(self.clock);
            if d < arrive && at.y - here.y > UNDER_BY && self.clock - since > UNDER_FOR {
                self.arrived = None;
                self.hop(game, at, self.clock - since);
                return Intent::default();
            }
            if self.clock - since < CIRCLE_AFTER {
                return Intent::default();
            }
            let a = (self.clock - since) * 0.6;
            target = at + Vec3::new(a.sin(), 0.0, a.cos()) * CIRCLE;
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let to = (target - here).with_y(0.0);
        let look = turn_to(p, to.x.atan2(to.z), CRUISE_PITCH, dt);
        let mut movement = Vec2::Y;
        let mut jump = false;
        if let Some((side, left)) = &mut self.sidestep {
            jump = *left == SIDESTEP;
            *left -= dt;
            movement = Vec2::new(*side, 0.5).normalize();
            if *left <= 0.0 {
                self.sidestep = None;
            }
        }
        Intent {
            movement,
            sprint: d > 12.0,
            walk: d < 3.0,
            jump,
            hold_jump: jump,
            look,
            ..Intent::default()
        }
    }

    /// Driving vehicle `i` to within `arrive` of `at`.
    fn drive(&mut self, game: &mut Game, i: usize, at: Vec3, arrive: f32, dt: f32) -> Intent {
        let Some((m, speed)) = vehicle_state(game, i) else {
            return Intent::default();
        };
        if self.leave {
            return self.press(speed);
        }
        let pos = m.w_axis.truncate();
        let d = (at - pos).with_y(0.0).length();
        let toward = self.waypoint(game, Some(i), pos, at, speed);
        if self.headway(game, at, self.left_to_go(pos, at)) {
            return Intent::default();
        }
        if d < arrive.max(6.0) {
            self.arrived.get_or_insert(self.clock);
            return Intent {
                hold_jump: speed.abs() > 0.3,
                ..Intent::default()
            };
        }
        let (angle, _) = heading_to(m, toward);
        // Round what stands in the way.
        let angle = match (
            game.scene.world.as_ref(),
            game.scene
                .vehicles
                .get(i)
                .and_then(|v| v.id.zip(Some(v.spec.hull))),
        ) {
            (Some(w), Some((id, hull))) => clear_way(&w.physics, id, m, hull, angle, speed),
            _ => angle,
        };
        // Another vehicle coming the other way in her lane: she keeps to
        // the right of it and eases off, as a driver would (PirCon002's
        // laden truck met a car head-on at 22 m/s closing, round 58).
        let oncoming = oncoming(game, i, m, speed);
        let angle = if oncoming {
            angle.max(0.0) + SWERVE
        } else {
            angle
        };
        // And round a vehicle standing in her way, to the side it isn't
        // on (PirCon004 round 78: her truck ran into a parked pickup and
        // a trailer by the drop-off, close by the way planned round them).
        let standing = standing_ahead(game, i, m, speed, toward);
        // In a boat: the nearest other vehicle, where in her frame and how
        // fast, each second (what she gives way to, or doesn't).
        if game.scene.vehicles[i].spec.is_boat() && self.clock - self.near_logged > 1.0 {
            self.near_logged = self.clock;
            let to_local = m.inverse();
            if let Some((j, l, s)) = (0..game.scene.vehicles.len())
                .filter(|&j| j != i)
                .filter_map(|j| {
                    let (t, s) = vehicle_state(game, j)?;
                    Some((j, to_local.transform_point3(t.w_axis.truncate()), s))
                })
                .min_by(|a, b| a.1.length().total_cmp(&b.1.length()))
                .filter(|(_, l, _)| l.length() < 200.0)
            {
                eprintln!(
                    "autoplay {:.1} s: boat at {pos:.0}, {speed:.1} m/s, heading for {toward:.0} ({:+.0} deg off); nearest the {} at {l:.1} (her frame), {s:.1} m/s; standing ahead {standing:?}",
                    self.clock,
                    angle.to_degrees(),
                    game.scene.vehicles[j].spec.label
                );
            }
        }
        // A boat, with no brakes, turns away harder and backs its engine
        // (GurCon003 round 84: her race boat met a moored fishing boat by
        // a gate at 31 m/s, easing off).
        let boat = game.scene.vehicles[i].spec.is_boat();
        let swerve = if boat { 2.0 * SWERVE } else { SWERVE };
        let angle = match standing {
            Some(x) => angle + if x > 0.0 { swerve } else { -swerve },
            None => angle,
        };
        let mut steer = (angle * 2.0).clamp(-1.0, 1.0);
        let mut throttle: f32 = if boat && standing.is_some() && speed > SLOW {
            -1.0
        } else if oncoming || standing.is_some() {
            0.3
        } else {
            1.0
        };
        if (d < SLOW_NEAR && speed > SLOW) || (angle.abs() > 0.8 && speed > SLOW) {
            throttle = throttle.min(0.2);
        }
        // Goods riding on her truck (PirCon002's rum): no faster than a
        // careful driver goes, so a crash costs less of them.
        // So too in a frail vehicle (OilCon021's Devastator, a 50-health
        // scooter: round 20 ran it into an EXT at 14 m/s and lost it).
        // And with someone riding along who doesn't man a gun (OilCon002's
        // hostage: round 3 ran the 100-health bank truck into a tank at
        // 20 m/s, 240 damage, and the blast killed him).
        let frail = game.scene.vehicles[i].most_health() <= FRAIL
            || game.scene.people.aboard().len() > game.scene.people.gunners();
        let laden = game.scene.props.carrying(i) || frail;
        if laden && speed > if frail { FRAIL_SPEED } else { LADEN_SPEED } {
            throttle = 0.0;
        }
        // And she gives way to a vehicle on course to cross ahead of her
        // (PirCon004 round 72: a car crossing her road at speed wrecked
        // the organ truck, 35.9 m/s closing); in a boat too (GurCon003
        // round 81: a pleasure boat wrecked her race boat, 34.8 m/s).
        if (laden || boat)
            && let Some(j) = crossing(game, i, m)
        {
            throttle = if speed > 1.0 { -1.0 } else { 0.0 };
            if self.clock - self.near_logged > 1.0 {
                self.near_logged = self.clock;
                eprintln!(
                    "autoplay {:.1} s: giving way to the {} crossing ahead",
                    self.clock, game.scene.vehicles[j].spec.label
                );
            }
        }
        // Stuck against something: back out, turning towards the way on.
        if throttle > 0.5 && speed.abs() < 1.0 && self.reversing.is_none() {
            self.stalled += dt;
        } else {
            self.stalled = 0.0;
        }
        if self.stalled > STALL {
            self.stalled = 0.0;
            // Each stall logged, with what holds it (round 7 counts the
            // tank's over its jungle drive).
            if let (Some(w), Some(v)) = (game.scene.world.as_ref(), game.scene.vehicles.get(i))
                && let Some(id) = v.id
            {
                let way = self.way.as_ref().map_or_else(
                    || "none".to_owned(),
                    |w| {
                        let n = w.route.points.len();
                        format!(
                            "{n} points over {:.0} m, {}",
                            w.route.length(),
                            w.route
                                .points
                                .iter()
                                .take(4)
                                .map(|p| format!("{p:.0}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    },
                );
                eprintln!(
                    "autoplay {:.1} s: stalled driving the {} at {:.1}: {}; her way: {way}",
                    self.clock,
                    v.spec.label,
                    game.focus(),
                    stuck_report(w, id, v.spec.hull, false)
                );
                // What holds her is kept out of from here on (PirCon003
                // rounds 61-62: the way out of the parrots' base ran over
                // a bank its truck caught on, again each time planned).
                let kept = self.avoid.len();
                for (key, _, (lo, hi)) in w.physics.vehicle_touching(id) {
                    let (lo, hi) = (
                        Vec2::new(lo.x, lo.z) - Vec2::splat(AVOID_ROOM),
                        Vec2::new(hi.x, hi.z) + Vec2::splat(AVOID_ROOM),
                    );
                    if matches!(key, Some(k) if !matches!(k, StaticKey::Cell(_)))
                        && (hi - lo).max_element() < AVOID_LARGEST
                        && !self.avoid.contains(&(lo, hi))
                    {
                        eprintln!(
                            "autoplay {:.1} s: keeping out of {} ({lo:.0} to {hi:.0}) from now on",
                            self.clock,
                            crate::static_name(w, key)
                        );
                        self.avoid.push((lo, hi));
                        self.way = None;
                    }
                }
                // Held by the ground alone (a bank too steep: PirCon004
                // rounds 71 and 79 stalled at one by a checkpoint, and a
                // grenade followed): the ground just ahead is kept out of,
                // unless that is where she is going. Not a boat's (the
                // races' shallows are in the way only for a moment).
                let ahead = m.transform_point3(Vec3::Z * TERRAIN_AHEAD);
                let (lo, hi) = (
                    Vec2::new(ahead.x, ahead.z) - Vec2::splat(TERRAIN_HALF),
                    Vec2::new(ahead.x, ahead.z) + Vec2::splat(TERRAIN_HALF),
                );
                let goal = Vec2::new(at.x, at.z);
                if self.avoid.len() == kept
                    && !v.spec.is_boat()
                    && !(goal.cmpge(lo).all() && goal.cmple(hi).all())
                {
                    eprintln!(
                        "autoplay {:.1} s: keeping out of the ground ahead ({lo:.0} to {hi:.0}) from now on",
                        self.clock
                    );
                    self.avoid.push((lo, hi));
                    self.way = None;
                }
            }
            // A boat run aground near the place: out, into the shallows
            // (further off, it backs off the rocks). Not when the place
            // lies on open water (a race's checkpoint: she would lose
            // the race leaving her jetski, PirCon001 round 24).
            let place_afloat = game.scene.world.as_ref().is_some_and(|w| {
                let v = &game.scene.vehicles[i];
                v.id.is_some_and(|id| {
                    afloat_near(&w.physics, &v.spec, id, at - Vec3::X * 0.5, at, 0.0).is_some()
                })
            });
            if d < RIDE_FOR && !place_afloat && self.can_leave(game, i) {
                eprintln!(
                    "autoplay {:.1} s: the {} is aground {d:.0} m off: getting out",
                    self.clock, game.scene.vehicles[i].spec.label
                );
                self.leave = true;
                return self.press(speed);
            }
            self.reversing = Some((-angle.signum(), REVERSE));
            // And the way is planned again from where she backs out to:
            // the old one led her onto what holds her (PirCon001's jetski
            // cut a rock's corner each time round).
            self.way = None;
        }
        if let Some((s, left)) = &mut self.reversing {
            steer = *s;
            throttle = -1.0;
            *left -= dt;
            if *left <= 0.0 {
                self.reversing = None;
            }
        }
        Intent {
            movement: Vec2::new(steer, throttle),
            ..Intent::default()
        }
    }

    /// Shooting person `npcs[i]`: closer first if out of range; from a
    /// tank, with its cannon.
    fn shoot(&mut self, game: &mut Game, i: usize, dt: f32) -> Intent {
        let Some(target) = game.scene.people.npcs.get(i).map(|n| n.chest()) else {
            return Intent::default();
        };
        if let Some(v) = game.scene.player.as_ref().and_then(|p| p.driving) {
            // A story character (AllCon003's General Peng): out, and on foot
            // with the rifle once near (round 7's APC sat 79 m off his
            // bunker 1500 s, its gun never reaching him).
            let held = game.scene.people.npcs.get(i).is_some_and(|n| n.held);
            if held
                && !self.leave
                && target.distance(game.focus()) < SHOOT_RANGE * 2.0
                && self.may_get_out(game, v)
            {
                eprintln!(
                    "autoplay {:.1} s: {} near: getting out to go on foot",
                    self.clock, self.what
                );
                self.leave = true;
            }
            if self.leave {
                let speed = vehicle_state(game, v).map_or(0.0, |(_, s)| s);
                return self.press(speed);
            }
            return self.at_target_driving(game, v, target, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        // Fighting beside a truck with goods on it draws the soldiers'
        // grenades onto them (PirCon003 round 60: one grenade broke all
        // 42 parrot boxes): away from it first.
        let here = p.character.position;
        if let Some(c) = cargo_near(game, here) {
            let away = (here - c).with_y(0.0).normalize_or(Vec3::X);
            return self.walk(game, c + away * (CARGO_CLEAR + 3.0), 1.0, dt);
        }
        let d = target.distance(here);
        if d > SHOOT_RANGE {
            return self.walk(game, target, SHOOT_RANGE - 5.0, dt);
        }
        let (look, error) = aim(p, target, dt);
        let clear = game.scene.world.as_ref().is_some_and(|w| {
            Combat::aim_probe(&w.physics, &game.scene.people, p)
                .is_some_and(|(_, who)| who == Some(i))
        });
        self.steady = if error < ON_TARGET {
            self.steady + dt
        } else {
            0.0
        };
        // No clear shot counts whether or not her aim has settled: one she
        // can't aim at never settles (OilCon002 round 1: 1700 s, 5 m from
        // three soldiers a floor below, no round fired).
        if clear {
            self.blocked = 0.0;
            self.vantage = None;
            self.tried.clear();
        } else {
            self.blocked += dt;
        }
        // Aimed a while with a wall in the way: from where she can see
        // the target; another spot if that one was no better.
        if self.blocked > BLOCKED {
            let at_spot = self
                .vantage
                .is_some_and(|(t, s)| t == i && s.with_y(0.0).distance(here.with_y(0.0)) < ARRIVE);
            if self.vantage.is_none_or(|(t, _)| t != i) || at_spot {
                if let Some((_, s)) = self.vantage {
                    self.tried.push(s);
                }
                self.blocked = 0.0;
                self.fresh_headway();
                self.vantage = game
                    .scene
                    .world
                    .as_ref()
                    .and_then(|w| {
                        vantage(&w.physics, target, here, &self.tried, &VANTAGE_RINGS, 1.0)
                    })
                    .map(|s| (i, s));
                // Fighting back one she can't see from anywhere near, she
                // leaves them be a while and goes on.
                if self.vantage.is_none() && self.what.starts_with("fighting back") {
                    self.unseen.retain(|&(_, until)| until > self.clock);
                    self.unseen.push((target, self.clock + LEAVE_UNSEEN));
                }
                eprintln!(
                    "autoplay {:.1} s: no clear shot at {} from {here:.1}: {}",
                    self.clock,
                    self.what,
                    self.vantage
                        .map_or("nowhere better to see it from".to_owned(), |(_, s)| {
                            format!("going to {s:.1}")
                        })
                );
            }
        }
        if let Some((t, spot)) = self.vantage
            && t == i
            && spot.with_y(0.0).distance(here.with_y(0.0)) >= ARRIVE
        {
            self.arrived = None;
            return self.walk(game, spot, 1.0, dt);
        }
        // Aimed, but something is in the way: closer.
        let movement = if !clear && self.steady > 1.0 && self.vantage.is_none() {
            Vec2::Y
        } else {
            Vec2::ZERO
        };
        Intent {
            look,
            movement,
            walk: true,
            trigger: Trigger {
                aim: true,
                fire: clear && self.steady > STEADY,
                select: (game.combat.weapon != Weapon::Rifle).then_some(Weapon::Rifle),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Where the place she is going to is, for a goal that is one.
    fn goal_at(&self) -> Option<Vec3> {
        match &self.goal {
            Some(Goal::Reach { at, .. } | Goal::Blast { at, .. } | Goal::Use { at, .. }) => {
                Some(*at)
            }
            _ => None,
        }
    }

    /// Whether she can fire her rifle: on foot, her weapons out, no gun
    /// manned.
    fn rifle_ready(game: &Game) -> bool {
        game.scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_none() && !p.weapons_away())
            && game.combat.manned.is_none()
    }

    /// Shooting what stands at `at` with her rifle, from within
    /// [`SHOOT_RANGE`], once aimed a moment.
    fn rifle_at(&mut self, game: &mut Game, target: u32, at: Vec3, dt: f32) -> Intent {
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        // What rounds break is shot at from further: from her sniper
        // tower, which she may not leave (PmcCon034 round 8: she walked
        // off it for a statue 98 m off, and left the contract's bounds).
        // So too a vehicle to destroy (PmcCon034's Easy16-19 "statues" are
        // L300 racing cars: she walked off the HQ's roof for one 97 m off,
        // round 7 of its thread).
        let a_vehicle = game
            .scene
            .vehicles
            .iter()
            .any(|v| v.guid == Some(target) && !v.wrecked);
        // Within the scripts' bounds, anything from where she stands.
        let range = if takes_rounds(game, target) || a_vehicle || !self.keep.is_empty() {
            SNIPE_RANGE
        } else {
            SHOOT_RANGE
        };
        // Within bounds, what is out of her reach, or has stood through
        // her rifle, is passed over for now rather than walked to
        // (PmcCon034 round 8 of its thread: off the HQ's roof for a bonus
        // statue 64 m off, she fell 15 m and the scripts took her weapons).
        let tried_out = self
            .rifle_on
            .is_some_and(|(t, spent)| t == target && spent >= self.rifle_try(game, t));
        // Nor what her rounds don't wear (the bonus statues: each stood 20 s
        // of them, round 13), with the contract's clock running (90 s for
        // all 19 the first time, its `nTimeLimit`).
        let unworn = !takes_rounds(game, target) && !a_vehicle;
        if !self.keep.is_empty() && (at.distance(here) > range || tried_out || unworn) {
            let again = self
                .passed
                .iter()
                .any(|&(t, until)| t == target && self.clock < until);
            if again {
                return Intent::default();
            }
            eprintln!(
                "autoplay {:.1} s: passing over {target:#x} for now ({:.0} m off, {})",
                self.clock,
                at.distance(here),
                if tried_out {
                    "stood through her rifle"
                } else if unworn {
                    "her rounds don't wear it"
                } else {
                    "out of her reach"
                }
            );
            self.passed.push((target, self.clock + KEEP_PASS));
            self.rifle_on = None;
            return Intent::default();
        }
        if at.distance(here) > range {
            return self.walk(game, at, range - 5.0, dt);
        }
        // What of it she can see (its middle hidden, a side of it).
        let at = visible_part(game, target, at, Combat::aim_ray(p).0).unwrap_or(at);
        let (look, error) = aim(p, at, dt);
        // Far off, the crosshair is held on it closer: 0.03 rad is 3 m at
        // 98 m, wider than a statue (PmcCon034 round 21: 20 anti-material
        // rounds, none broke one). The width is invented.
        let on = ON_TARGET.min(SNIPE_WIDTH / at.distance(p.eye()).max(1.0));
        self.steady = if error < on { self.steady + dt } else { 0.0 };
        // Aimed, a round would strike something of the world's well short
        // of it (PmcCon034 round 1 of its thread: all 24 anti-material
        // rounds on the HQ's roof she stands on, 7 m off, the statue 98 m
        // off and 35 m below): she goes to a spot on her own level she can
        // see it from ([`perch`]), within the scripts' bounds; no round
        // fired, nor her time with it counted, on the way. Judged once
        // aimed.
        if self.rifle_on.is_none_or(|(t, _)| t != target) {
            self.perch = None;
            self.perch_tried.clear();
            self.blocked_logged = false;
        }
        if self.trust_sight.is_some_and(|t| t != target) {
            self.trust_sight = None;
        }
        if self.steady > STEADY && self.perch.is_none() && self.trust_sight != Some(target) {
            let was = self.blocked_logged;
            self.blocked_logged = game.scene.world.as_ref().is_some_and(|w| {
                Combat::aim_probe(&w.physics, &game.scene.people, p).is_some_and(|(t, who)| {
                    // From where her shot starts (in front of the camera).
                    who.is_none() && t < at.distance(Combat::aim_ray(p).0) - SHOT_SHORT
                })
            });
            if self.blocked_logged && !was {
                // A few spots tried, none better: no more of them (PmcCon031
                // round 27: a new spot each 0.4 s for 10 s on the MG
                // platform, each in sight of it by a ray and not by her aim).
                let spot = game
                    .scene
                    .world
                    .as_ref()
                    .filter(|_| self.perch_tried.len() < PERCH_TRIES)
                    .and_then(|w| perch(&w.physics, at, here, &self.perch_tried, &self.keep));
                eprintln!(
                    "autoplay {:.1} s: no clear shot at {target:#x} from {here:.1}: {}",
                    self.clock,
                    spot.map_or(
                        "nowhere on her level to see it from".to_owned(),
                        |s| format!("going to {s:.1}")
                    )
                );
                self.perch_tried.push(here);
                if let Some(s) = spot {
                    self.perch = Some((s, self.clock));
                    self.blocked_logged = false;
                } else {
                    // Nowhere (within bounds) to see it from: the others
                    // first. (PmcCon031's MG platform: its rail hid one
                    // column, and she stood at it 20 s, round 25.)
                    self.passed.push((target, self.clock + KEEP_PASS));
                    self.rifle_on = None;
                    return Intent::default();
                }
            }
        }
        if let Some((spot, since)) = self.perch {
            // Or where she sees it from on the way (round 13: 20 s walking
            // to a spot 2.6 m off, the statue in sight 1 m on).
            let eye = Combat::aim_ray(p).0;
            let seen = game.scene.world.as_ref().is_some_and(|w| {
                let (dir, len) = ((at - eye).normalize_or_zero(), at.distance(eye));
                w.physics
                    .cast_ray(eye, dir, (len - SHOT_SHORT).max(0.0))
                    .is_none()
            });
            let there = seen || spot.with_y(0.0).distance(here.with_y(0.0)) < PERCH_ARRIVE;
            if !there && self.clock - since < SPOT_GIVE_UP {
                // Not `arrived` where she stood (walk would keep her there).
                self.arrived = None;
                return self.walk(game, spot, PERCH_ARRIVE * 0.5, dt);
            }
            // There (or no nearer after a while): aimed again, she judges
            // the shot from here.
            if !there {
                self.perch_tried.push(spot);
            }
            // In sight from where her aim was blocked, without a step: her
            // eye's line is taken over her aim's, and she fires (PmcCon031
            // round 34: 122 spots on the MG platform in 150 s, each in
            // sight at once; the columns fall to her rounds from there).
            if seen
                && self
                    .perch_tried
                    .last()
                    .is_some_and(|t| t.distance(here) < PERCH_ARRIVE)
            {
                self.trust_sight = Some(target);
            }
            self.perch = None;
            self.steady = 0.0;
        }
        // Only her time with a clear shot counts toward giving it up.
        let short = self.blocked_logged;
        match &mut self.rifle_on {
            Some((t, spent)) if *t == target => {
                if !short {
                    *spent += dt;
                }
            }
            _ => {
                // The last she shot at broke: her rounds break its kind.
                if let Some((t, _)) = self.rifle_on
                    && let Some(kind) = self.rifle_kind
                    && (game.scene.props.broke_apart(t) || game.scene.props.get(t).is_none())
                {
                    self.rifle_broke.insert(kind);
                }
                self.rifle_kind = game.scene.props.get(target).map(|(p, _)| p.template);
                eprintln!(
                    "autoplay {:.1} s: shooting {target:#x} with her rifle",
                    self.clock
                );
                self.rifle_on = Some((target, 0.0));
            }
        }
        Intent {
            look,
            walk: true,
            trigger: Trigger {
                aim: true,
                // In short bursts, her spread settling between them (the
                // weapons' bloom): held down, 240 rounds broke 8 of
                // PmcCon032's 21 statues and her clips ran dry (round 13).
                // (So too PmcCon031's columns held down near, or in bursts
                // twice as often: her 240 went on about 42 of its 50, rounds
                // 33 and 36 of its thread.)
                fire: !short && self.steady > STEADY && self.clock.rem_euclid(TAP_EVERY) < TAP_FOR,
                select: (game.combat.weapon != Weapon::Rifle).then_some(Weapon::Rifle),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Firing the emplaced gun she mans on what stands at `at`, once
    /// aimed a moment.
    fn fire_manned(&mut self, game: &mut Game, target: u32, at: Vec3, dt: f32) -> Intent {
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        // A round that blows up fired on it: the others meanwhile, while
        // it flies (its flight taken as half again its straight-line time,
        // invented). Held on one at a time, the grenade launcher's 50 went
        // on 45 of PmcCon031's columns and 5 stood (round 27 of its
        // thread); waiting on each, 6 fell in 25 s (round 29).
        if let Some(m) = &game.combat.manned {
            if self.manned_rounds.is_some_and(|r| m.rounds < r) && m.weapon.ammo.blast.is_some() {
                let flight = at.distance(m.pivot) / m.weapon.ammo.velocity.max(1.0) * 1.5;
                self.passed.push((target, self.clock + flight + LANDS));
            }
            self.manned_rounds = Some(m.rounds);
        }
        // Not into what stands right by the gun: a round that blows up
        // there takes the gun with it (PmcCon031 round 32: a palm by its
        // grenade launcher, and both its launchers went, which ends the
        // contract). Another target meanwhile.
        let muzzle_clear = game.combat.manned.as_ref().is_none_or(|m| {
            m.weapon.ammo.blast.is_none()
                || game.scene.world.as_ref().is_none_or(|w| {
                    let dir = launch_dir(m, at);
                    w.physics
                        .cast_ray(m.pivot + dir * 0.9, dir, MUZZLE_CLEAR)
                        .is_none()
                })
        });
        if !muzzle_clear && self.steady > STEADY {
            eprintln!(
                "autoplay {:.1} s: something by the gun is in the way of {target:#x}: another \
                 for {KEEP_PASS:.0} s",
                self.clock
            );
            self.passed.push((target, self.clock + KEEP_PASS));
            self.steady = 0.0;
        }
        // A round flying straight leaves the muzzle along her crosshair's
        // line, not from her eye: that line is laid from the gun to it
        // (`Combat::manned_fire`), else it passes by as far off as her eye
        // is from the gun. (A lobbed one goes for where the crosshair
        // falls, so at it.)
        let sight = match &game.combat.manned {
            Some(m) if m.weapon.flight() != m2_engine::weapons::Flight::Lobbed => {
                p.eye() + (at - m.pivot)
            }
            _ => at,
        };
        let (look, error) = aim(p, sight, dt);
        // Far off, held closer on it: 0.03 rad is 4 m at 145 m, a car's
        // length, and 30 recoilless rounds missed PmcCon031's cars there
        // (round 32 of its thread).
        let on = ON_TARGET.min(MANNED_WIDTH / at.distance(p.eye()).max(1.0));
        self.steady = if error < on { self.steady + dt } else { 0.0 };
        Intent {
            look,
            trigger: Trigger {
                aim: true,
                fire: self.steady > STEADY
                    && muzzle_clear
                    && game.combat.manned.as_ref().is_none_or(|m| covers(m, at)),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Blowing up what stands at `at` with what she carries in the
    /// launcher's place: rockets from between [`BLAST_RANGE`], grenades
    /// from between [`LOB_RANGE`]; from a tank, with its cannon.
    fn blast(&mut self, game: &mut Game, target: u32, at: Vec3, dt: f32) -> Intent {
        if let Some(v) = game.scene.player.as_ref().and_then(|p| p.driving) {
            return self.at_target_driving(game, v, at, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let swimming_now = p.swimming();
        let d = at.distance(here);
        let lobbing = game.combat.second == Some(Weapon::GrenadeLauncher);
        let (near, far) = if lobbing { LOB_RANGE } else { BLAST_RANGE };
        if self.blast_spot.is_some_and(|(t, _)| t != target) {
            self.blast_spot = None;
            self.tried.clear();
            self.blocked = 0.0;
        }
        // Out of rockets (round 3's AllCon003: four for four AA guns, two
        // left standing): C4 instead, while she carries any.
        let rockets = game.combat.launcher_rounds.map_or(0, |r| r.total());
        // Not by an emplaced gun that blows it up (PmcCon031 round 28: C4 on
        // the flung cars from by the recoilless rifle, her rockets spent).
        if !lobbing
            && rockets == 0
            && (game.combat.charges > 0 || self.planted.is_some())
            && game.combat.manned.is_none()
            && self.gun_by(game, here, target, at).is_none()
        {
            return self.plant(game, at, dt);
        }
        // In the water she can't fire: out where she can climb, first.
        if swimming_now {
            let out = game
                .scene
                .world
                .as_ref()
                .and_then(|w| landing(&w.physics, here, at, &self.tried));
            // One she swims at and can't climb out at: another.
            if let Some(l) = self.landing
                && self.since_moved() > SPOT_GIVE_UP
            {
                eprintln!(
                    "autoplay {:.1} s: no way out at {l:.1} in {SPOT_GIVE_UP} s: another",
                    self.clock
                );
                self.tried.push(l);
                self.landing = None;
                self.fresh_headway();
                self.stranded += 1;
                // Swimming she gets out at none of them (round 33's
                // AllCon002: 11 banks in 2000 s): hopped out, as a hop
                // anywhere she makes no headway, and logged with them.
                if self.stranded >= STRANDED_HOP
                    && let Some(p) = game.scene.player.as_mut()
                {
                    eprintln!(
                        "autoplay {:.1} s: no headway for {:.0} s at {here:.1} ({}): hopped to {l:.1}",
                        self.clock,
                        SPOT_GIVE_UP * self.stranded as f32,
                        self.what
                    );
                    p.character.position = l + Vec3::Y * 0.1;
                    p.character.velocity = Vec3::ZERO;
                    self.hops.push((here, l));
                    self.stranded = 0;
                    return Intent::default();
                }
            }
            if let Some(out) = out {
                if self.landing.is_none_or(|l| l.distance(out) > 4.0) {
                    eprintln!(
                        "autoplay {:.1} s: in the water at {here:.1}: out at {out:.1}",
                        self.clock
                    );
                }
                self.landing = Some(out);
                self.blast_spot = None;
                self.blocked = 0.0;
                return self.walk(game, out, 1.0, dt);
            }
        } else if here.y > WATERLINE + LANDING_DRY {
            // Only once out on dry ground: in the shallows she is in and out
            // of swimming (round 34's AllCon002: 12 ways out, never hopped).
            self.stranded = 0;
        }
        self.landing = None;
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        // Nothing to blow it up with: an emplaced gun in reach of it,
        // manned and fired (PmcCon031 round 3: 50 columns to bring down,
        // the scripts' "MoveToMG" first, and she stood 370 s at one with
        // an empty launcher). So too with her weapons put away by the
        // scripts (round 4: its range's).
        let holstered = game.scene.player.as_ref().is_some_and(|p| p.weapons_away());
        // An emplaced gun right by her that reaches it: manned whatever
        // she carries (PmcCon031 round 17: at its grenade-launcher post she
        // shot 20 s with her rifle, then walked off round the range).
        // (One that breaks it: `gun_by`.)
        let by = self.gun_by(game, here, target, at);
        if self.gun_logged != Some(target) {
            self.gun_logged = Some(target);
            let (n, nearest) = game.placed.emplaced_nearest(&game.scene.props, here);
            eprintln!(
                "autoplay {:.1} s: for {target:#x}, {:.0} m off: a gun by her that blows it up \
                 {by:.1?}; nearest of {n} {nearest:x?}; manning {}",
                self.clock,
                at.distance(here),
                game.combat.manned.is_some()
            );
        }
        if (!lobbing && rockets == 0) || holstered || by.is_some() || game.combat.manned.is_some() {
            if let Some(m) = &game.combat.manned {
                let point = gun_point(game, m, target, at);
                // One that can't turn to it, with another by her that can:
                // off this one, for that.
                if point.is_none()
                    && by.is_some_and(|g| {
                        g.with_y(0.0).distance(m.pivot.with_y(0.0)) > crate::placed::MAN_REACH
                    })
                {
                    eprintln!(
                        "autoplay {:.1} s: the emplaced gun {:#x} doesn't turn to {target:#x}: \
                         to the one at {:.1}",
                        self.clock,
                        m.entity,
                        by.unwrap_or_default()
                    );
                    return Intent {
                        use_pressed: true,
                        ..Intent::default()
                    };
                }
                // Nor any gun by her: the others first, from this one; it
                // again with nothing else left, off the gun, for her own
                // weapons (PmcCon031 round 37: a column under the heavy MG's
                // tilt, 210 s aimed at it with 20 more to bring down).
                if point.is_none() {
                    if self.uncovered.insert(target) {
                        eprintln!(
                            "autoplay {:.1} s: the emplaced gun {:#x} doesn't turn to \
                             {target:#x}: the others first",
                            self.clock, m.entity
                        );
                        self.passed.push((target, self.clock + PASS_FOR));
                        return Intent::default();
                    }
                    eprintln!(
                        "autoplay {:.1} s: the emplaced gun {:#x} doesn't turn to {target:#x}: \
                         off it",
                        self.clock, m.entity
                    );
                    return Intent {
                        use_pressed: true,
                        ..Intent::default()
                    };
                }
                return self.fire_manned(game, target, point.unwrap_or(at), dt);
            }
            if let Some(gun) = by.or_else(|| {
                game.placed.emplacement_for(
                    &game.scene.props,
                    here,
                    at,
                    MAN_SEEK,
                    MANNED_REACH,
                    |g| !self.fired_dry(g),
                )
            }) {
                if gun.with_y(0.0).distance(here.with_y(0.0)) > crate::placed::MAN_REACH * 0.75 {
                    self.arrived = None;
                    return self.walk(game, gun, 0.5, dt);
                }
                return Intent {
                    use_pressed: true,
                    ..Intent::default()
                };
            }
            if self.no_gun_logged != Some(target) {
                self.no_gun_logged = Some(target);
                let (n, nearest) = game.placed.emplaced_nearest(&game.scene.props, here);
                let swimming = game.scene.player.as_ref().is_some_and(|p| p.swimming());
                eprintln!(
                    "autoplay {:.1} s: nothing to blow up {target:#x} with (rockets {rockets}, \
                     weapons away {holstered}, swimming {swimming}) and no emplaced gun in \
                     reach: {n} known, nearest {nearest:x?}",
                    self.clock
                );
            }
            // Her weapons out: rounds, which what breaks to them takes
            // (the ranges' targets, from their damage rows).
            if !holstered {
                return self.rifle_at(game, target, at, dt);
            }
            // Her weapons away and no gun to man: she stays where the
            // scripts put her until they give them back (PmcCon034 round
            // 8: she walked off its tower and the contract was cancelled
            // for leaving its bounds).
            return Intent::default();
        }
        // Going where she can fire from (in range, nothing in the way).
        if let Some((_, spot)) = self.blast_spot {
            let off = spot.with_y(0.0).distance(here.with_y(0.0));
            // Near enough once she gets no nearer (round 35's AllCon002:
            // stopped 3 m off, each such spot given up as unreached).
            let close = off < 2.0 * ARRIVE && self.since_moved() > SPOT_GIVE_UP;
            if off >= ARRIVE && !close {
                // One she gets no nearer to (round 28's AllCon002: 6 m short
                // of it, 2000 s of hops on the spot): another.
                if self.since_moved() > SPOT_GIVE_UP {
                    eprintln!(
                        "autoplay {:.1} s: no nearer the spot {spot:.1} in {SPOT_GIVE_UP} s: another",
                        self.clock
                    );
                    self.tried.push(spot);
                    self.blast_spot = None;
                    self.blocked = BLAST_BLOCKED;
                    self.fresh_headway();
                } else {
                    self.arrived = None;
                    return self.walk(game, spot, 1.0, dt);
                }
            }
            // Under it or over it (round 6's AllCon002: a roof by the
            // river, and she fell in under it): one she can't get to.
            if (spot.y - here.y).abs() > SPOT_RISE {
                eprintln!(
                    "autoplay {:.1} s: the spot {spot:.1} is {:.0} m from her height: another",
                    self.clock,
                    spot.y - here.y
                );
                self.tried.push(spot);
                self.blast_spot = None;
                self.blocked = BLAST_BLOCKED;
            }
        }
        if d > far {
            // A boat: from a dry spot in range, not straight out at it into
            // the river (rounds 30 to 33's AllCon002, 48 m short of the
            // first boat, in the water from there on).
            if self.blast_spot.is_none()
                && at.y < WATERLINE + AFLOAT
                && let Some(s) = game.scene.world.as_ref().and_then(|w| {
                    let rings: &[f32] = if lobbing { &LOB_RINGS } else { &BLAST_RINGS };
                    // Else the dry ground nearest it (round 34's AllCon002:
                    // no spot in range saw it, and she went in again).
                    // Within range of it (round 35's: spots 270 m off).
                    vantage(&w.physics, at, here, &self.tried, rings, BLAST_BODY).or_else(|| {
                        landing(&w.physics, at, here, &self.tried)
                            .filter(|s| s.distance(at) < far - ARRIVE)
                    })
                })
            {
                eprintln!(
                    "autoplay {:.1} s: {} is on the water: firing from {s:.1}",
                    self.clock, self.what
                );
                self.blast_spot = Some((target, s));
                self.fresh_headway();
                return self.walk(game, s, 1.0, dt);
            }
            // On the water with no dry ground in range of it (rounds 30 to
            // 37's first boat: she went in after it every time): another
            // for now, this one once the rest are done.
            if self.blast_spot.is_none()
                && at.y < WATERLINE + AFLOAT
                && !self
                    .passed
                    .iter()
                    .any(|&(t, until)| t == target && self.clock < until)
            {
                eprintln!(
                    "autoplay {:.1} s: no dry ground in range of {}: another for {PASS_FOR} s",
                    self.clock, self.what
                );
                self.passed.push((target, self.clock + PASS_FOR));
            }
            return self.walk(game, at, far - 5.0, dt);
        }
        if d < near {
            // Too close for a rocket or a round: back off, to a spot to
            // fire from if there is one (round 36's AllCon002: in the river
            // by the third boat, 1900 s getting no further off).
            if self.blast_spot.is_none()
                && let Some(s) = game.scene.world.as_ref().and_then(|w| {
                    let rings: &[f32] = if lobbing { &LOB_RINGS } else { &BLAST_RINGS };
                    vantage(&w.physics, at, here, &self.tried, rings, BLAST_BODY)
                })
            {
                eprintln!(
                    "autoplay {:.1} s: too close to {}: firing from {s:.1}",
                    self.clock, self.what
                );
                self.blast_spot = Some((target, s));
                self.fresh_headway();
                return self.walk(game, s, 1.0, dt);
            }
            let away = here + (here - at).with_y(0.0).normalize_or(Vec3::X) * 10.0;
            return self.walk(game, away, 1.0, dt);
        }
        let (look, error) = aim(p, at, dt);
        let reach = at.distance(p.pivot());
        let (from, dir) = Combat::aim_ray(p);
        // What the shot meets first: nothing, something past or near the
        // target, or its own body (a big vehicle's, round 3's MLRS, is hit
        // well short of its middle).
        let clear = game.scene.world.as_ref().is_some_and(|w| {
            Combat::aim_probe(&w.physics, &game.scene.people, p)
                .is_none_or(|(t, _)| t > reach - 4.0 || (from + dir * t).distance(at) < BLAST_BODY)
        });
        self.steady = if error < ON_TARGET {
            self.steady + dt
        } else {
            0.0
        };
        // In range a while with no shot (round 2's AllCon002: she stood
        // 2400 s aimed at the hilltop MLRS): from somewhere else on a ring
        // round it, another if that is no better.
        // Swimming she can't fire either.
        let swimming = game.scene.player.as_ref().is_some_and(|p| p.swimming());
        if clear && error < ON_TARGET && !swimming {
            self.blocked = 0.0;
        } else {
            self.blocked += dt;
        }
        if self.blocked > BLAST_BLOCKED {
            if let Some((_, s)) = self.blast_spot {
                self.tried.push(s);
            }
            self.blocked = 0.0;
            self.fresh_headway();
            let rings: &[f32] = if lobbing { &LOB_RINGS } else { &BLAST_RINGS };
            self.blast_spot = game
                .scene
                .world
                .as_ref()
                .and_then(|w| vantage(&w.physics, at, here, &self.tried, rings, BLAST_BODY))
                .map(|s| (target, s));
            self.nowhere = if self.blast_spot.is_none() {
                self.nowhere + 1
            } else {
                0
            };
            if self.nowhere >= PASS_AFTER {
                eprintln!(
                    "autoplay {:.1} s: nowhere to fire at {} from: another for {PASS_FOR} s",
                    self.clock, self.what
                );
                self.passed.push((target, self.clock + PASS_FOR));
                self.nowhere = 0;
                self.tried.clear();
            }
            eprintln!(
                "autoplay {:.1} s: no shot at {} from {here:.1} ({}, aim off {error:.3}): {}",
                self.clock,
                self.what,
                if clear {
                    "clear"
                } else {
                    "something in the way"
                },
                self.blast_spot
                    .map_or("nowhere better to fire from".to_owned(), |(_, s)| {
                        format!("going to {s:.1}")
                    })
            );
        }
        // Not over a drop, though (round 27's AllCon002: aimed at a boat
        // from the top of a bank, she stepped off it into the river).
        let ahead = here + (at - here).with_y(0.0).normalize_or_zero() * 2.0;
        let ledge = game.scene.world.as_ref().is_none_or(|w| {
            w.physics
                .cast_ray(ahead + Vec3::Y * 1.0, Vec3::NEG_Y, 1.0 + LEDGE_DROP)
                .is_none()
        });
        let movement = if !clear && self.steady > 1.0 && self.blast_spot.is_none() && !ledge {
            Vec2::Y
        } else {
            Vec2::ZERO
        };
        Intent {
            look,
            movement,
            walk: true,
            trigger: Trigger {
                aim: true,
                // A launcher fires once a pull: pulled and let go.
                fire: clear
                    && self.steady > STEADY
                    && Some(game.combat.weapon) == game.combat.second
                    && self.tap(),
                select: (Some(game.combat.weapon) != game.combat.second).then_some(Weapon::Rpg),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// C4 on what stands at `at`, as a player out of rockets would: up to
    /// within [`PLANT_FROM`], the C4 taken up and thrown at it, then off
    /// to [`PLANT_SAFE`] away (or [`PLANT_AWAY`] seconds on) and set off.
    /// Again while it stands and she has C4.
    fn plant(&mut self, game: &mut Game, at: Vec3, dt: f32) -> Intent {
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let d = (at - here).with_y(0.0).length();
        if let Some(since) = self.planted {
            if d >= PLANT_SAFE || self.clock - since > PLANT_AWAY {
                eprintln!(
                    "autoplay {:.1} s: setting off the C4 on {} from {d:.0} m",
                    self.clock, self.what
                );
                self.planted = None;
                return Intent {
                    trigger: Trigger {
                        detonate: true,
                        ..Trigger::default()
                    },
                    ..Intent::default()
                };
            }
            let away = at + (here - at).with_y(0.0).normalize_or(Vec3::X) * (PLANT_SAFE + 4.0);
            self.arrived = None;
            return self.walk(game, away, 1.0, dt);
        }
        if d > PLANT_FROM {
            return self.walk(game, at, PLANT_FROM - 1.0, dt);
        }
        // The C4 taken up (the throwables go round: grenades, C4, mines).
        if game.combat.explosive != 1 {
            return Intent {
                trigger: Trigger {
                    next_explosive: self.tap(),
                    ..Trigger::default()
                },
                ..Intent::default()
            };
        }
        let (look, error) = aim(p, at, dt);
        if error < ON_TARGET * 4.0 && self.tap() {
            eprintln!(
                "autoplay {:.1} s: C4 thrown at {} from {d:.1} m ({} left)",
                self.clock,
                self.what,
                game.combat.charges.saturating_sub(1)
            );
            self.planted = Some(self.clock);
            return Intent {
                look,
                trigger: Trigger {
                    throw: true,
                    ..Trigger::default()
                },
                ..Intent::default()
            };
        }
        Intent {
            look,
            ..Intent::default()
        }
    }

    /// Brings down `at`, which only a support strike fells: from between
    /// [`CALL_FROM`] off she calls the bomb as a player would (the support
    /// key, the use key, the view steered over it and confirmed, the
    /// wheel pressed in each wedge), then waits out of its reach.
    /// At her PDA: opened, looked at for [`PDA_LOOK`] seconds and shut
    /// once the scripts wait for that (else after [`PDA_MOST`]).
    fn pda(&mut self, open: bool, close_awaited: bool) -> Intent {
        let press = match open {
            false => true,
            true => {
                let looked = self.clock - *self.pda_since.get_or_insert(self.clock);
                (close_awaited && looked > PDA_LOOK) || looked > PDA_MOST
            }
        } && self.pda_pressed > PRESS_GAP;
        if press {
            self.pda_pressed = 0.0;
            if open {
                self.pda_since = None;
            }
            eprintln!(
                "autoplay {:.1} s: {} her PDA",
                self.clock,
                if open { "shutting" } else { "opening" }
            );
        }
        Intent {
            trigger: Trigger {
                pda: press,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    fn strike(&mut self, game: &mut Game, at: Vec3, dt: f32) -> Intent {
        let flat = |a: Vec3| a.with_y(0.0).distance(at.with_y(0.0));
        if let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) {
            let speed = vehicle_state(game, i).map_or(0.0, |s| s.1);
            let d = vehicle_at(game, i).map_or(0.0, |m| flat(m.w_axis.truncate()));
            if d < CALL_FROM.1 {
                return self.press(speed);
            }
            return self.drive(game, i, at, CALL_FROM.1 - 15.0, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let d = flat(here);
        let support = &game.support;
        let step = match support.phase {
            Phase::Idle => "idle",
            Phase::Menu => "the support menu",
            Phase::Holding { .. } => "the designator",
            Phase::Viewing { .. } => "the satellite's view",
            Phase::Timing { .. } => "the timing wheel",
            Phase::Confirmed { .. } => "confirmed",
            Phase::Designating { .. } => "aiming a designator",
        };
        if step != self.support_logged {
            self.support_logged = step;
            eprintln!(
                "autoplay {:.1} s: support: {step}, {d:.0} m from the target{}",
                self.clock,
                support
                    .inbound()
                    .map_or_else(String::new, |t| format!(", a bomb lands in {t:.1} s"))
            );
        }
        let away = at + (here - at).with_y(0.0).normalize_or(Vec3::X) * (CALL_FROM.0 + 10.0);
        let waiting = support.phase == Phase::Idle
            && (support.inbound().is_some() || support.ready().is_none());
        if waiting || support.phase == Phase::Idle {
            if d < CALL_FROM.0 {
                return self.walk(game, away, 2.0, dt);
            }
            if waiting {
                // Watching where it is to come down.
                let look = game
                    .scene
                    .player
                    .as_ref()
                    .map_or(Vec2::ZERO, |p| aim(p, at, dt).0);
                return Intent {
                    look,
                    ..Intent::default()
                };
            }
            if d > CALL_FROM.1 {
                return self.walk(game, at, CALL_FROM.1 - 15.0, dt);
            }
        }
        let mut intent = Intent::default();
        match support.phase {
            Phase::Idle => intent.trigger.support = true,
            Phase::Menu => intent.use_pressed = true,
            // The view opens by itself.
            Phase::Holding { .. } => {}
            Phase::Viewing { at: view, .. }
            | Phase::Designating {
                aim: Aim::Satellite,
                at: view,
                ..
            } => {
                let to = Vec2::new(at.x, at.z) - view;
                if to.length() < ON_SPOT {
                    intent.trigger.fire = self.tap();
                } else {
                    // Screen right is the world's -x, looking down.
                    intent.movement = Vec2::new(-to.x, to.y).clamp_length_max(1.0);
                }
            }
            Phase::Timing { .. } => intent.trigger.fire = support.wheel_ready() && self.tap(),
            Phase::Confirmed { .. } => {}
            // The laser held on the target.
            Phase::Designating {
                aim: Aim::Laser, ..
            } => {
                if let Some(p) = game.scene.player.as_ref() {
                    intent.look = aim(p, at, dt).0;
                }
                intent.trigger.fire = true;
            }
        }
        intent
    }

    /// Fire pressed and let go on alternate ticks: presses, not a hold.
    fn tap(&mut self) -> bool {
        self.tapped = !self.tapped;
        self.tapped
    }

    /// At a target from vehicle `i`: a cannon (a tank's, the Stryker's)
    /// fires at it from within range; from anything else she gets out
    /// close to it.
    fn at_target_driving(&mut self, game: &mut Game, i: usize, target: Vec3, dt: f32) -> Intent {
        let Some((m, speed)) = vehicle_state(game, i) else {
            return Intent::default();
        };
        let Some(v) = game.scene.vehicles.get(i) else {
            return Intent::default();
        };
        if self.leave {
            return self.press(speed);
        }
        let cannon = v.spec.cannon();
        let d = target.distance(m.w_axis.truncate());
        if !cannon {
            if d < GET_OUT {
                return self.press(speed);
            }
            return self.drive(game, i, target, GET_OUT - 5.0, dt);
        }
        if d > CANNON_RANGE {
            return self.drive(game, i, target, CANNON_RANGE - 10.0, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let (look, error) = aim(p, target, dt);
        // The turret follows the camera at its own pace, and its gun
        // tilts after it (as far as it goes).
        let f = m.transform_vector3(Vec3::Z);
        let turret = f.x.atan2(f.z) + v.turret;
        let off = wrap(p.yaw + look.x - turret).abs();
        let laid = v.spec.muzzle_pose(v.turret, v.gun).is_none_or(|(at, dir)| {
            let (at, dir) = (m.transform_point3(at), m.transform_vector3(dir));
            let (down, up) = v.spec.elevation;
            let stopped = v.gun <= down + 0.01 || v.gun >= up - 0.01;
            stopped || (target - at).normalize_or(dir).angle_between(dir) < TURRET_ON
        });
        // Its gun at its stop with the goal past it (round 14's AMX30 under
        // the hilltop MLRS): out soon. In range this long and the goal
        // still there anyway (round 8's tank sat 250 s): out too, to go on
        // on foot.
        let beyond = v
            .spec
            .muzzle_pose(v.turret, v.gun)
            .is_some_and(|(at, dir)| {
                let (at, dir) = (m.transform_point3(at), m.transform_vector3(dir));
                let (down, up) = v.spec.elevation;
                let want = (target - at).normalize_or(dir);
                (v.gun >= up - 0.01 && want.y > dir.y + 0.05)
                    || (v.gun <= down + 0.01 && want.y < dir.y - 0.05)
            });
        self.gunning += dt;
        let give_up = if beyond {
            CANNON_BEYOND
        } else {
            CANNON_GIVE_UP
        };
        if self.gunning > give_up && self.may_get_out(game, i) {
            eprintln!(
                "autoplay {:.1} s: the {} has had {} in range {:.0} s: getting out",
                self.clock, v.spec.label, self.what, self.gunning
            );
            self.leave = true;
            return self.press(speed);
        }
        self.steady = if error < ON_TARGET && off < TURRET_ON && laid {
            self.steady + dt
        } else {
            0.0
        };
        Intent {
            look,
            hold_jump: speed.abs() > 0.3,
            trigger: Trigger {
                fire: self.steady > STEADY,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Into vehicle `vehicles[i]`: to its door, then the use key; out of
    /// any other first.
    fn board(&mut self, game: &mut Game, i: usize, dt: f32) -> Intent {
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        if let Some(other) = p.driving {
            let speed = vehicle_state(game, other).map_or(0.0, |s| s.1);
            return self.press(speed);
        }
        // Going for it this long without getting in (round 19's AllCon001:
        // 1330 s after a semi 320 m off, her way round replanned each time
        // so she never hopped): left, on foot or another.
        let since = match self.boarding {
            Some((j, t)) if j == i => t,
            _ => {
                self.boarding = Some((i, self.clock));
                self.clock
            }
        };
        if self.clock - since > BOARD_FOR && !self.left_rides.contains(&i) {
            eprintln!(
                "autoplay {:.1} s: the {} not reached in {BOARD_FOR:.0} s: left",
                self.clock, game.scene.vehicles[i].spec.label
            );
            self.left_rides.push(i);
        }
        let Some(m) = vehicle_at(game, i) else {
            return Intent::default();
        };
        let spec = &game.scene.vehicles[i].spec;
        // Its door, else beside it on the left.
        let door = spec.door.map_or_else(
            || {
                let (lo, hi) = spec.bounds;
                m.transform_point3(Vec3::new(hi.x + 0.8, 0.0, (lo.z + hi.z) * 0.5))
            },
            |d| m.transform_point3(d),
        );
        let here = p.character.position;
        if (door - here).with_y(0.0).length() < DOOR {
            return self.press(0.0);
        }
        // Crewed and taken only by its hijack (PmcCon001's villa tank):
        // the key anywhere close by starts it, door or not (round 4 stood
        // 3 m off its door for minutes, the way to it blocked).
        let v = &game.scene.vehicles[i];
        if game.hijack_only.contains(&v.spec.model)
            && !v.crew.is_empty()
            && (m.w_axis.truncate() - here).length() < HIJACK_NEAR
        {
            return self.press(0.0);
        }
        self.walk(game, door, 1.0, dt)
    }

    /// Where to head for on her way by road to `at` from `pos`, once it is
    /// farther than [`ROAD_FROM`]: a road point [`ROAD_AHEAD`] on along
    /// the way, else `at` itself (no way by road, or near its end).
    fn by_road(&mut self, game: &Game, pos: Vec3, at: Vec3) -> Vec3 {
        let flat = |a: Vec3, b: Vec3| (a - b).with_y(0.0).length();
        if flat(pos, at) < ROAD_FROM {
            self.road_way = None;
            return at;
        }
        let Some(world) = game.scene.world.as_ref() else {
            return at;
        };
        let roads = self
            .roads
            .get_or_insert_with(|| RoadGraph::new(&world.index.roads));
        let strayed = |way: &[Vec3]| way.iter().all(|&p| flat(p, pos) > ROAD_REPLAN);
        let stale = self
            .road_way
            .as_ref()
            .is_none_or(|(goal, way)| flat(*goal, at) > ROAD_REPLAN || strayed(way));
        if stale {
            let way = roads.way(pos, at, ROAD_REACH).unwrap_or_default();
            let length: f32 = way.windows(2).map(|w| w[0].distance(w[1])).sum();
            eprintln!(
                "autoplay {:.1} s: by road to {} ({:.0} m off): {}",
                self.clock,
                self.what,
                flat(pos, at),
                if way.is_empty() {
                    "no way".to_owned()
                } else {
                    format!("{} points, {length:.0} m", way.len())
                }
            );
            self.road_way = Some((at, way));
        }
        let Some((_, way)) = &self.road_way else {
            return at;
        };
        // From the road point nearest her, the first one far enough on.
        let Some(here) =
            (0..way.len()).min_by(|&a, &b| flat(way[a], pos).total_cmp(&flat(way[b], pos)))
        else {
            return at;
        };
        way[here..]
            .iter()
            .copied()
            .find(|&p| flat(p, pos) >= ROAD_AHEAD)
            .unwrap_or(at)
    }

    /// Where to head for on her way to `at` from `pos` (driving vehicle
    /// `driving`, or on foot), going `speed` (m/s): along the way planned
    /// to it ([`Route`]), planned again as said at [`PLAN_FROM`]; straight
    /// at it from within that.
    fn waypoint(
        &mut self,
        game: &Game,
        driving: Option<usize>,
        pos: Vec3,
        at: Vec3,
        speed: f32,
    ) -> Vec3 {
        // In a boat, a place up out of the sea (PirCon001's checkpoint 30
        // stands on a dock 4 m up: her jetski ran into the dock, and lost
        // the gate) is gone for on the water short of it, on her side: a
        // race's checkpoint is passed within 10 m (`MrxTaskRace`'s
        // `fWidth`).
        let boat = driving
            .and_then(|i| game.scene.vehicles.get(i))
            .is_some_and(|v| v.spec.is_boat());
        let sea = game
            .scene
            .world
            .as_ref()
            .map_or(WATERLINE, |w| w.settings.sea_level.max(WATERLINE));
        let at = if boat && at.y > sea + ASHORE {
            let back = (pos - at).with_y(0.0).normalize_or_zero();
            (at + back * SHORT_OF_SHORE).with_y(sea)
        } else {
            at
        };
        // Far, by road.
        let at = if boat {
            at
        } else {
            self.by_road(game, pos, at)
        };
        let straight = (at - pos).with_y(0.0).length();
        if straight < PLAN_FROM {
            self.way = None;
            return at;
        }
        // Driving, planned again as soon as a vehicle comes to stand near
        // her (PirCon004 round 77: a fuel trailer the scripts put on her
        // road, run into before the way was planned again).
        let standing = match driving {
            Some(i) => standing_near(game, i, pos, STANDING_FRESH),
            None => Vec::new(),
        };
        let stale = self.way.as_ref().is_none_or(|w| {
            w.driving != driving.is_some()
                || w.route.goal.distance(at) > REPLAN_MOVED
                || self.clock - w.planned > REPLAN
                || (self.clock - w.planned > REPLAN_SOONEST
                    && standing.iter().any(|j| !w.standing.contains(j)))
        });
        if stale && let Some(world) = game.scene.world.as_ref() {
            let mover = match driving.and_then(|i| game.scene.vehicles.get(i)) {
                Some(v) => Mover {
                    cell: DRIVE_CELL,
                    steepest: if v.spec.tracked { ON_TRACKS } else { ON_WHEELS },
                    wades: false,
                    skip: v.id,
                    afloat: v.spec.is_boat().then(|| keel_depth(&v.spec) + UNDER_KEEL),
                },
                None => Mover {
                    cell: FOOT_CELL,
                    steepest: ON_FOOT,
                    wades: true,
                    skip: None,
                    afloat: None,
                },
            };
            let sea = world.settings.sea_level.max(WATERLINE);
            // Driving, she keeps out of what she stuck on, and round the
            // vehicles standing near (PirCon004 rounds 63-70: her way ran
            // through a parked fuel trailer, which wrecked her truck).
            let avoid: Vec<(Vec2, Vec2)> = match driving {
                Some(i) => self
                    .avoid
                    .iter()
                    .copied()
                    .chain(parked_near(game, i, pos).into_iter().filter(|&(lo, hi)| {
                        // Not one standing where she is going.
                        let goal = Vec2::new(at.x, at.z);
                        !(goal.cmpge(lo).all() && goal.cmple(hi).all())
                    }))
                    .collect(),
                None => Vec::new(),
            };
            let route = Route::plan_avoiding(&world.physics, mover, (pos, at), sea, &avoid);
            if (!route.reaches || route.length() > straight * ROUNDABOUT)
                && self.clock - self.way_logged > LOG_WAY
            {
                self.way_logged = self.clock;
                let end = route.points.last().copied().unwrap_or(pos);
                let short = (route.goal - end).with_y(0.0).length();
                let how = if route.reaches {
                    format!(
                        "goes round: {:.0} m, not {straight:.0} m straight",
                        route.length()
                    )
                } else if short < 0.5 {
                    "is not found: no step brings her nearer, so straight at it".to_owned()
                } else {
                    format!(
                        "is not found: {:.0} m of way ends {short:.0} m short",
                        route.length()
                    )
                };
                eprintln!(
                    "autoplay {:.1} s: the way to {} from {pos:.0} {how}, by {} points",
                    self.clock,
                    self.what,
                    route.points.len()
                );
            }
            // Headway is measured along the new way from here on.
            let left = route.remaining(pos);
            if left > self.best.0 + 1.0 {
                self.best.0 = left;
            }
            self.way = Some(Way {
                route,
                planned: self.clock,
                driving: driving.is_some(),
                standing,
            });
        }
        let Some(way) = &mut self.way else {
            return at;
        };
        let ahead = (LOOK_AHEAD + speed.abs() * LOOK_TIME).min(LOOK_MOST);
        let (toward, off) = way.route.steer(pos, ahead);
        if off > STRAYED {
            self.way = None;
        }
        toward
    }

    /// How far she has still to go from `pos` to `at`, metres: along her
    /// way, else straight.
    fn left_to_go(&self, pos: Vec3, at: Vec3) -> f32 {
        self.way.as_ref().map_or_else(
            || (at - pos).with_y(0.0).length(),
            |w| w.route.remaining(pos),
        )
    }

    /// Measures headway afresh: a new goal, after a hop.
    fn fresh_headway(&mut self) {
        self.best = (f32::INFINITY, self.clock);
        self.moved = None;
    }

    /// Seconds since she last made headway that counts towards the hop.
    fn since_moved(&self) -> f32 {
        self.clock - self.moved.map_or(self.best.1, |(_, at)| at)
    }

    /// The use key, once a [`PRESS_GAP`]: stopped first when driving.
    fn press(&mut self, speed: f32) -> Intent {
        if speed.abs() > 1.0 {
            return Intent {
                hold_jump: true,
                ..Intent::default()
            };
        }
        let press = self.pressed > PRESS_GAP;
        if press {
            self.pressed = 0.0;
        }
        Intent {
            use_pressed: press,
            hold_jump: true,
            ..Intent::default()
        }
    }

    /// Whether she can get out of vehicle `i` where it is: anything on
    /// land, a boat not over deep water ([`Self::can_leave`]).
    fn may_get_out(&self, game: &Game, i: usize) -> bool {
        game.scene
            .vehicles
            .get(i)
            .is_some_and(|v| !v.spec.is_boat())
            || self.can_leave(game, i)
    }

    /// Whether vehicle `i` is a boat she can get out of where it is (not
    /// over deep water).
    fn can_leave(&self, game: &Game, i: usize) -> bool {
        let (Some(world), Some(v)) = (&game.scene.world, game.scene.vehicles.get(i)) else {
            return false;
        };
        v.spec.is_boat() && can_get_out(&world.physics, v)
    }

    /// Keeps track of how close she has come to `at` (`d` metres off, on
    /// her way): short of headway she sidesteps and jumps, long short of it
    /// she hops ahead (true then: nothing more this tick).
    fn headway(&mut self, game: &mut Game, at: Vec3, d: f32) -> bool {
        if d < self.best.0 - 1.0 {
            self.best = (d, self.clock);
            let here = game.focus();
            if self
                .moved
                .is_none_or(|(was, _)| was.distance(here) > HEADWAY_MOVE)
            {
                self.moved = Some((here, self.clock));
            }
            return false;
        }
        let stuck = self.clock - self.best.1;
        if self.arrived.is_some() {
            return false;
        }
        let still = self.since_moved();
        // A boat aground hops sooner: a race's gate waits no 25 s
        // (GurCon003 round 85 ran aground on a sandbar; its timer ran out
        // 16 s on).
        let afloat = game
            .scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| game.scene.vehicles.get(i))
            .is_some_and(|v| v.spec.is_boat());
        // So does one on foot stuck short of a place close by: the timed
        // ranges' points lie behind their counters (PmcCon033 round 9: 31
        // s of its 90 s lost 36 m short of the pistol range's second).
        let near = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_none())
            && self
                .goal_at()
                .is_some_and(|at| at.distance(game.focus()) < HOP_NEAR);
        if still
            > if afloat || near {
                HOP_AFTER_BOAT
            } else {
                HOP_AFTER
            }
        {
            // Stuck driving close to the place, or stuck driving a second
            // time on the way there (round 26: a truck hopped a few metres
            // at a time for 370 s, 165 m off): on foot from here. Not
            // with goods riding on her vehicle: they are what she brings
            // there (PirCon002's rum, left 162 m short in round 43); the
            // vehicle hops on with them instead.
            let driving = game.scene.player.as_ref().and_then(|p| p.driving);
            let near = (at - game.focus()).with_y(0.0).length() < RIDE_FOR;
            // Nor in a boat: off it she is in the sea, and a race is lost
            // (PirCon001 round 98 got off her jetski 58 m from a gate).
            if let Some(i) = driving
                && (near || self.drive_hops > 0)
                && !afloat
                && !game.scene.props.carrying(i)
                && game
                    .scene
                    .world
                    .as_ref()
                    .is_none_or(|w| can_get_out(&w.physics, &game.scene.vehicles[i]))
            {
                eprintln!(
                    "autoplay {:.1} s: no headway driving for {still:.0} s, {d:.0} m off: getting out",
                    self.clock
                );
                self.leave = true;
                self.fresh_headway();
                return false;
            }
            // Ahead along her way, else (a way that ends where she is:
            // round 86 hopped her 2 m twice by gate 1, to the floor she
            // was on) straight at the place.
            let from = game.focus();
            let to = self
                .way
                .as_mut()
                .map(|w| w.route.steer(from, HOP).0)
                .filter(|to| (*to - from).with_y(0.0).length() >= HOP_LEAST)
                .unwrap_or(at);
            if driving.is_some() {
                self.drive_hops += 1;
            }
            self.hop(game, to, still);
            return true;
        }
        if stuck > STUCK && self.sidestep.is_none() && (stuck % STUCK) < 0.05 {
            self.side = if self.side > 0.0 { -1.0 } else { 1.0 };
            self.sidestep = Some((self.side, SIDESTEP));
        }
        false
    }

    /// Moves her (and what she drives) up to [`HOP`] metres towards `at`,
    /// onto the ground there; a boat, onto water deep enough near there
    /// (else she gets out of it, onto the ground).
    fn hop(&mut self, game: &mut Game, at: Vec3, stuck: f32) {
        let from = game.focus();
        let flat = (at - from).with_y(0.0);
        // Not onto what she is blowing up (round 20's AllCon003: hopped onto
        // the last AA gun, too close to fire and stuck on it 2000 s).
        // Kept off the thing itself, not off the point of her way hopped to
        // (round 29's AllCon002: a point 15 m on, 376 m from the boat, and
        // 56 hops of 0 m).
        let room = match self.goal {
            Some(Goal::Blast { at: t, .. }) => {
                (t - from).with_y(0.0).length() - (BLAST_RANGE.0 + 5.0)
            }
            _ => f32::INFINITY,
        };
        let step = flat.length().min(HOP).min(room.max(0.0));
        let mut to = from + flat.normalize_or_zero() * step;
        // Already nearer than that (round 36's AllCon002: 5 m from the third
        // boat, hopped straight down to the riverbed 25 times): out to it.
        if let Some(Goal::Blast { at: t, .. }) = self.goal
            && room < 0.0
        {
            to = from + (from - t).with_y(0.0).normalize_or(Vec3::X) * (-room).min(HOP);
        }
        let (Some(world), Some(p)) = (&mut game.scene.world, &mut game.scene.player) else {
            return;
        };
        // Onto the ground by the way's height there (the floor under a
        // roof), else the highest there is.
        let ground = |physics: &Physics, to: Vec3| {
            let near = Vec3::new(to.x, at.y + HOP_ABOVE, to.z);
            let high = Vec3::new(to.x, to.y.max(at.y) + 80.0, to.z);
            physics
                .cast_ray(near, Vec3::NEG_Y, HOP_ABOVE + HOP_BELOW)
                .map(|(t, _)| near.y - t)
                .or_else(|| {
                    physics
                        .cast_ray(high, Vec3::NEG_Y, 400.0)
                        .map(|(t, _)| high.y - t)
                })
        };
        // On foot, not into water out of her depth, where she can't fire
        // (OilCon002 round 3: hopped into the harbour by the first post,
        // then 1500 s swimming under three soldiers' fire): shorter, else
        // as far again, to dry ground.
        let wet = |physics: &Physics, to: Vec3, y: f32| {
            physics
                .water_at(to.x, to.z)
                .is_some_and(|w| w - y > crate::player::SWIM_START)
        };
        if p.driving.is_none()
            && let Some(y) = ground(&world.physics, to)
            && wet(&world.physics, to, y)
        {
            let dir = flat.normalize_or_zero();
            let reach = (to - from).with_y(0.0).length();
            let dry = (1..=8)
                .map(|k| reach - 3.0 * k as f32)
                .filter(|&l| l > 2.0)
                .chain((1..=8).map(|k| reach + 3.0 * k as f32))
                .map(|l| from + dir * l)
                .find(|&t| ground(&world.physics, t).is_some_and(|y| !wet(&world.physics, t, y)));
            if let Some(t) = dry {
                to = t;
            }
        }
        if let Some(y) = ground(&world.physics, to) {
            to.y = y;
        }
        let yaw = flat.x.atan2(flat.z);
        // A boat she drives, and its body.
        let boat = p
            .driving
            .filter(|&i| game.scene.vehicles[i].spec.is_boat())
            .and_then(|i| game.scene.vehicles[i].id.map(|id| (i, id)));
        match (p.driving, boat) {
            // Round 88 hopped the start boat 35 m up onto the rocks it
            // was stuck on.
            (_, Some((i, id))) => {
                let v = &mut game.scene.vehicles[i];
                eprintln!(
                    "autoplay {:.1} s: stuck driving the {}: {}",
                    self.clock,
                    v.spec.label,
                    stuck_report(world, id, v.spec.hull, self.reversing.is_some())
                );
                match afloat_near(&world.physics, &v.spec, id, from, to, yaw) {
                    Some(spot) => {
                        world.physics.remove_vehicle(id);
                        v.id = None;
                        if !place(&mut world.physics, v, spot, yaw) {
                            eprintln!(
                                "autoplay: no water to put the {} on at {spot:.1}",
                                v.spec.label
                            );
                        }
                        to = spot;
                    }
                    None => {
                        eprintln!(
                            "autoplay: no water deep enough for the {} near {to:.1}: out of it",
                            v.spec.label
                        );
                        world.physics.drive(
                            id,
                            VehicleControls {
                                parked: true,
                                ..VehicleControls::default()
                            },
                        );
                        get_out(&world.physics, v, p);
                        p.character.position = to + Vec3::Y * 0.1;
                        p.character.velocity = Vec3::ZERO;
                    }
                }
            }
            (Some(i), None) => {
                let v = &mut game.scene.vehicles[i];
                if let Some(id) = v.id {
                    eprintln!(
                        "autoplay {:.1} s: stuck driving the {}: {}",
                        self.clock,
                        v.spec.label,
                        stuck_report(world, id, v.spec.hull, self.reversing.is_some())
                    );
                }
                if let Some(id) = v.id.take() {
                    world.physics.remove_vehicle(id);
                }
                if !place(&mut world.physics, v, to, yaw) {
                    eprintln!(
                        "autoplay: no ground to put the {} on at {to:.1}",
                        v.spec.label
                    );
                }
            }
            (None, _) => {
                p.character.position = to + Vec3::Y * 0.1;
                p.character.velocity = Vec3::ZERO;
            }
        }
        eprintln!(
            "autoplay {:.1} s: no headway for {stuck:.0} s at {from:.1} ({}): hopped to {to:.1}",
            self.clock, self.what
        );
        let brought = game.scene.people.bring_followers(to);
        if brought > 0 {
            eprintln!(
                "autoplay {:.1} s: {brought} following her hopped with her",
                self.clock
            );
        }
        self.hops.push((from, to));
        // A ride she couldn't get to is left (round 10's AllCon002 spent
        // 160 s after a WZ551 behind a wall): another or on foot.
        if let Some(Goal::Board(i)) = self.goal
            && !self.left_rides.contains(&i)
        {
            self.left_rides.push(i);
        }
        self.fresh_headway();
        self.sidestep = None;
        self.reversing = None;
        self.way = None;
    }

    /// Fallen through a hole in the island (it hadn't streamed in under
    /// her): back up on the ground.
    fn ground(&mut self, game: &mut Game) {
        let (Some(world), Some(p)) = (&game.scene.world, &mut game.scene.player) else {
            return;
        };
        if p.driving.is_some() {
            return;
        }
        let at = p.character.position;
        // Something under her: not fallen through (a roof over her head
        // doesn't count).
        if world
            .physics
            .cast_ray(at + Vec3::Y, Vec3::NEG_Y, 300.0)
            .is_some()
        {
            return;
        }
        let top = Vec3::new(at.x, at.y + 300.0, at.z);
        let Some((t, _)) = world.physics.cast_ray(top, Vec3::NEG_Y, 600.0) else {
            return;
        };
        let ground = top.y - t;
        if at.y < ground - UNDER {
            eprintln!(
                "autoplay {:.1} s: fell through at {at:.1}; back up on the ground at {ground:.1}",
                self.clock
            );
            p.character.position.y = ground + 0.1;
            p.character.velocity = Vec3::ZERO;
        }
    }

    /// How she is getting on, for the log.
    fn report(&self, game: &Game) {
        let Some(p) = game.scene.player.as_ref() else {
            return;
        };
        let riding = p
            .driving
            .and_then(|i| game.scene.vehicles.get(i))
            .map(|v| format!("driving the {}", v.spec.label));
        // What may hold her still (round 97's tank stood 125 s).
        let mut holding = String::new();
        if let Some(since) = self.arrived {
            holding += &format!("; there {:.0} s", self.clock - since);
        }
        if game.support.busy() {
            holding += &format!("; support {:?}", game.support.phase);
        }
        // Why a fight goes on without rounds (OilCon002 rounds 1-2).
        if self.blocked > 0.0 || self.steady > 0.0 || self.vantage.is_some() {
            holding += &format!(
                "; aim steady {:.1} s, blocked {:.1} s, vantage {:?}, {} tried",
                self.steady,
                self.blocked,
                self.vantage.map(|(_, s)| s),
                self.tried.len()
            );
            if let Some(Goal::Shoot(i)) = self.goal
                && let Some(n) = game.scene.people.npcs.get(i)
            {
                holding += &format!(", target at {:.1}", n.character.position);
            }
        }
        eprintln!(
            "autoplay {:.1} s: at {:.1}, {}, health {:.0}, {}; closest {:.0} m, {:.0} s ago; {} hops{holding}; {}",
            self.clock,
            game.focus(),
            riding.unwrap_or_else(|| "on foot".to_owned()),
            game.combat.hp,
            self.what,
            self.best.0,
            self.since_moved(),
            self.hops.len(),
            game.scene.people.fight_summary()
        );
        eprintln!(
            "autoplay tally {:.1} s: {}; {}; {}",
            self.clock,
            game.combat.tally.line(game.combat.fired.rifle),
            game.scene.people.idle.line(),
            game.combat.shots_line()
        );
        eprintln!(
            "autoplay checks {:.1} s: {}; arrived {:?}, blocked {:.1} s, steady {:.1} s, landed {}",
            self.clock,
            game.scene.people.checks.line(),
            self.arrived.map(|t| self.clock - t),
            self.blocked,
            self.steady,
            p.landed
        );
    }

    /// A summary for the end of the run.
    pub fn summary(&self) -> String {
        format!(
            "{:.1} s played, {} hops{}",
            self.clock,
            self.hops.len(),
            self.hops
                .iter()
                .map(|(a, b)| format!("; {a:.0} to {b:.0}"))
                .collect::<String>()
        )
    }
}

/// Whether she wants what lies at `p`: a weapon she doesn't carry (one
/// she left herself only once what took its place is spent), or
/// ammunition or health she would take walking over it (its template's
/// `Pickup` records for the weapons she carries, with room for them). A
/// rifle lying there (a dead soldier's `Assault Rifle`) she wants only for
/// the rounds she would take walking over it: she keeps her own.
fn wanted(templates: &Templates, combat: &Combat, p: &crate::flights::Pickup) -> bool {
    match p.what.weapon {
        Some(w) if p.from == 0 => combat.second != Some(w) && !combat.second_loaded(),
        Some(w) if w != Weapon::Rifle => combat.second != Some(w),
        // A rifle she doesn't carry gives nothing short of taking it up.
        Some(_)
            if crate::mission::offer(templates, combat, &p.what)
                == crate::mission::Offer::Equip =>
        {
            false
        }
        _ => {
            let refill = crate::mission::refill(templates, combat, &p.what);
            combat.can_use(refill, p.what.health)
        }
    }
}

/// A spot she can stand on and see `target` from: on `rings` round it,
/// with ground under it and nothing between eye height there and `short`
/// of the target, not within a few metres of one already `tried`; of the
/// nearest ring with any, the one closest to `from`, preferring ground
/// about as high as hers.
fn vantage(
    physics: &Physics,
    target: Vec3,
    from: Vec3,
    tried: &[Vec3],
    rings: &[f32],
    short: f32,
) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for &r in rings {
        for k in 0..24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let (x, z) = (target.x + r * a.sin(), target.z + r * a.cos());
            let top = Vec3::new(x, target.y.max(from.y) + 30.0, z);
            let Some((d, n)) = physics.cast_ray(top, Vec3::NEG_Y, 120.0) else {
                continue;
            };
            let spot = Vec3::new(x, top.y - d, z);
            // Flat enough, out of the water, and not one tried.
            if n.y < 0.7 || spot.y < WATERLINE + 0.3 || tried.iter().any(|t| t.distance(spot) < 4.0)
            {
                continue;
            }
            let eye = spot + Vec3::Y * 1.6;
            let (dir, len) = ((target - eye).normalize_or_zero(), target.distance(eye));
            // Short of the target's own body.
            if physics.cast_ray(eye, dir, (len - short).max(0.0)).is_some() {
                continue;
            }
            let climb = (spot.y - from.y).abs();
            let cost = from.distance(spot) + if climb > VANTAGE_CLIMB { 100.0 } else { 0.0 };
            if best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, spot));
            }
        }
        if best.is_some_and(|(c, _)| c < 100.0) {
            break;
        }
    }
    best.map(|(_, s)| s)
}

/// A spot on her own level she can shoot `target` from with her rifle:
/// on [`PERCH_RINGS`] round `from`, with ground under it no more than
/// [`PERCH_LEVEL`] above or below hers and all the way to it (no drop
/// off her roof between), nothing in her way there, within every region
/// of `keep`, not within 2 m of a spot `tried`, and nothing between eye
/// height there and the target's own body. Of the nearest ring with any,
/// the one nearest the target.
fn perch(
    physics: &Physics,
    target: Vec3,
    from: Vec3,
    tried: &[Vec3],
    keep: &[Keep],
) -> Option<Vec3> {
    // Her level's ground at (x, z), if there is some.
    let ground = |x: f32, z: f32| {
        let top = Vec3::new(x, from.y + PERCH_LEVEL + 0.5, z);
        physics
            .cast_ray(top, Vec3::NEG_Y, 2.0 * PERCH_LEVEL + 0.5)
            .filter(|&(_, n)| n.y >= 0.7)
            .map(|(d, _)| Vec3::new(x, top.y - d, z))
    };
    for &r in &PERCH_RINGS {
        let mut best: Option<(f32, Vec3)> = None;
        for k in 0..24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let Some(spot) = ground(from.x + r * a.sin(), from.z + r * a.cos()) else {
                continue;
            };
            if tried.iter().any(|t| t.distance(spot) < 2.0) || !keep.iter().all(|k| k.holds(spot)) {
                continue;
            }
            // The way there: ground at every stride, nothing at her middle.
            let steps = (r / PERCH_STRIDE).ceil() as usize;
            let floor = (1..steps).all(|i| {
                let q = from.lerp(spot, i as f32 / steps as f32);
                ground(q.x, q.z).is_some()
            });
            let waist = from + Vec3::Y * 1.0;
            let to = spot + Vec3::Y * 1.0;
            if !floor
                || physics
                    .cast_ray(waist, (to - waist).normalize_or_zero(), waist.distance(to))
                    .is_some()
            {
                continue;
            }
            let eye = spot + Vec3::Y * 1.6;
            let (dir, len) = ((target - eye).normalize_or_zero(), target.distance(eye));
            if physics.cast_ray(eye, dir, (len - 1.5).max(0.0)).is_some() {
                continue;
            }
            let cost = spot.distance(target);
            if best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, spot));
            }
        }
        if let Some((_, s)) = best {
            return Some(s);
        }
    }
    None
}

/// Where she can climb out of the water nearest `at`: ground on
/// [`LANDING_RINGS`] round her, flat enough, no deeper than she can stand
/// in and no more than [`SPOT_RISE`] over the water (round 27's AllCon002:
/// in the river under a bank 18 m high, she swam there 2000 s). Of the
/// nearest ring with any, the one closest to `toward`.
fn landing(physics: &Physics, at: Vec3, toward: Vec3, tried: &[Vec3]) -> Option<Vec3> {
    let ground = |x: f32, z: f32, water: f32| {
        let top = Vec3::new(x, water + 30.0, z);
        physics
            .cast_ray(top, Vec3::NEG_Y, 60.0)
            .map(|(d, n)| (top.y - d, n))
    };
    LANDING_RINGS.iter().find_map(|&r| {
        (0..24)
            .filter_map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 24.0;
                let (x, z) = (at.x + r * a.sin(), at.z + r * a.cos());
                let water = physics.water_at(x, z).unwrap_or(WATERLINE).max(WATERLINE);
                let (y, n) = ground(x, z, water)?;
                let spot = Vec3::new(x, y, z);
                let rise = spot.y - water;
                // Dry: shallows she still swims in (round 30's AllCon002:
                // 1900 s from one to the next) are no way out.
                if n.y < 0.7
                    || !(LANDING_DRY..=SPOT_RISE).contains(&rise)
                    || tried.iter().any(|t| t.distance(spot) < 4.0)
                {
                    return None;
                }
                // And waded up to from her side, not a lip over deep water
                // (round 32's AllCon002: 2900 tries at banks 1 m up).
                let back = spot + (at - spot).with_y(0.0).normalize_or_zero() * LANDING_WADE;
                let (by, _) = ground(back.x, back.z, water)?;
                (water - by < crate::player::SWIM_STOP * 0.8).then_some(spot)
            })
            .min_by(|a, b| a.distance(toward).total_cmp(&b.distance(toward)))
    })
}

/// Vehicle `vehicles[i]`'s model to world, if placed.
/// How far to the side (her left, +) the nearest vehicle standing still
/// within [`AHEAD_STANDING`] ahead of vehicle `i` (whose place is `m`,
/// going `speed`), or as far as it goes in [`ONCOMING_SECONDS`], and
/// within [`ONCOMING_SIDE`] of her line is, if one is (GurCon003 round 83:
/// her race boat ran into a fishing boat turning on its path at 23 m/s).
fn standing_ahead(game: &Game, i: usize, m: Mat4, speed: f32, toward: Vec3) -> Option<f32> {
    // A boat, slow to stop, looks further, and takes one crawling for
    // standing (GurCon003 rounds 84 and 86: a fishing boat at 1 to 3 m/s
    // on its path, met at 31 m/s).
    let boat = game.scene.vehicles.get(i).is_some_and(|v| v.spec.is_boat());
    let (seconds, still) = if boat {
        (BOAT_AHEAD_SECONDS, CRAWLING)
    } else {
        (ONCOMING_SECONDS, PARKED_SPEED)
    };
    let reach = AHEAD_STANDING.max(speed.abs() * seconds);
    // Along her heading, and along the line she turns onto, towards
    // where she steers (GurCon003 round 88: the fishing boat lay 19 m to
    // her side as she turned for the next gate, into it): (ahead, to her
    // left) of each.
    let here = m.w_axis.truncate().with_y(0.0);
    let forward = m
        .transform_vector3(Vec3::Z)
        .with_y(0.0)
        .normalize_or(Vec3::Z);
    let turning = (toward - here).with_y(0.0).normalize_or(forward);
    let frame = |dir: Vec3, p: Vec3| {
        let d = p.with_y(0.0) - here;
        (d.dot(dir), d.dot(Vec3::new(dir.z, 0.0, -dir.x)))
    };
    let in_way = |(z, x): (f32, f32)| z > 0.0 && z < reach && x.abs() < ONCOMING_SIDE;
    (0..game.scene.vehicles.len())
        .filter(|&j| j != i)
        .filter_map(|j| vehicle_state(game, j))
        .filter(|(_, speed)| speed.abs() <= still)
        .filter_map(|(t, _)| {
            let p = t.w_axis.truncate();
            [frame(turning, p), frame(forward, p)]
                .into_iter()
                .find(|&l| in_way(l))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, x)| x)
}

/// The vehicles other than `i` standing still within `reach` of `pos`.
fn standing_near(game: &Game, i: usize, pos: Vec3, reach: f32) -> Vec<usize> {
    (0..game.scene.vehicles.len())
        .filter(|&j| j != i)
        .filter(|&j| {
            vehicle_state(game, j).is_some_and(|(m, speed)| {
                speed.abs() <= PARKED_SPEED && m.w_axis.truncate().distance(pos) <= reach
            })
        })
        .collect()
}

/// The ground boxes (least and most corner, x and z, with [`AVOID_ROOM`]
/// round them) of the vehicles other than `i` standing still within
/// [`PARKED_NEAR`] of `pos`.
fn parked_near(game: &Game, i: usize, pos: Vec3) -> Vec<(Vec2, Vec2)> {
    (0..game.scene.vehicles.len())
        .filter(|&j| j != i)
        .filter_map(|j| {
            let (m, speed) = vehicle_state(game, j)?;
            let centre = m.w_axis.truncate();
            if speed.abs() > PARKED_SPEED || centre.distance(pos) > PARKED_NEAR {
                return None;
            }
            let (lo, hi) = game.scene.vehicles[j].model.bounds;
            let corners = [
                Vec3::new(lo.x, 0.0, lo.z),
                Vec3::new(lo.x, 0.0, hi.z),
                Vec3::new(hi.x, 0.0, lo.z),
                Vec3::new(hi.x, 0.0, hi.z),
            ]
            .map(|c| {
                let w = m.transform_point3(c);
                Vec2::new(w.x, w.z)
            });
            let least = corners.iter().fold(Vec2::splat(f32::MAX), |a, &c| a.min(c));
            let most = corners.iter().fold(Vec2::splat(f32::MIN), |a, &c| a.max(c));
            Some((
                least - Vec2::splat(AVOID_ROOM),
                most + Vec2::splat(AVOID_ROOM),
            ))
        })
        .collect()
}

/// Whether a vehicle other than `i` (whose place is `m`) going faster
/// than [`PARKED_SPEED`] comes within [`CROSS_CLOSE`] of it in the next
/// [`CROSS_SECONDS`], each going on as it goes, where it would meet it
/// ahead of it.
fn crossing(game: &Game, i: usize, m: Mat4) -> Option<usize> {
    let physics = game.scene.world.as_ref().map(|w| &w.physics)?;
    let mine = game.scene.vehicles.get(i).and_then(|v| v.id)?;
    let here = m.w_axis.truncate().with_y(0.0);
    let v_mine = physics.vehicle_velocity(mine).with_y(0.0);
    let forward = m.transform_vector3(Vec3::Z).with_y(0.0).normalize_or_zero();
    game.scene.vehicles.iter().enumerate().position(|(j, v)| {
        let Some(id) = v.id.filter(|_| j != i) else {
            return false;
        };
        let Some(t) = physics.vehicle_transform(id) else {
            return false;
        };
        let theirs = physics.vehicle_velocity(id).with_y(0.0);
        if theirs.length() < PARKED_SPEED {
            return false;
        }
        let d = t.w_axis.truncate().with_y(0.0) - here;
        let closing = theirs - v_mine;
        let when = (-d.dot(closing) / closing.length_squared().max(1e-3)).clamp(0.0, CROSS_SECONDS);
        let meet = d + closing * when;
        when > 0.0 && meet.length() < CROSS_CLOSE && (d + theirs * when).dot(forward) > 0.0
    })
}

/// Of target `g` (a vehicle or a prop, standing at `at`), a point she can
/// see from `eye`: its middle, else one of its sides or its top, inside
/// its box (PmcCon034 round 10 of its thread: a palm tree hid the middle
/// of the L300 she was to destroy from all her roof). `None` for none.
fn visible_part(game: &Game, g: u32, at: Vec3, eye: Vec3) -> Option<Vec3> {
    let w = game.scene.world.as_ref()?;
    let placed = game
        .scene
        .vehicles
        .iter()
        .position(|v| v.guid == Some(g) && !v.wrecked)
        .and_then(|i| Some((vehicle_at(game, i)?, game.scene.vehicles[i].model.bounds)))
        .or_else(|| game.scene.props.placed_bounds(g));
    let mut points = vec![at];
    if let Some((m, (lo, hi))) = placed {
        let (c, h) = ((lo + hi) * 0.5, (hi - lo) * 0.35);
        for d in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z, Vec3::Y] {
            points.push(m.transform_point3(c + d * h));
        }
    }
    points.into_iter().find(|&p| {
        let (dir, len) = ((p - eye).normalize_or_zero(), p.distance(eye));
        w.physics.cast_ray(eye, dir, (len - 1.5).max(0.0)).is_none()
    })
}

fn vehicle_at(game: &Game, i: usize) -> Option<Mat4> {
    let id = game.scene.vehicles.get(i)?.id?;
    game.scene.world.as_ref()?.physics.vehicle_transform(id)
}

/// And its forward speed (m/s).
fn vehicle_state(game: &Game, i: usize) -> Option<(Mat4, f32)> {
    let id = game.scene.vehicles.get(i)?.id?;
    let physics = &game.scene.world.as_ref()?.physics;
    Some((physics.vehicle_transform(id)?, physics.vehicle_speed(id)))
}

/// How far `at` lies off the heading of a vehicle at `m` (radians,
/// positive to its right), and how far away (metres).
/// Where a vehicle with goods riding on it is, within [`CARGO_CLEAR`] of
/// `at` (on the ground), if one is.
fn cargo_near(game: &Game, at: Vec3) -> Option<Vec3> {
    (0..game.scene.vehicles.len())
        .filter(|&j| game.scene.props.carrying(j))
        .filter_map(|j| vehicle_at(game, j))
        .map(|t| t.w_axis.truncate())
        .find(|c| c.with_y(0.0).distance(at.with_y(0.0)) < CARGO_CLEAR)
}

/// Whether a vehicle other than `i` (whose place is `m`, going `speed`)
/// is ahead of it within [`ONCOMING_AHEAD`] metres, or as far as it goes
/// in [`ONCOMING_SECONDS`], within [`ONCOMING_SIDE`] of its line, heading
/// the other way.
fn oncoming(game: &Game, i: usize, m: Mat4, speed: f32) -> bool {
    let reach = ONCOMING_AHEAD.max(speed.abs() * ONCOMING_SECONDS);
    let to_local = m.inverse();
    let forward = m.transform_vector3(Vec3::Z).with_y(0.0).normalize_or_zero();
    (0..game.scene.vehicles.len())
        .filter(|&j| j != i)
        .filter_map(|j| vehicle_at(game, j))
        .any(|t| {
            let l = to_local.transform_point3(t.w_axis.truncate());
            let theirs = t.transform_vector3(Vec3::Z).with_y(0.0).normalize_or_zero();
            l.z > 0.0 && l.z < reach && l.x.abs() < ONCOMING_SIDE && theirs.dot(forward) < -0.5
        })
}

fn heading_to(m: Mat4, at: Vec3) -> (f32, f32) {
    let f = m
        .transform_vector3(Vec3::Z)
        .with_y(0.0)
        .normalize_or(Vec3::Z);
    // Right-handed, +y up: right is forward turned clockwise from above.
    let right = Vec3::new(-f.z, 0.0, f.x);
    let to = (at - m.w_axis.truncate()).with_y(0.0);
    let d = to.length();
    let dir = to / d.max(1e-3);
    (dir.dot(right).atan2(dir.dot(f)), d)
}

/// The way to steer (radians off the heading, positive to the right)
/// nearest `want` that is clear for a little way, further the faster it
/// goes: rays from both front corners at bumper height, fanned out from
/// `want`. Walls, other vehicles and slopes too steep to drive up are in
/// the way; ground it can drive up isn't.
fn clear_way(
    physics: &Physics,
    id: VehicleId,
    m: Mat4,
    hull: (Vec3, Vec3),
    want: f32,
    speed: f32,
) -> f32 {
    let (lo, hi) = hull;
    let reach = (WAY_AHEAD + speed.abs() * WAY_TIME).min(WAY_MOST);
    let up = m.transform_vector3(Vec3::Y);
    let forward = m.transform_vector3(Vec3::Z);
    let clearance = |a: f32| {
        // Positive is to the right: clockwise from above.
        let dir = Quat::from_axis_angle(up, -a) * forward;
        [lo.x + 0.1, hi.x - 0.1]
            .iter()
            .map(|&x| {
                let from = m.transform_point3(Vec3::new(x, lo.y + 0.8, hi.z));
                match physics.cast_ray_past(from, dir, reach, id) {
                    Some((d, n)) if n.y < WALL => d,
                    _ => reach,
                }
            })
            .fold(reach, f32::min)
    };
    let want = want.clamp(-WAY_FAN, WAY_FAN);
    if clearance(want) >= reach {
        return want;
    }
    let mut best = (f32::NEG_INFINITY, want);
    for k in 1..=10 {
        for side in [-1.0, 1.0] {
            let a = (want + side * k as f32 * 0.15).clamp(-WAY_FAN, WAY_FAN);
            let score = clearance(a) - (a - want).abs() * 4.0;
            if score > best.0 {
                best = (score, a);
            }
        }
    }
    best.1
}

/// Where boat `spec` (body `id`), stuck at `from`, can be put afloat
/// facing `yaw` on its way to `to`: the furthest point along it, or to
/// either side of it, with water [`UNDER_KEEL`] under its keel all over its
/// hull's footprint, at the water's height.
fn afloat_near(
    physics: &Physics,
    spec: &VehicleSpec,
    id: VehicleId,
    from: Vec3,
    to: Vec3,
    yaw: f32,
) -> Option<Vec3> {
    let water = physics.water()?;
    let floor = water - keel_depth(spec) - UNDER_KEEL;
    let flat = (to - from).with_y(0.0);
    let length = flat.length();
    let ahead = flat.normalize_or_zero();
    let side = Vec3::new(ahead.z, 0.0, -ahead.x);
    let turn = Quat::from_rotation_y(yaw);
    let (lo, hi) = spec.hull;
    let footprint = [
        Vec3::ZERO,
        Vec3::new(lo.x, 0.0, lo.z),
        Vec3::new(hi.x, 0.0, lo.z),
        Vec3::new(lo.x, 0.0, hi.z),
        Vec3::new(hi.x, 0.0, hi.z),
    ];
    let open = |p: Vec3| {
        footprint.iter().all(|&q| {
            let at = p + turn * q;
            let top = Vec3::new(at.x, water + OVER_WATER, at.z);
            physics
                .cast_ray_past(top, Vec3::NEG_Y, top.y - floor, id)
                .is_none()
        })
    };
    let steps = (length / AFLOAT_STEP).ceil() as usize;
    (1..=steps)
        .rev()
        .flat_map(|k| {
            let along = from.with_y(water) + ahead * (length * k as f32 / steps as f32);
            AFLOAT_SIDES.map(|s| along + side * s)
        })
        .find(|&p| open(p))
}

/// How deep a boat's keel floats under the water, metres.
fn keel_depth(spec: &VehicleSpec) -> f32 {
    FLOAT_DEPTH - spec.hull.0.y
}

/// Whether something stands within `within` metres of a vehicle's
/// bumper at its height (another parked close ahead, a wall): rays from
/// both front corners and the middle, a little inside the bumper.
pub(crate) fn blocked_ahead(
    physics: &Physics,
    id: VehicleId,
    hull: (Vec3, Vec3),
    within: f32,
) -> bool {
    let Some(m) = physics.vehicle_transform(id) else {
        return false;
    };
    let (lo, hi) = hull;
    let forward = m.transform_vector3(Vec3::Z);
    [lo.x + 0.1, 0.0, hi.x - 0.1].iter().any(|&x| {
        [0.8, 1.5].iter().any(|&h| {
            let from = m.transform_point3(Vec3::new(x, lo.y + h, hi.z - 0.2));
            physics
                .cast_ray_past(from, forward, within + 0.2, id)
                .is_some_and(|(_, n)| n.y < WALL)
        })
    })
}

/// How to steer a vehicle (-1 left to 1 right) to keep on its way round
/// what stands ahead ([`clear_way`]).
pub(crate) fn steer_round(physics: &Physics, id: VehicleId, hull: (Vec3, Vec3)) -> f32 {
    physics.vehicle_transform(id).map_or(0.0, |m| {
        let way = clear_way(physics, id, m, hull, 0.0, physics.vehicle_speed(id));
        (way * 2.0).clamp(-1.0, 1.0)
    })
}

/// Whether the emplaced gun `m` turns and tilts far enough to fire at
/// `at` (its rounds leave clamped to its turn and tilt,
/// `Combat::manned_fire`).
fn covers(m: &crate::combat::Manned, at: Vec3) -> bool {
    let d = launch_dir(m, at);
    m.clamp(d).dot(d) > COVERS
}

/// Where on `target` (its middle at `at`) the emplaced gun `m` fires at:
/// its middle, else just under its top if the gun tilts down only that far
/// (PmcCon031's two columns under its MG platform, past the heavy MG's
/// 20 degrees at their middles, round 38 of its thread); `None` if
/// neither.
fn gun_point(game: &Game, m: &crate::combat::Manned, target: u32, at: Vec3) -> Option<Vec3> {
    if covers(m, at) {
        return Some(at);
    }
    let top = game.scene.props.top(target)? - Vec3::Y * TOP_IN;
    covers(m, top).then_some(top)
}

/// The way the emplaced gun `m`'s round leaves its muzzle for `at`: a
/// lobbed one on the lower of its arcs (as `Combat::manned_fire` fires it;
/// 45 degrees up when out of its reach), else straight at it.
fn launch_dir(m: &crate::combat::Manned, at: Vec3) -> Vec3 {
    let d = at - m.pivot;
    if m.weapon.flight() != m2_engine::weapons::Flight::Lobbed {
        return d.normalize_or_zero();
    }
    let (g, v2) = (
        m2_engine::physics::GRAVITY,
        m.weapon.ammo.velocity.max(1.0).powi(2),
    );
    let flat = d.with_y(0.0);
    let x = flat.length().max(0.1);
    let room = v2 * v2 - g * (g * x * x + 2.0 * d.y * v2);
    let up = if room < 0.0 {
        std::f32::consts::FRAC_PI_4
    } else {
        ((v2 - room.sqrt()) / (g * x)).atan()
    };
    (flat / x * up.cos() + Vec3::Y * up.sin()).normalize_or_zero()
}

/// An angle wrapped to -π..π.
fn wrap(a: f32) -> f32 {
    let mut a = a.rem_euclid(std::f32::consts::TAU);
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    a
}

/// The camera turn (yaw, pitch) towards heading `yaw` and `pitch`, as a
/// quick hand on the mouse turns it in `dt` seconds.
fn turn_to(p: &Player, yaw: f32, pitch: f32, dt: f32) -> Vec2 {
    let most = TURN * dt;
    Vec2::new(
        wrap(yaw - p.yaw).clamp(-most, most),
        (pitch - p.pitch).clamp(-most, most),
    )
}

/// The camera turn putting the crosshair on `target`, and how far off it
/// is now (radians).
fn aim(p: &Player, target: Vec3, dt: f32) -> (Vec2, f32) {
    // Shots follow the crosshair's line from the camera.
    let dir = (target - p.eye()).normalize_or(Vec3::Z);
    let error = p.aim_forward().angle_between(dir);
    // The crosshair sits above the camera's line.
    let lift = p.aim_forward().y.asin() - p.camera_forward().y.asin();
    (
        turn_to(
            p,
            dir.x.atan2(dir.z),
            dir.y.clamp(-1.0, 1.0).asin() - lift,
            dt,
        ),
        error,
    )
}

/// Why a vehicle may be stuck: its speed, how it sits (nose up, rolled),
/// how many wheels touch, what is just ahead of its bumper, how the
/// ground rises ahead and what its body touches (where on the model).
pub(crate) fn stuck_report(
    world: &m2_engine::World,
    id: m2_engine::VehicleId,
    hull: (Vec3, Vec3),
    reversing: bool,
) -> String {
    let physics = &world.physics;
    let Some(m) = physics.vehicle_transform(id) else {
        return "not placed".to_owned();
    };
    let (lo, hi) = hull;
    let forward = m.transform_vector3(Vec3::Z);
    let right = m.transform_vector3(Vec3::X);
    let (down, wheels) = physics.vehicle_contacts(id);
    // Rays from the bumper at three heights, 6 m on.
    let ahead: Vec<String> = [0.3, 0.8, 1.5]
        .iter()
        .map(|&h| {
            let from = m.transform_point3(Vec3::new(0.0, lo.y + h, hi.z + 0.1));
            match physics.cast_ray_keyed(from, forward, 6.0) {
                Some((d, n, key)) => format!(
                    "{h} m up: {d:.1} m to {} (face {:.0} deg from flat)",
                    crate::static_name(world, key),
                    n.y.clamp(-1.0, 1.0).acos().to_degrees()
                ),
                None => format!("{h} m up: clear"),
            }
        })
        .collect();
    // The ground under the middle and 3, 6 and 10 m ahead.
    let ground = |p: Vec3| {
        let top = p + Vec3::Y * 30.0;
        physics
            .cast_ray(top, Vec3::NEG_Y, 80.0)
            .map(|(d, _)| top.y - d)
    };
    let centre = m.transform_point3((lo + hi) * 0.5);
    let flat = forward.with_y(0.0).normalize_or_zero();
    let rises: Vec<String> = [3.0, 6.0, 10.0]
        .iter()
        .map(|&k| match (ground(centre), ground(centre + flat * k)) {
            (Some(a), Some(b)) => format!("{:+.1} m at {k} m", b - a),
            _ => format!("? at {k} m"),
        })
        .collect();
    // What its body touches, where on the model, and the box what it
    // touches fills.
    let to_model = m.inverse();
    let touching: Vec<String> = physics
        .vehicle_touching(id)
        .iter()
        .map(|&(key, at, (lo, hi))| {
            format!(
                "{} at {:.1}, filling {:.1} to {:.1}",
                crate::static_name(world, key),
                to_model.transform_point3(at),
                lo,
                hi
            )
        })
        .collect();
    format!(
        "{:.1} m/s{}, nose {:+.0} deg, rolled {:+.0} deg, {down} of {wheels} wheels down; ahead [{}]; ground [{}]; body touching [{}]",
        physics.vehicle_speed(id),
        if reversing { " backing out" } else { "" },
        forward.y.clamp(-1.0, 1.0).asin().to_degrees(),
        right.y.clamp(-1.0, 1.0).asin().to_degrees(),
        ahead.join("; "),
        rises.join(", "),
        touching.join("; ")
    )
}

/// One of the contract's active deliveries (`MrxTaskObjectiveDeliver`):
/// its destination place (`dest`, at `at`), its drop-off radius and
/// whether flat, how many targets must be there, and its targets with
/// where they are and, where it has a region, whether inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    pub dest: u32,
    pub at: Vec3,
    pub radius: Option<f32>,
    pub xz: bool,
    pub quota: usize,
    pub targets: Vec<(u32, Vec3, Option<bool>)>,
}

impl Delivery {
    /// Whether at least its quota of targets is at the destination: inside
    /// its region where the scripts' region test answers, else within its
    /// radius of its place (flat where it says). With nothing known to
    /// measure by, it counts as there (the old behaviour).
    pub fn arrived(&self) -> bool {
        let Some(radius) = self.radius else {
            if self.targets.iter().all(|t| t.2.is_none()) {
                return true;
            }
            return self.targets.iter().filter(|t| t.2 == Some(true)).count() >= self.quota;
        };
        let there = |&(_, p, inside): &(u32, Vec3, Option<bool>)| match inside {
            Some(i) => i,
            None => self.off(p) <= radius,
        };
        if self.targets.is_empty() {
            return true;
        }
        self.targets.iter().filter(|t| there(t)).count() >= self.quota
    }

    fn off(&self, p: Vec3) -> f32 {
        if self.xz {
            (p - self.at).with_y(0.0).length()
        } else {
            p.distance(self.at)
        }
    }

    /// How far its nearest target is from the place, against its radius.
    pub fn describe(&self) -> String {
        let near = self
            .targets
            .iter()
            .map(|t| self.off(t.1))
            .fold(f32::INFINITY, f32::min);
        match self.radius {
            Some(r) => format!("{near:.1} m (radius {r:.0} m)"),
            None => format!("{near:.1} m"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OilCon020's gun drop: a place and 15 m, flat, 1 pickup of those on
    // the truck. b3i stopped just outside (the crates 15.6 m off); b3c 7 m
    // nearer, inside.
    #[test]
    fn a_delivery_is_there_only_inside_its_radius_or_region() {
        let at = Vec3::new(2510.0, -34.0, 1555.0);
        let gun = |off: f32| (7, at + Vec3::new(0.0, 3.0, -off), None);
        let d = |targets| Delivery {
            dest: 0xfa748,
            at,
            radius: Some(15.0),
            xz: true,
            quota: 1,
            targets,
        };
        // Just outside, entering, inside.
        assert!(!d(vec![gun(15.6)]).arrived());
        assert!(d(vec![gun(15.0)]).arrived());
        assert!(d(vec![gun(7.0)]).arrived());
        // Flat: 3 m up doesn't count.
        assert!(d(vec![gun(14.9)]).arrived());
        // A quota of 2 needs two there.
        let two = Delivery {
            quota: 2,
            ..d(vec![gun(7.0), gun(20.0)])
        };
        assert!(!two.arrived());
        // A region answers for itself, whatever the radius says.
        let region = Delivery {
            radius: None,
            ..d(vec![(7, at + Vec3::new(0.0, 0.0, -40.0), Some(true))])
        };
        assert!(region.arrived());
        let outside = Delivery {
            radius: None,
            ..d(vec![(7, at, Some(false))])
        };
        assert!(!outside.arrived());
    }

    // A box of the carbine's rounds is wanted while it has room for them,
    // as walking over it takes it.
    #[test]
    fn rifle_ammunition_is_wanted_while_there_is_room() {
        use crate::flights::{Leaves, Pickup};
        use m2_engine::TemplateInfo;
        use m2_engine::pickup::{Leavings, PickupInfo};
        use std::collections::HashMap;
        let (gun, round, small) = (0x8000_0010, 0x8000_0011, 0x8000_0012);
        let templates = Templates {
            by_name: HashMap::from([(
                "carbine".to_owned(),
                TemplateInfo {
                    model: Some(0xabc),
                    vehicle: None,
                    engine: None,
                    person: false,
                    human: false,
                },
            )]),
            names: HashMap::from([
                (gun, "Carbine".to_owned()),
                (round, "Carbine Round".to_owned()),
                (small, "Ammo Pickup (Small)".to_owned()),
            ]),
            leavings: Leavings {
                pickups: HashMap::from([(
                    small,
                    PickupInfo {
                        kind: 1,
                        radius: 1.0,
                        ammo: Some(round),
                        rounds: 3,
                        gives: vec![(round, 3)],
                        walked: vec![(round, 3)],
                        heals: 0,
                        cue: None,
                        effect: None,
                    },
                )]),
                // The carbine fires the round, by its model.
                ammo: HashMap::from([(gun, round)]),
                ..Leavings::default()
            },
            ..Templates::default()
        };
        let mut combat = Combat::default();
        combat.carbine.template = Some(gun);
        combat.carbine_only();
        let p = Pickup {
            from: 0x4000_0001,
            what: Leaves {
                template: small,
                ..Leaves::default()
            },
            at: Vec3::ZERO,
            yaw: 0.0,
        };
        assert!(combat.rifle_rounds.stored < combat.rifle.spare);
        assert!(wanted(&templates, &combat, &p));
        combat.rifle_rounds.stored = combat.rifle.spare;
        assert!(!wanted(&templates, &combat, &p));
        // Nothing of hers takes it: not wanted.
        assert!(!wanted(&Templates::default(), &Combat::default(), &p));
    }

    #[test]
    fn a_target_behind_a_wall_is_seen_from_round_it() {
        use m2_engine::{CollisionShape, StaticKey};
        use m2_math::glam::Quat;
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let ground = quad(
            Vec3::new(-50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, 50.0),
            Vec3::new(-50.0, 0.0, 50.0),
        );
        // A wall 3 m high across her way, 2 m ahead.
        let wall = quad(
            Vec3::new(-10.0, 0.0, 2.0),
            Vec3::new(10.0, 0.0, 2.0),
            Vec3::new(10.0, 3.0, 2.0),
            Vec3::new(-10.0, 3.0, 2.0),
        );
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        p.add_static(StaticKey::Cell(2), &wall, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        let target = Vec3::new(0.0, 1.3, 4.0);
        let spot = vantage(&p, target, Vec3::ZERO, &[], &VANTAGE_RINGS, 1.0)
            .expect("somewhere to see it from");
        assert!(spot.z > 2.0, "still behind the wall at {spot}");
        assert!(spot.y.abs() < 0.1, "not on the ground: {spot}");
        // Not the same spot twice.
        let next = vantage(&p, target, Vec3::ZERO, &[spot], &VANTAGE_RINGS, 1.0).expect("another");
        assert!(next.distance(spot) >= 4.0, "{next} is {spot} again");
    }

    // PmcCon034: on a roof 20 m up, the target 60 m off on the ground is
    // hidden by the roof's own edge from its middle; she shoots from by
    // the edge, on the roof, unless that is out of the scripts' bounds.
    #[test]
    fn a_sniper_on_a_roof_shoots_from_by_its_edge_within_bounds() {
        use m2_engine::{CollisionShape, StaticKey};
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let square = |half: f32, y: f32| {
            quad(
                Vec3::new(-half, y, -half),
                Vec3::new(half, y, -half),
                Vec3::new(half, y, half),
                Vec3::new(-half, y, half),
            )
        };
        p.add_static(
            StaticKey::Cell(1),
            &square(100.0, 0.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        p.add_static(
            StaticKey::Cell(2),
            &square(10.0, 20.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        p.step(1.0 / 60.0);
        let (from, target) = (Vec3::new(0.0, 20.0, 0.0), Vec3::new(0.0, 1.0, 60.0));
        let eye = from + Vec3::Y * 1.6;
        let dir = (target - eye).normalize();
        assert!(
            p.cast_ray(eye, dir, eye.distance(target) - 1.5).is_some(),
            "the roof hides it from its middle"
        );
        let spot = perch(&p, target, from, &[], &[]).expect("somewhere on the roof");
        assert!(spot.z > 4.0 && spot.z < 10.0, "not by the edge: {spot}");
        assert!((spot.y - 20.0).abs() < 0.1, "not on the roof: {spot}");
        // Bounds that end short of where it can be seen from: nowhere.
        let keep = Keep {
            region: m2_assets::region::Region {
                flags: 0,
                unk_0x04: 0,
                corners: vec![[-20.0, -20.0], [20.0, -20.0], [20.0, 4.0], [-20.0, 4.0]],
            },
            centre: [0.0, 0.0],
            yaw: 0.0,
        };
        assert!(keep.holds(from) && !keep.holds(spot));
        assert_eq!(perch(&p, target, from, &[], &[keep]), None);
    }

    // Ground under the sea (round 6's riverbed): no spot to fire from.
    #[test]
    fn no_spot_is_under_the_water() {
        use m2_engine::{CollisionShape, StaticKey};
        let mut p = Physics::new();
        let y = WATERLINE - 4.0;
        let bed = CollisionShape::mesh(
            &[
                Vec3::new(-60.0, y, -60.0),
                Vec3::new(60.0, y, -60.0),
                Vec3::new(60.0, y, 60.0),
                Vec3::new(-60.0, y, 60.0),
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
        p.add_static(StaticKey::Cell(1), &bed, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        let target = Vec3::new(0.0, y + 1.0, 0.0);
        assert_eq!(
            vantage(&p, target, target, &[], &BLAST_RINGS, BLAST_BODY),
            None
        );
    }

    // In a river with a bank 18 m high close by and a beach further off
    // (round 27's AllCon002): out at the beach, not under the bank.
    #[test]
    fn out_of_the_water_where_she_can_climb() {
        use m2_engine::{CollisionShape, StaticKey};
        let mut p = Physics::new();
        let flat = |x0: f32, x1: f32, y: f32| {
            CollisionShape::mesh(
                &[
                    Vec3::new(x0, y, -80.0),
                    Vec3::new(x1, y, -80.0),
                    Vec3::new(x1, y, 80.0),
                    Vec3::new(x0, y, 80.0),
                ],
                &[[0, 1, 2], [0, 2, 3]],
            )
            .unwrap()
        };
        let w = WATERLINE;
        p.add_static(
            StaticKey::Cell(1),
            &flat(-30.0, 0.0, w - 4.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        // Shallows nearer than the beach: still in the water.
        p.add_static(
            StaticKey::Cell(4),
            &flat(0.0, 20.0, w - 0.3),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        p.add_static(
            StaticKey::Cell(2),
            &flat(20.0, 80.0, w + 18.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        p.add_static(
            StaticKey::Cell(3),
            &flat(-80.0, -30.0, w + 0.5),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        p.step(1.0 / 60.0);
        let here = Vec3::new(10.0, w - 0.3, 0.0);
        let out = landing(&p, here, Vec3::new(30.0, w + 18.0, 0.0), &[]).expect("a way out");
        assert!(out.x < -30.0, "not on the beach: {out}");
        assert!((out.y - (w + 0.5)).abs() < 0.1, "{out}");
        // No beach at all: nowhere.
        let mut q = Physics::new();
        q.add_static(
            StaticKey::Cell(1),
            &flat(-300.0, 20.0, w - 4.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        q.add_static(
            StaticKey::Cell(2),
            &flat(20.0, 300.0, w + 18.0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        q.step(1.0 / 60.0);
        assert_eq!(landing(&q, here, here, &[]), None);
    }

    // A truck-sized box round what she is blowing up (round 3's MLRS):
    // every line to its middle meets its own sides metres short, so only
    // stopping BLAST_BODY short finds her a spot to fire from.
    #[test]
    fn blast_spots_see_past_the_targets_own_body() {
        use m2_engine::{CollisionShape, StaticKey};
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let ground = quad(
            Vec3::new(-60.0, 0.0, -60.0),
            Vec3::new(60.0, 0.0, -60.0),
            Vec3::new(60.0, 0.0, 60.0),
            Vec3::new(-60.0, 0.0, 60.0),
        );
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        // The box's four sides, 8 m by 3 m, 3 m high.
        let (x, z, h) = (4.0, 1.5, 3.0);
        let corners = [(-x, -z), (x, -z), (x, z), (-x, z)];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let side = quad(
                Vec3::new(a.0, 0.0, a.1),
                Vec3::new(b.0, 0.0, b.1),
                Vec3::new(b.0, h, b.1),
                Vec3::new(a.0, h, a.1),
            );
            p.add_static(
                StaticKey::Cell(2 + k as u32),
                &side,
                Vec3::ZERO,
                Quat::IDENTITY,
            );
        }
        p.step(1.0 / 60.0);
        let target = Vec3::new(0.0, 1.3, 0.0);
        let from = Vec3::new(30.0, 0.0, 0.0);
        assert_eq!(vantage(&p, target, from, &[], &BLAST_RINGS, 1.0), None);
        let spot = vantage(&p, target, from, &[], &BLAST_RINGS, BLAST_BODY)
            .expect("somewhere to fire from");
        let d = spot.with_y(0.0).distance(target.with_y(0.0));
        assert!(
            (BLAST_RANGE.0..BLAST_RANGE.1).contains(&d),
            "{spot} is {d:.1} m off"
        );
    }

    // A wall across the way with a gap to its left: the driver steers
    // for the gap, and straight on once the way is clear.
    #[test]
    fn drivers_steer_round_a_wall() {
        use m2_engine::vehicle::WheelSpec;
        use m2_engine::{CollisionShape, StaticKey, VehicleSpec};
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let ground = quad(
            Vec3::new(-50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, 50.0),
            Vec3::new(-50.0, 0.0, 50.0),
        );
        // Facing +z, left is +x: the wall ends 4 m to the left.
        let wall = quad(
            Vec3::new(-30.0, 0.0, 10.0),
            Vec3::new(4.0, 0.0, 10.0),
            Vec3::new(4.0, 3.0, 10.0),
            Vec3::new(-30.0, 3.0, 10.0),
        );
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        p.add_static(StaticKey::Cell(2), &wall, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.35, z),
            radius: 0.35,
            width: 0.25,
            steers: z > 0.0,
        };
        let hull = (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2));
        let spec = VehicleSpec {
            model: 1,
            kind: "car".into(),
            label: "test".into(),
            node_names: Vec::new(),
            armed: Vec::new(),
            wheels: vec![
                wheel(-0.9, 1.4),
                wheel(0.9, 1.4),
                wheel(-0.9, -1.3),
                wheel(0.9, -1.3),
            ],
            bounds: hull,
            seat: None,
            door: None,
            tracked: false,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            node_rolls: Vec::new(),
            mass: 1400.0,
            turret: Vec::new(),
            pivot: None,
            muzzle: None,
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull,
            turret_box: None,
            rotors: Vec::new(),
            tilts: Vec::new(),
            bomb_bay: None,
            damage: None,
            tuning: Default::default(),
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, 0.0), 0.0);
        p.step(1.0 / 60.0);
        let m = p.vehicle_transform(id).unwrap();
        let target = Vec3::new(0.0, 0.0, 40.0);
        let (want, _) = heading_to(m, target);
        let way = clear_way(&p, id, m, hull, want, 5.0);
        assert!(way < -0.2, "steers {way} (left is negative)");
        // Past the wall's end: straight on.
        let m = Mat4::from_translation(Vec3::new(8.0, 0.0, 0.0));
        p.remove_vehicle(id);
        let id = p.add_vehicle(&spec, Vec3::new(8.0, 0.1, 0.0), 0.0);
        p.step(1.0 / 60.0);
        let (want, _) = heading_to(m, Vec3::new(8.0, 0.0, 40.0));
        assert_eq!(clear_way(&p, id, m, hull, want, 5.0), want);
    }

    #[test]
    fn a_place_to_the_right_steers_right() {
        // Facing +z, right is -x.
        let (angle, d) = heading_to(Mat4::IDENTITY, Vec3::new(-10.0, 0.0, 10.0));
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "{angle}"
        );
        assert!((d - 200f32.sqrt()).abs() < 1e-4);
        let (angle, _) = heading_to(Mat4::IDENTITY, Vec3::new(10.0, 0.0, 0.0));
        assert!(
            (angle + std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "{angle}"
        );
    }

    #[test]
    fn turns_take_the_short_way_round() {
        assert!(
            (wrap(3.0 * std::f32::consts::FRAC_PI_2) + std::f32::consts::FRAC_PI_2).abs() < 1e-5
        );
        assert!((wrap(-0.5) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_crosshair_comes_onto_a_target() {
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.pitch = 0.0;
        let target = p.pivot() + Vec3::new(5.0, 0.0, 5.0);
        let mut error = f32::MAX;
        for _ in 0..120 {
            let (look, e) = aim(&p, target, 1.0 / 60.0);
            p.yaw += look.x;
            p.pitch += look.y;
            error = e;
        }
        assert!(error < 1e-3, "{error}");
        assert!(
            (p.yaw - std::f32::consts::FRAC_PI_4).abs() < 1e-3,
            "{}",
            p.yaw
        );
    }

    /// An emplaced gun at the origin facing +z, turning +-90 and tilting
    /// -20..30 degrees (PmcCon031's, from its log), firing rounds at
    /// `speed` that blow up and fall as `gravity` says.
    fn gun(speed: f32, gravity: f32) -> crate::combat::Manned {
        use m2_engine::weapons::{Ammo, Blast, WeaponSpec};
        let blast = Blast {
            template: 1,
            radius: 4.0,
            damage: 100.0,
            min_damage: 10.0,
            max_force: 0.0,
            key: None,
        };
        crate::combat::Manned {
            entity: 1,
            template: 2,
            pivot: Vec3::ZERO,
            facing: 0.0,
            yaw: (-90f32.to_radians(), 90f32.to_radians()),
            pitch: (-20f32.to_radians(), 30f32.to_radians()),
            weapon: WeaponSpec {
                template: 3,
                name: "Mounted Grenade Launcher".into(),
                model: None,
                class: None,
                semi: false,
                rate_of_fire: 200.0,
                clip: 50,
                reserve_clips: 0,
                pellets: 1,
                rounds_per_reload: 0,
                scatter: 0.0,
                ammo: Ammo {
                    template: 4,
                    velocity: speed,
                    max_velocity: 0.0,
                    human_boost: 1.0,
                    gravity,
                    damage: 0.0,
                    damage_min: 0.0,
                    falloff: (0.0, 0.0),
                    dropoff: 0.0,
                    hero: 1.0,
                    key: None,
                    blast: Some(blast),
                    timer: None,
                    timer_blast: None,
                    homing: None,
                    model: None,
                    sticky: false,
                },
                lock_on: None,
                pickup: None,
                feel: Default::default(),
                hint: None,
                ui: None,
                assist: None,
                camera: None,
                scope: None,
            },
            rounds: 50,
        }
    }

    #[test]
    fn a_gun_covers_only_what_it_turns_and_tilts_to() {
        let rocket = gun(100.0, 0.0);
        // Ahead and level, and 80 degrees round: covered.
        assert!(covers(&rocket, Vec3::new(0.0, 0.0, 50.0)));
        assert!(covers(&rocket, Vec3::new(49.0, 0.0, 8.6)));
        // Behind it: not.
        assert!(!covers(&rocket, Vec3::new(0.0, 0.0, -50.0)));
        // A column's middle 12 m off and 4.9 m down is past its 20 degrees
        // down; one 2 m down is not (PmcCon031's MG platform).
        assert!(!covers(&rocket, Vec3::new(0.0, -4.9, 12.0)));
        assert!(covers(&rocket, Vec3::new(0.0, -2.0, 12.0)));
    }

    #[test]
    fn a_lobbed_round_leaves_the_muzzle_above_its_target() {
        // A grenade at 40 m/s for a column 60 m off and 4 m down: up, on
        // the lower arc, and covered though the column is below the gun.
        let gl = gun(40.0, 1.0);
        let at = Vec3::new(0.0, -4.0, 60.0);
        let d = launch_dir(&gl, at);
        assert!(d.y > 0.0, "{d}");
        assert!(d.y < 45f32.to_radians().sin(), "{d}");
        assert!(covers(&gl, at));
        // Out of its reach: 45 degrees up.
        let far = launch_dir(&gl, Vec3::new(0.0, 0.0, 1000.0));
        assert!((far.y - 45f32.to_radians().sin()).abs() < 1e-4, "{far}");
        // A rocket goes straight at it.
        let rocket = gun(100.0, 0.0);
        assert!((launch_dir(&rocket, at) - at.normalize()).length() < 1e-5);
    }
}

//! The scripts' camera: the camera a scene of theirs (cinematic mode)
//! looks through, as the 360's script camera controller keeps it.
//!
//! From the 360 code (class vtable 0x820f02e8, size 0x690; message
//! handler 0x824c73b8, update 0x824c75f0, constructor 0x824c7260):
//!
//! - An eye: an object (+0x624), its bone (+0x628) and a point
//!   (+0x618). `Camera.SetPosition` sets all three.
//! - A look: an object (+0x60c), its bone (+0x610) and a point (+0x600).
//!   With "along" (+0x614) the point is a direction instead.
//!   `Camera.SetLookAt` sets them, and `Camera.Follow` puts both eye and
//!   look on one object with "along" on.
//! - Two holds (+0x62c eye, +0x62d look) that stop either following its
//!   object (`Camera.Hold`).
//! - A blend (+0x630 elapsed, +0x634 duration, +0x638 the transform it
//!   starts from: `Camera.Blend`).
//!
//! Each update ([`ScriptCamera::update`]):
//! - The eye follows its object and the look its object, unless held.
//! - Only when one of them moved is the view rebuilt: from the eye towards
//!   the look point (or along the direction), with the world's up.
//! - While a blend runs, the view is moved from its start towards that new
//!   view: position by straight interpolation, turn by spherical, at
//!   t = elapsed / duration with no easing. The elapsed time grows only on
//!   those frames, and the blend ends once it reaches the duration.
//! - The engine zeroes the elapsed time only when the camera is made, so a
//!   second `Blend` on the same camera starts at once past its end and
//!   shows no blend. That is kept here.
//!
//! The camera the cinematic state makes (0x824453c0) is bound to the
//! player's character, eye and look, with "along" on. That is
//! [`ScriptCamera::on`].
//!
//! Stand-ins (inferred): the scripts' world holds no bone transforms, so a
//! bone stands at its object's place, and "along" looks the way the object
//! faces. A look at an object with no bone is raised by its box height
//! times 0.85 for people and 0.5 for anything else (from the 360 code).
//! The box isn't held here, so people are taken as [`PERSON_HEIGHT`] tall
//! (invented) and other objects as flat.

use m2_math::glam::{Mat3, Quat, Vec3};

/// The share of a person's height a look at them is raised by (0.85 in
/// the 360 code); others are raised by [`OTHER_LOOK_RAISE`] of theirs.
pub const PERSON_LOOK_RAISE: f32 = 0.85;
pub const OTHER_LOOK_RAISE: f32 = 0.5;

/// How tall a person is taken to be, metres (invented: their bounding
/// boxes aren't held by the scripts' world).
pub const PERSON_HEIGHT: f32 = 1.8;

/// Where an object stands, as the camera needs it: its place, the way it
/// faces (radians clockwise from +z) and its box's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: Vec3,
    pub yaw: f32,
    pub height: f32,
    pub person: bool,
}

/// The camera's view: where it is and how it is turned (its forward is
/// the turn applied to +z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub eye: Vec3,
    pub turn: Quat,
}

impl View {
    /// Looking from `eye` along `dir`, with the world's up.
    pub fn looking(eye: Vec3, dir: Vec3) -> Self {
        let f = dir.normalize_or(Vec3::Z);
        let right = Vec3::Y.cross(f).normalize_or(Vec3::X);
        let up = f.cross(right);
        Self {
            eye,
            turn: Quat::from_mat3(&Mat3::from_cols(right, up, f)),
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.turn * Vec3::Z
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            eye: Vec3::ZERO,
            turn: Quat::IDENTITY,
        }
    }
}

/// One end of the camera: an object (0 for none), its bone (0 for none)
/// and a point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub object: u32,
    pub bone: u32,
    pub point: Vec3,
}

/// The scripts' camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptCamera {
    pub view: View,
    pub eye: Anchor,
    pub look: Anchor,
    /// The look point is a direction.
    pub along: bool,
    pub hold_eye: bool,
    pub hold_look: bool,
    pub elapsed: f32,
    pub duration: f32,
    pub start: View,
}

impl ScriptCamera {
    /// The camera the cinematic state makes: eye and look on `object` and
    /// `bone`, along it (0x824c7260).
    pub fn on(object: u32, bone: u32) -> Self {
        let at = Anchor {
            object,
            bone,
            point: Vec3::ZERO,
        };
        Self {
            eye: at,
            look: at,
            along: true,
            ..Self::default()
        }
    }

    /// `Camera.SetPosition`'s message (0x7953668e): the eye's object, bone
    /// and point.
    pub fn set_position(&mut self, eye: Anchor) {
        self.eye = eye;
    }

    /// `Camera.SetLookAt`'s message (0x54e7bfed): the look's object, bone
    /// and point; `along` when its number is above 0.
    pub fn set_look_at(&mut self, look: Anchor, along: bool) {
        self.look = look;
        self.along = along;
    }

    /// `Camera.Follow`'s message (0x4580eaae): eye and look on one object
    /// and bone, along it.
    pub fn follow(&mut self, object: u32, bone: u32) {
        self.eye.object = object;
        self.eye.bone = bone;
        self.look.object = object;
        self.look.bone = bone;
        self.along = true;
    }

    /// `Camera.Hold`'s message (0x2125d0d6).
    pub fn hold(&mut self, eye: bool, look: bool) {
        self.hold_eye = eye;
        self.hold_look = look;
    }

    /// `Camera.Blend`'s message (0x6b5a3cf6): blend for `seconds` from the
    /// view as it is now. The elapsed time is left as it is.
    pub fn blend(&mut self, seconds: f32) {
        self.duration = seconds;
        self.start = self.view;
    }

    /// One update, `dt` seconds on, finding objects with `place`. Returns
    /// whether the view was rebuilt.
    pub fn update(&mut self, dt: f32, place: &dyn Fn(u32) -> Option<Placed>) -> bool {
        let looked = self.track_look(place);
        let placed = self.track_eye(place);
        if !(looked || placed) {
            return false;
        }
        let dir = if self.along {
            self.look.point
        } else {
            self.look.point - self.eye.point
        };
        let target = View::looking(self.eye.point, dir);
        self.view = target;
        if self.duration > 0.0 {
            let t = self.elapsed / self.duration;
            self.view = View {
                eye: self.start.eye.lerp(target.eye, t),
                turn: self.start.turn.slerp(target.turn, t),
            };
            self.elapsed += dt;
            if self.elapsed >= self.duration {
                self.duration = 0.0;
            }
        }
        true
    }

    /// The look follows its object (0x824c77e8).
    fn track_look(&mut self, place: &dyn Fn(u32) -> Option<Placed>) -> bool {
        if self.hold_look || self.look.object == 0 {
            return false;
        }
        let Some(p) = place(self.look.object) else {
            return false;
        };
        let facing = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
        self.look.point = if self.along {
            facing
        } else if self.look.bone != 0 {
            p.position
        } else {
            let share = if p.person {
                PERSON_LOOK_RAISE
            } else {
                OTHER_LOOK_RAISE
            };
            p.position + Vec3::Y * (p.height * share)
        };
        true
    }

    /// The eye follows its object (0x824c7a28).
    fn track_eye(&mut self, place: &dyn Fn(u32) -> Option<Placed>) -> bool {
        if self.hold_eye || self.eye.object == 0 {
            return false;
        }
        match place(self.eye.object) {
            Some(p) => {
                self.eye.point = p.position;
                true
            }
            None => false,
        }
    }
}

/// The bone the cinematic state's camera is put on (0x824453c0); which
/// bone it is isn't known.
pub const CINEMATIC_BONE: u32 = 0x31db_4f50;

/// The scripts' camera's place in the scripts' world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cinematic {
    pub camera: ScriptCamera,
    /// Whether a script has addressed the camera (set it, followed,
    /// looked, held or blended). Until then the game keeps showing its
    /// own camera: the default camera's bone isn't known (see the module
    /// notes), so its view would stand at her feet (inferred choice).
    pub directed: bool,
}

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

impl ScriptWorld {
    /// The cinematic state starts or ends: the scripts' camera is made on
    /// the player's character, or let go.
    pub fn cinematic_camera(&mut self, on: bool) {
        self.script_camera = match (on, self.player) {
            (true, Some(p)) => Some(Cinematic {
                camera: ScriptCamera::on(p, CINEMATIC_BONE),
                directed: false,
            }),
            _ => None,
        };
    }

    /// Moves the scripts' camera on by `dt` seconds, its objects where the
    /// scripts' world has them.
    pub fn update_script_camera(&mut self, dt: f32) {
        let Some(mut c) = self.script_camera.take() else {
            return;
        };
        let place = |g: u32| {
            self.objects.get(&g).map(|o| {
                let person = o.player_controlled
                    || o.labels.iter().any(|l| {
                        ["human", "soldier", "civilian"]
                            .iter()
                            .any(|k| l.eq_ignore_ascii_case(k))
                    });
                Placed {
                    position: Vec3::from(o.position),
                    yaw: o.yaw,
                    height: if person { PERSON_HEIGHT } else { 0.0 },
                    person,
                }
            })
        };
        c.camera.update(dt, &place);
        self.script_camera = Some(c);
    }

    /// The scripts' camera's view while a script directs it.
    pub fn script_view(&self) -> Option<View> {
        self.script_camera
            .as_ref()
            .filter(|c| c.directed)
            .map(|c| c.camera.view)
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let camera = lib("Camera")?;
    add(&camera, "SetPosition", set_position)?;
    add(&camera, "SetLookAt", set_look_at)?;
    add(&camera, "Follow", follow)?;
    add(&camera, "Hold", hold)?;
    add(&camera, "Blend", blend)?;
    add(&camera, "SetPitch", answered)?;
    add(&camera, "StopBlending", answered)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn boolean(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn hash(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Value::Integer(i) => Some(*i as u32),
        Value::Number(n) if n.is_finite() => Some(*n as i64 as u32),
        _ => None,
    }
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// The camera a handle names: the game's one camera, named by the player
/// (`Player.GetCamera`). `None` is no handle (the natives answer nil).
fn handle(w: &ScriptWorld, a: &[Value]) -> Option<bool> {
    let g = guid(a.first())?;
    Some(Some(g) == w.player)
}

/// A point at `a[i]` (three numbers) and the index after it, or the origin
/// and `i` when there is none, as the 360's vector read leaves it.
fn point(a: &[Value], i: usize) -> (Vec3, usize) {
    match (number(a.get(i)), number(a.get(i + 1)), number(a.get(i + 2))) {
        (Some(x), Some(y), Some(z)) => (Vec3::new(x, y, z), i + 3),
        _ => (Vec3::ZERO, i),
    }
}

/// Runs `f` on the scripts' camera when the handle names the camera and a
/// scene has it: true; false when there is none (the 360 asks the
/// viewport's controller to be the script camera, kind 0xd); nil without
/// a handle.
fn direct(
    w: &mut ScriptWorld,
    a: &[Value],
    f: impl FnOnce(&mut ScriptCamera),
) -> mlua::Result<MultiValue> {
    let Some(named) = handle(w, a) else {
        return one(Value::Nil);
    };
    match w.script_camera.as_mut().filter(|_| named) {
        Some(c) => {
            f(&mut c.camera);
            c.directed = true;
            one(Value::Boolean(true))
        }
        None => one(Value::Boolean(false)),
    }
}

/// An anchor at `a[1]` on: a point, then an object and a bone, and the
/// index after them.
fn anchor(a: &[Value]) -> (Anchor, usize) {
    let (p, mut i) = point(a, 1);
    let object = guid(a.get(i)).unwrap_or(0);
    if guid(a.get(i)).is_some() {
        i += 1;
    }
    let bone = hash(a.get(i)).unwrap_or(0);
    if hash(a.get(i)).is_some() {
        i += 1;
    }
    (
        Anchor {
            object,
            bone,
            point: p,
        },
        i,
    )
}

/// `Camera.SetPosition(camera, [x, y, z], [object], [bone])` (0x82317118).
fn set_position(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (eye, _) = anchor(a);
    direct(w, a, |c| c.set_position(eye))
}

/// `Camera.SetLookAt(camera, [x, y, z], [object], [bone], [along])`
/// (0x823172e0).
fn set_look_at(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (look, i) = anchor(a);
    let along = boolean(a.get(i)).unwrap_or(false);
    direct(w, a, |c| c.set_look_at(look, along))
}

/// `Camera.Follow(camera, object, [bone])` (0x823176b0): nil without an
/// object.
fn follow(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(object) = guid(a.get(1)) else {
        return one(Value::Nil);
    };
    let bone = hash(a.get(2)).unwrap_or(0);
    direct(w, a, |c| c.follow(object, bone))
}

/// `Camera.Hold(camera, [eye], [look])` (0x823174f0): eye true unless
/// given, look as eye unless given.
fn hold(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let eye = boolean(a.get(1)).unwrap_or(true);
    let look = boolean(a.get(2)).unwrap_or(eye);
    direct(w, a, |c| c.hold(eye, look))
}

/// `Camera.Blend(camera, seconds)` (0x82317828): nil without seconds.
fn blend(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(s) = number(a.get(1)) else {
        return one(Value::Nil);
    };
    direct(w, a, |c| c.blend(s))
}

/// `Camera.SetPitch(camera, degrees)` and `Camera.StopBlending(camera)`
/// (0x82316d10, 0x82316da8 through 0x82316a00): true when the handle names
/// a camera, whatever drives it; nil without one. What they do to the
/// view (the pitch message's turn, the base camera's transition blend) is
/// not ported.
fn answered(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match handle(w, a) {
        Some(named) => one(Value::Boolean(named)),
        None => one(Value::Nil),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: Vec3, yaw: f32) -> Placed {
        Placed {
            position,
            yaw,
            height: PERSON_HEIGHT,
            person: true,
        }
    }

    #[test]
    fn it_looks_from_its_eye_to_its_target_raised_by_its_height() {
        let mut c = ScriptCamera::default();
        c.set_position(Anchor {
            object: 1,
            ..Anchor::default()
        });
        c.set_look_at(
            Anchor {
                object: 2,
                ..Anchor::default()
            },
            false,
        );
        let place = |g: u32| match g {
            1 => Some(at(Vec3::new(0.0, 0.0, 0.0), 0.0)),
            2 => Some(at(Vec3::new(0.0, 0.0, 10.0), 0.0)),
            _ => None,
        };
        assert!(c.update(0.1, &place));
        assert_eq!(c.view.eye, Vec3::ZERO);
        let want = Vec3::new(0.0, PERSON_HEIGHT * PERSON_LOOK_RAISE, 10.0).normalize();
        assert!((c.view.forward() - want).length() < 1e-5);
    }

    #[test]
    fn following_looks_the_way_the_object_faces_and_holds_stop_it() {
        let mut c = ScriptCamera::on(1, 0x31db_4f50);
        let yaw = std::f32::consts::FRAC_PI_2;
        let place = |g: u32| (g == 1).then(|| at(Vec3::new(3.0, 1.0, 2.0), yaw));
        assert!(c.update(0.1, &place));
        assert_eq!(c.view.eye, Vec3::new(3.0, 1.0, 2.0));
        assert!((c.view.forward() - Vec3::X).length() < 1e-5);
        // Held both ways: nothing is rebuilt.
        c.hold(true, true);
        let moved = |g: u32| (g == 1).then(|| at(Vec3::new(9.0, 1.0, 2.0), 0.0));
        assert!(!c.update(0.1, &moved));
        assert_eq!(c.view.eye, Vec3::new(3.0, 1.0, 2.0));
    }

    /// The natives as a scene's script calls them.
    #[test]
    fn the_natives_direct_the_camera_only_in_a_scene() {
        use crate::api::{Object, Shared, install};
        use crate::script::ScriptHost;
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            let mut hero = Object::new("Mattias", [0.0, 0.0, 0.0]);
            hero.player_controlled = true;
            w.insert(1, hero);
            w.player = Some(1);
            w.insert(2, Object::new("Fiona", [0.0, 0.0, 10.0]));
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        let cam = "Player.GetCamera(Player.GetLocalPlayer())";
        // No scene: false, as the gameplay camera isn't the script one.
        assert_eq!(
            got(&format!("Camera.Blend({cam}, 1)")),
            Value::Boolean(false)
        );
        assert_eq!(got("Camera.Blend(nil, 1)"), Value::Nil);
        assert_eq!(got(&format!("Camera.Blend({cam})")), Value::Nil);
        lua.load("Player.SetCinematicMode(true)").exec().unwrap();
        assert!(world.borrow().script_view().is_none());
        // From a point 5 m up, at Fiona.
        assert_eq!(
            got(&format!("Camera.SetPosition({cam}, 0, 5, 0)")),
            Value::Boolean(true)
        );
        assert_eq!(
            got(&format!(
                "Camera.SetLookAt({cam}, 0, 0, 0, Pg.GetGuidByName('Fiona'))"
            )),
            Value::Boolean(true)
        );
        // A point with no object: the eye stays where it was put (the
        // message clears the object), and the look follows Fiona.
        assert_eq!(
            world
                .borrow()
                .script_camera
                .as_ref()
                .unwrap()
                .camera
                .eye
                .object,
            0
        );
        world.borrow_mut().update_script_camera(0.1);
        let v = world.borrow().script_view().unwrap();
        assert_eq!(v.eye, Vec3::new(0.0, 5.0, 0.0));
        assert!((v.forward() - Vec3::new(0.0, -5.0, 10.0).normalize()).length() < 1e-5);
        // Hold's defaults, then the end of the scene lets it go.
        assert_eq!(got(&format!("Camera.Hold({cam})")), Value::Boolean(true));
        {
            let w = world.borrow();
            let c = &w.script_camera.as_ref().unwrap().camera;
            assert!(c.hold_eye && c.hold_look);
        }
        assert_eq!(
            got(&format!("Camera.SetPitch({cam}, 10)")),
            Value::Boolean(true)
        );
        assert_eq!(
            got(&format!("Camera.StopBlending({cam})")),
            Value::Boolean(true)
        );
        lua.load("Player.SetCinematicMode(false)").exec().unwrap();
        assert!(world.borrow().script_camera.is_none());
    }

    #[test]
    fn a_blend_runs_linearly_and_a_second_one_shows_nothing() {
        let mut c = ScriptCamera::on(1, 0);
        let p = std::cell::Cell::new(Vec3::ZERO);
        let place = |g: u32| (g == 1).then(|| at(p.get(), 0.0));
        c.update(0.0, &place);
        // Blend 1 s to a point 10 m on.
        c.blend(1.0);
        p.set(Vec3::new(10.0, 0.0, 0.0));
        for (frame, want) in [(0, 0.0), (1, 2.5), (2, 5.0), (3, 7.5)] {
            c.update(0.25, &place);
            assert!(
                (c.view.eye.x - want).abs() < 1e-4,
                "frame {frame}: {}",
                c.view.eye.x
            );
        }
        c.update(0.25, &place);
        assert_eq!(c.duration, 0.0);
        c.update(0.25, &place);
        assert_eq!(c.view.eye.x, 10.0);
        // A second blend: elapsed is still 1 s from the first, so its first
        // frame is already at its end (t = 1) and it ends at once.
        assert_eq!(c.elapsed, 1.0);
        c.blend(1.0);
        p.set(Vec3::new(20.0, 0.0, 0.0));
        c.update(0.25, &place);
        assert!((c.view.eye.x - 20.0).abs() < 1e-4, "{}", c.view.eye.x);
        assert_eq!(c.duration, 0.0);
    }
}

//! What the two games say to each other.
//!
//! Every message is a tag byte followed by its fields, little-endian, in
//! the order the struct lists them. Invented for co-op (the original
//! PC game's own online protocol is not read): the host is authoritative
//! for the world, and each player is authoritative for their own hero.
//!
//! Two channels carry them:
//! - the reliable stream (in order, never lost): [`Msg::Hello`],
//!   [`Msg::Welcome`], [`Msg::Bye`], [`Msg::Kind`], [`Msg::PersonIn`],
//!   [`Msg::PersonOut`], each framed by a `u16` length;
//! - datagrams (latest wins, may be lost or reordered): [`Msg::Hero`]
//!   and [`Msg::People`], sent many times a second, each one whole, so a
//!   lost one is simply replaced by the next.
//!
//! The host's people reach the friend in three steps: a kind (which
//! model, by the template's name and the model's hash, never the host's
//! own list index) once, then each person as they come near the friend
//! (`PersonIn`, with their look), then their place many times a second
//! (`People`) until they go (`PersonOut`).

use thiserror::Error;

/// Bumped whenever a message changes shape. Games that differ refuse
/// each other in the handshake rather than misreading.
pub const VERSION: u16 = 2;

/// The most people one [`Msg::People`] datagram carries (a datagram
/// must fit a packet: 40 of 22 bytes is 880).
pub const PEOPLE_PER_DATAGRAM: usize = 40;

/// The longest string a message may carry (names), bytes.
pub const MAX_STRING: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// The joiner's first word.
    Hello(Hello),
    /// The host's answer to a [`Msg::Hello`] it accepts.
    Welcome(Hello),
    /// Leaving, with a reason to show the other player.
    Bye(String),
    /// Where a player's hero is now.
    Hero(HeroState),
    /// Host: a kind of person, which [`PersonIn`]s name by `id`.
    Kind(KindInfo),
    /// Host: a person now shared with the friend.
    PersonIn(PersonIn),
    /// Host: a person no longer shared (gone, or far from the friend).
    PersonOut(u32),
    /// Host: where the shared people are now.
    People(PeopleBatch),
}

/// A kind of person as the host has loaded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindInfo {
    /// The host's number for it, only to name it in [`PersonIn`].
    pub id: u16,
    /// The template's name (`named`), else the catalogue role
    /// (`vz_hum_soldier`).
    pub name: String,
    /// The model's name hash.
    pub model: u32,
    pub named: bool,
}

/// A person the friend's game should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonIn {
    /// The host's id for this person for the session.
    pub net_id: u32,
    pub kind: u16,
    /// Which of the model's nodes are hidden (their outfit, picked at
    /// random on the host), one bit each, low bit first.
    pub outfit: Vec<bool>,
    /// Holds a gun.
    pub armed: bool,
}

/// One person's place, in a [`PeopleBatch`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PersonState {
    pub net_id: u32,
    pub position: [f32; 3],
    pub facing: f32,
    pub gait: Gait,
    pub flags: u8,
}

impl PersonState {
    pub const ALIVE: u8 = 1;
    pub const AIMING: u8 = 2;
    pub const DOWN: u8 = 4;
    pub const SWIMMING: u8 = 8;
}

/// Where some of the shared people are, at the host's `time` (the
/// session clock, as [`HeroState::time`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeopleBatch {
    pub time: f64,
    pub people: Vec<PersonState>,
}

/// Who a player is: the build they run, the hero they play, their name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub version: u16,
    /// [`Hero`](crate) index: 0 Mattias, 1 Jennifer, 2 Chris (the
    /// front end's order).
    pub hero: u8,
    pub name: String,
}

/// How a hero moves on foot, as the locomotion clips are picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Gait {
    #[default]
    Idle = 0,
    Walk = 1,
    Run = 2,
    Sprint = 3,
}

impl Gait {
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        Ok(match v {
            0 => Self::Idle,
            1 => Self::Walk,
            2 => Self::Run,
            3 => Self::Sprint,
            _ => return Err(DecodeError::Bad("gait")),
        })
    }
}

/// One snapshot of a hero, as its own player's game has it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeroState {
    /// Counts up with every snapshot sent: a datagram older than the
    /// newest one received is dropped.
    pub seq: u32,
    /// The sender's clock (seconds since its session started) when this
    /// was taken; the receiver plays snapshots back on this clock.
    pub time: f64,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// Which way the body faces, radians (0 is +z, as `Player::facing`).
    pub facing: f32,
    pub gait: Gait,
    pub flags: u8,
}

impl HeroState {
    /// In the air (jumping or falling).
    pub const AIRBORNE: u8 = 1;
    /// Crouched.
    pub const CROUCHED: u8 = 2;
    /// Weapon up, aiming.
    pub const AIMING: u8 = 4;
    /// In a vehicle (not drawn on foot).
    pub const DRIVING: u8 = 8;
    /// Swimming.
    pub const SWIMMING: u8 = 16;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("message ends early")]
    Short,
    #[error("unknown message tag {0}")]
    Tag(u8),
    #[error("bad {0}")]
    Bad(&'static str),
    #[error("{0} bytes left over")]
    Trailing(usize),
}

const TAG_HELLO: u8 = 1;
const TAG_WELCOME: u8 = 2;
const TAG_BYE: u8 = 3;
const TAG_HERO: u8 = 4;
const TAG_KIND: u8 = 5;
const TAG_PERSON_IN: u8 = 6;
const TAG_PERSON_OUT: u8 = 7;
const TAG_PEOPLE: u8 = 8;
/// The most nodes an outfit mask may name.
const MAX_OUTFIT: usize = 1024;

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Vec::with_capacity(48);
        match self {
            Msg::Hello(h) | Msg::Welcome(h) => {
                w.push(if matches!(self, Msg::Hello(_)) {
                    TAG_HELLO
                } else {
                    TAG_WELCOME
                });
                w.extend_from_slice(&h.version.to_le_bytes());
                w.push(h.hero);
                put_str(&mut w, &h.name);
            }
            Msg::Bye(reason) => {
                w.push(TAG_BYE);
                put_str(&mut w, reason);
            }
            Msg::Hero(s) => {
                w.push(TAG_HERO);
                w.extend_from_slice(&s.seq.to_le_bytes());
                w.extend_from_slice(&s.time.to_le_bytes());
                for v in s.position.iter().chain(&s.velocity) {
                    w.extend_from_slice(&v.to_le_bytes());
                }
                w.extend_from_slice(&s.facing.to_le_bytes());
                w.push(s.gait as u8);
                w.push(s.flags);
            }
            Msg::Kind(k) => {
                w.push(TAG_KIND);
                w.extend_from_slice(&k.id.to_le_bytes());
                put_str(&mut w, &k.name);
                w.extend_from_slice(&k.model.to_le_bytes());
                w.push(k.named as u8);
            }
            Msg::PersonIn(p) => {
                w.push(TAG_PERSON_IN);
                w.extend_from_slice(&p.net_id.to_le_bytes());
                w.extend_from_slice(&p.kind.to_le_bytes());
                let n = p.outfit.len().min(MAX_OUTFIT);
                w.extend_from_slice(&(n as u16).to_le_bytes());
                for chunk in p.outfit[..n].chunks(8) {
                    let byte = chunk
                        .iter()
                        .enumerate()
                        .fold(0u8, |b, (i, &on)| b | (u8::from(on) << i));
                    w.push(byte);
                }
                w.push(p.armed as u8);
            }
            Msg::PersonOut(id) => {
                w.push(TAG_PERSON_OUT);
                w.extend_from_slice(&id.to_le_bytes());
            }
            Msg::People(b) => {
                w.push(TAG_PEOPLE);
                w.extend_from_slice(&b.time.to_le_bytes());
                let n = b.people.len().min(PEOPLE_PER_DATAGRAM);
                w.push(n as u8);
                for p in &b.people[..n] {
                    w.extend_from_slice(&p.net_id.to_le_bytes());
                    for v in p.position.iter().chain([&p.facing]) {
                        w.extend_from_slice(&v.to_le_bytes());
                    }
                    w.push(p.gait as u8);
                    w.push(p.flags);
                }
            }
        }
        w
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(bytes);
        let msg = match r.u8()? {
            tag @ (TAG_HELLO | TAG_WELCOME) => {
                let h = Hello {
                    version: r.u16()?,
                    hero: r.u8()?,
                    name: r.string()?,
                };
                if tag == TAG_HELLO {
                    Msg::Hello(h)
                } else {
                    Msg::Welcome(h)
                }
            }
            TAG_BYE => Msg::Bye(r.string()?),
            TAG_HERO => {
                let seq = r.u32()?;
                let time = r.f64()?;
                let position = [r.f32()?, r.f32()?, r.f32()?];
                let velocity = [r.f32()?, r.f32()?, r.f32()?];
                let facing = r.f32()?;
                let gait = Gait::from_u8(r.u8()?)?;
                let flags = r.u8()?;
                let finite = position.iter().chain(&velocity).all(|v| v.is_finite())
                    && facing.is_finite()
                    && time.is_finite();
                if !finite {
                    return Err(DecodeError::Bad("number"));
                }
                Msg::Hero(HeroState {
                    seq,
                    time,
                    position,
                    velocity,
                    facing,
                    gait,
                    flags,
                })
            }
            TAG_KIND => Msg::Kind(KindInfo {
                id: r.u16()?,
                name: r.string()?,
                model: r.u32()?,
                named: r.u8()? != 0,
            }),
            TAG_PERSON_IN => {
                let net_id = r.u32()?;
                let kind = r.u16()?;
                let n = r.u16()? as usize;
                if n > MAX_OUTFIT {
                    return Err(DecodeError::Bad("outfit"));
                }
                let mut outfit = Vec::with_capacity(n);
                for _ in 0..n.div_ceil(8) {
                    let byte = r.u8()?;
                    for i in 0..8 {
                        if outfit.len() < n {
                            outfit.push(byte & (1 << i) != 0);
                        }
                    }
                }
                Msg::PersonIn(PersonIn {
                    net_id,
                    kind,
                    outfit,
                    armed: r.u8()? != 0,
                })
            }
            TAG_PERSON_OUT => Msg::PersonOut(r.u32()?),
            TAG_PEOPLE => {
                let time = r.f64()?;
                let n = r.u8()? as usize;
                if n > PEOPLE_PER_DATAGRAM {
                    return Err(DecodeError::Bad("people count"));
                }
                let mut people = Vec::with_capacity(n);
                for _ in 0..n {
                    let net_id = r.u32()?;
                    let position = [r.f32()?, r.f32()?, r.f32()?];
                    let facing = r.f32()?;
                    let gait = Gait::from_u8(r.u8()?)?;
                    let flags = r.u8()?;
                    if !(position.iter().all(|v| v.is_finite()) && facing.is_finite()) {
                        return Err(DecodeError::Bad("number"));
                    }
                    people.push(PersonState {
                        net_id,
                        position,
                        facing,
                        gait,
                        flags,
                    });
                }
                if !time.is_finite() {
                    return Err(DecodeError::Bad("number"));
                }
                Msg::People(PeopleBatch { time, people })
            }
            tag => return Err(DecodeError::Tag(tag)),
        };
        if !r.0.is_empty() {
            return Err(DecodeError::Trailing(r.0.len()));
        }
        Ok(msg)
    }
}

/// A string as a length byte then UTF-8, cut to [`MAX_STRING`] bytes at
/// a character boundary.
fn put_str(w: &mut Vec<u8>, s: &str) {
    let mut end = s.len().min(MAX_STRING);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    w.push(end as u8);
    w.extend_from_slice(&s.as_bytes()[..end]);
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(DecodeError::Short)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.take()?))
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.take()?))
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let n = self.u8()? as usize;
        if n > MAX_STRING || n > self.0.len() {
            return Err(DecodeError::Short);
        }
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        String::from_utf8(s.to_vec()).map_err(|_| DecodeError::Bad("string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero() -> HeroState {
        HeroState {
            seq: 7,
            time: 12.5,
            position: [450.0, 12.25, -1950.0],
            velocity: [1.0, 0.0, -3.5],
            facing: 1.25,
            gait: Gait::Run,
            flags: HeroState::AIMING,
        }
    }

    #[test]
    fn every_message_comes_back_the_same() {
        let hello = Hello {
            version: VERSION,
            hero: 2,
            name: "Chris".into(),
        };
        for msg in [
            Msg::Hello(hello.clone()),
            Msg::Welcome(hello),
            Msg::Bye("host quit".into()),
            Msg::Hero(hero()),
            Msg::Kind(KindInfo {
                id: 3,
                name: "vz_hum_soldier".into(),
                model: 0x1234_5678,
                named: false,
            }),
            Msg::PersonIn(PersonIn {
                net_id: 77,
                kind: 3,
                outfit: (0..19).map(|i| i % 3 == 0).collect(),
                armed: true,
            }),
            Msg::PersonOut(77),
            Msg::People(PeopleBatch {
                time: 3.5,
                people: vec![
                    PersonState {
                        net_id: 77,
                        position: [1.0, 2.0, 3.0],
                        facing: 0.5,
                        gait: Gait::Walk,
                        flags: PersonState::ALIVE | PersonState::AIMING,
                    };
                    2
                ],
            }),
        ] {
            assert_eq!(Msg::decode(&msg.encode()), Ok(msg));
        }
    }

    #[test]
    fn a_hero_snapshot_is_43_bytes() {
        // Tag, seq, time, six floats, facing, gait, flags: small enough
        // that 30 a second is about 1.3 KB/s each way.
        assert_eq!(Msg::Hero(hero()).encode().len(), 1 + 4 + 8 + 24 + 4 + 2);
    }

    #[test]
    fn a_full_people_datagram_fits_a_packet() {
        let full = Msg::People(PeopleBatch {
            time: 1.0,
            people: vec![PersonState::default(); PEOPLE_PER_DATAGRAM + 5],
        })
        .encode();
        // Tag, time, count, then 22 bytes each; the extra five left out.
        assert_eq!(full.len(), 1 + 8 + 1 + 22 * PEOPLE_PER_DATAGRAM);
        assert!(full.len() < 1100);
    }

    #[test]
    fn short_bad_and_trailing_bytes_are_refused() {
        let bytes = Msg::Hero(hero()).encode();
        for n in 0..bytes.len() {
            assert_eq!(Msg::decode(&bytes[..n]), Err(DecodeError::Short), "{n}");
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(Msg::decode(&long), Err(DecodeError::Trailing(1)));
        assert_eq!(Msg::decode(&[99]), Err(DecodeError::Tag(99)));
        let mut nan = hero();
        nan.position[1] = f32::NAN;
        assert_eq!(
            Msg::decode(&Msg::Hero(nan).encode()),
            Err(DecodeError::Bad("number"))
        );
    }

    #[test]
    fn long_names_are_cut_at_a_character() {
        let name = "é".repeat(40); // 80 bytes
        let msg = Msg::Bye(name);
        let Ok(Msg::Bye(back)) = Msg::decode(&msg.encode()) else {
            panic!()
        };
        assert_eq!(back.len(), 64);
        assert_eq!(back, "é".repeat(32));
    }
}

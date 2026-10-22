//! What the two games say to each other.
//!
//! Every message is a tag byte followed by its fields, little-endian, in
//! the order the struct lists them. Invented for co-op (the original
//! PC game's own online protocol is not read): the host is authoritative
//! for the world, and each player is authoritative for their own hero.
//!
//! Two channels carry them:
//! - the reliable stream (in order, never lost): [`Msg::Hello`],
//!   [`Msg::Welcome`], [`Msg::Bye`], each framed by a `u16` length;
//! - datagrams (latest wins, may be lost or reordered): [`Msg::Hero`],
//!   sent many times a second, each one whole, so a lost one is simply
//!   replaced by the next.

use thiserror::Error;

/// Bumped whenever a message changes shape. Games that differ refuse
/// each other in the handshake rather than misreading.
pub const VERSION: u16 = 1;

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

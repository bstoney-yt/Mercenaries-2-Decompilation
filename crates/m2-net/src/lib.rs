//! Online co-op: two players, each in their own copy of the game, on the
//! same island.
//!
//! - [`proto`]: the messages and how they are written.
//! - [`invite`]: the code and link a host sends a friend.
//! - [`interp`]: playing the other hero back smoothly.
//! - [`session`]: the connection itself, on a thread of its own.
//!
//! Only our own code goes over the wire or into the download: each game
//! reads the island from its own player's install.

pub mod interp;
pub mod invite;
pub mod proto;
pub mod session;

pub use interp::{Playback, Pose};
pub use invite::{Invite, InviteError};
pub use proto::{Gait, Hello, HeroState, Msg};
pub use session::{Event, Options, Session};

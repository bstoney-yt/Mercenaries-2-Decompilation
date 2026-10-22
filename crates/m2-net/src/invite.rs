//! The invite: what the host sends a friend so their game can find it.
//!
//! An invite names the host's endpoint (its public key, which also proves
//! the friend reached the right game) and, to find it faster, the relay
//! server it sits on and any addresses it can be reached at directly:
//!
//! ```text
//! <endpoint id>[@<relay host>][+<ip:port>]...
//! ```
//!
//! The link sent to a friend is [`LINK_BASE`] followed by that code. The
//! code rides in the link's fragment (after `#`), which browsers never
//! send to the web server: the page reads it to offer the download and
//! hand the code to the game.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};
use thiserror::Error;

/// The join page the invite link opens.
pub const LINK_BASE: &str = "https://bstoney-yt.github.io/Mercenaries-2-Decompilation/join/#";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
    pub direct: Vec<SocketAddr>,
}

#[derive(Debug, Error)]
pub enum InviteError {
    #[error("no invite code in it")]
    Empty,
    #[error("the game's id in it is wrong: {0}")]
    Id(String),
    #[error("the relay in it is wrong: {0}")]
    Relay(String),
    #[error("an address in it is wrong: {0}")]
    Address(String),
}

impl Invite {
    /// The invite for an endpoint's current address.
    pub fn from_addr(addr: &EndpointAddr) -> Self {
        Self {
            id: addr.id,
            relay: addr.relay_urls().next().cloned(),
            direct: addr.ip_addrs().copied().collect(),
        }
    }

    /// Where to dial.
    pub fn to_addr(&self) -> EndpointAddr {
        let relay = self.relay.iter().cloned().map(TransportAddr::Relay);
        let direct = self.direct.iter().copied().map(TransportAddr::Ip);
        EndpointAddr::from_parts(self.id, relay.chain(direct))
    }

    /// The link to send a friend.
    pub fn link(&self) -> String {
        format!("{LINK_BASE}{self}")
    }

    /// Just the id and the relay: what a link to someone outside this
    /// network needs (the direct addresses are found by the relay).
    pub fn without_direct(&self) -> Self {
        Self {
            direct: Vec::new(),
            ..self.clone()
        }
    }
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)?;
        if let Some(relay) = &self.relay
            && let Some(host) = relay.host_str()
        {
            write!(f, "@{host}")?;
        }
        for a in &self.direct {
            write!(f, "+{a}")?;
        }
        Ok(())
    }
}

impl FromStr for Invite {
    type Err = InviteError;

    /// Reads a code, an `m2coop:` code, or a whole link (anything up to
    /// its `#` is ignored), with any whitespace around it.
    fn from_str(s: &str) -> Result<Self, InviteError> {
        let s = s.trim();
        let s = s.rsplit_once('#').map_or(s, |(_, code)| code);
        let s = s.strip_prefix("m2coop:").unwrap_or(s).trim();
        if s.is_empty() {
            return Err(InviteError::Empty);
        }
        let mut parts = s.split('+');
        let head = parts.next().unwrap_or_default();
        let (id, relay) = match head.split_once('@') {
            Some((id, host)) => (id, Some(host)),
            None => (head, None),
        };
        let id = EndpointId::from_str(id).map_err(|e| InviteError::Id(e.to_string()))?;
        let relay = relay
            .map(|host| {
                RelayUrl::from_str(&format!("https://{host}/"))
                    .map_err(|e| InviteError::Relay(e.to_string()))
            })
            .transpose()?;
        let direct = parts
            .map(|a| SocketAddr::from_str(a).map_err(|_| InviteError::Address(a.into())))
            .collect::<Result<_, _>>()?;
        Ok(Self { id, relay, direct })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn invite() -> Invite {
        Invite {
            id: SecretKey::from_bytes(&[7; 32]).public(),
            relay: Some("https://use1-1.relay.n0.iroh.link./".parse().unwrap()),
            direct: vec![
                "192.168.1.20:51000".parse().unwrap(),
                "[::1]:51001".parse().unwrap(),
            ],
        }
    }

    #[test]
    fn a_code_reads_back_the_same() {
        let i = invite();
        assert_eq!(i.to_string().parse::<Invite>().unwrap(), i);
        let bare = Invite {
            relay: None,
            direct: Vec::new(),
            ..i
        };
        assert_eq!(bare.to_string().parse::<Invite>().unwrap(), bare);
    }

    #[test]
    fn a_link_or_a_prefixed_code_reads_too() {
        let i = invite().without_direct();
        assert!(i.link().starts_with(LINK_BASE));
        assert_eq!(i.link().parse::<Invite>().unwrap(), i);
        assert_eq!(format!("  m2coop:{i}\n").parse::<Invite>().unwrap(), i);
    }

    #[test]
    fn rubbish_is_refused() {
        assert!(matches!("".parse::<Invite>(), Err(InviteError::Empty)));
        assert!(matches!(
            "https://example.com/join/#".parse::<Invite>(),
            Err(InviteError::Empty)
        ));
        assert!(matches!("hello".parse::<Invite>(), Err(InviteError::Id(_))));
        let id = invite().id;
        assert!(matches!(
            format!("{id}+nowhere").parse::<Invite>(),
            Err(InviteError::Address(_))
        ));
    }
}

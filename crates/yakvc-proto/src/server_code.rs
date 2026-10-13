//! The server code: what a self-hoster hands their friends to point their
//! clients at another rendezvous, in one line that survives chat and
//! copy-paste.

use std::fmt;
use std::str::FromStr;

use iroh_base::{EndpointId, RelayUrl};

use crate::IssuerId;

/// A rendezvous and its issuer, as `yakvc1:<endpoint id>:<issuer id>:<relay URL>`.
///
/// The relay comes last because it holds colons of its own. Clients reach the
/// rendezvous through its relay, by hostname, so the code needs no IP address
/// and the server can move with a DNS change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCode {
    pub endpoint_id: EndpointId,
    pub issuer_id: IssuerId,
    pub relay: RelayUrl,
}

const PREFIX: &str = "yakvc1:";

#[derive(Debug, thiserror::Error)]
#[error("not a Yak VC server code: {0}")]
pub struct ServerCodeError(String);

impl fmt::Display for ServerCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{PREFIX}{}:{}:{}",
            self.endpoint_id, self.issuer_id, self.relay
        )
    }
}

impl FromStr for ServerCode {
    type Err = ServerCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |why: &str| ServerCodeError(why.to_owned());
        let rest = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| invalid("it must start with yakvc1:"))?;
        let mut parts = rest.splitn(3, ':');
        let (Some(endpoint), Some(issuer), Some(relay)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid("it has fewer than three parts"));
        };
        Ok(ServerCode {
            endpoint_id: endpoint
                .parse()
                .map_err(|e| invalid(&format!("bad endpoint id: {e}")))?,
            issuer_id: issuer
                .parse()
                .map_err(|e| invalid(&format!("bad issuer id: {e}")))?,
            relay: relay
                .parse()
                .map_err(|e| invalid(&format!("bad relay URL: {e}")))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssuerKey;
    use iroh_base::SecretKey;

    #[test]
    fn round_trips() {
        let code = ServerCode {
            endpoint_id: SecretKey::generate().public(),
            issuer_id: IssuerKey::generate().id(),
            relay: "https://relay.example.com:443".parse().unwrap(),
        };
        let text = code.to_string();
        assert!(text.starts_with("yakvc1:"), "{text}");
        assert!(!text.contains(char::is_whitespace), "{text}");
        assert_eq!(format!(" {text}\n").parse::<ServerCode>().unwrap(), code);
    }

    #[test]
    fn rejects_other_text() {
        for text in [
            "",
            "yakvc1:",
            "yakvc1:a:b",
            "yakvc1:x:y:https://r/",
            "https://relay.example.com/",
        ] {
            assert!(text.parse::<ServerCode>().is_err(), "{text}");
        }
    }
}

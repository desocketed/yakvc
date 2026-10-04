//! The Mojang session challenge used to prove a UUID to the rendezvous.

use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// Random challenge sent by the rendezvous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nonce(pub [u8; 32]);

impl Nonce {
    pub fn random() -> Self {
        Nonce(rand::random())
    }
}

/// The `serverId` both sides pass to Mojang: the client to `joinServer`, the
/// rendezvous to `hasJoined`.
///
/// `mc_hex_digest(SHA-1("yakvc-auth-v1" ‖ nonce ‖ client ‖ rendezvous))`, where
/// `mc_hex_digest` is Minecraft's signed two's-complement hex form.
pub fn session_server_id(nonce: &Nonce, client: EndpointId, rendezvous: EndpointId) -> String {
    let hash = Sha1::new()
        .chain_update(b"yakvc-auth-v1")
        .chain_update(nonce.0)
        .chain_update(client.as_bytes())
        .chain_update(rendezvous.as_bytes())
        .finalize();
    mc_hex_digest(hash.into())
}

/// Minecraft prints a SHA-1 as Java's `new BigInteger(hash).toString(16)`:
/// the bytes as a signed big-endian number, so a set top bit gives a minus
/// sign and the magnitude, and leading zeros are dropped.
fn mc_hex_digest(mut hash: [u8; 20]) -> String {
    let negative = hash[0] & 0x80 != 0;
    if negative {
        // Two's-complement negation: invert, then add one.
        let mut carry = true;
        for byte in hash.iter_mut().rev() {
            let (sum, overflow) = (!*byte).overflowing_add(u8::from(carry));
            *byte = sum;
            carry = overflow;
        }
    }
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let digits = hex.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    if negative {
        format!("-{digits}")
    } else {
        digits.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;

    fn digest_of(s: &str) -> String {
        mc_hex_digest(Sha1::digest(s).into())
    }

    /// The well-known examples from the Minecraft protocol documentation.
    #[test]
    fn hex_digest_matches_minecraft() {
        assert_eq!(
            digest_of("Notch"),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            digest_of("jeb_"),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(
            digest_of("simon"),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn hex_digest_edge_cases() {
        assert_eq!(mc_hex_digest([0; 20]), "0");
        assert_eq!(mc_hex_digest([0xff; 20]), "-1");
        let mut min = [0; 20];
        min[0] = 0x80;
        assert_eq!(mc_hex_digest(min), format!("-8{}", "0".repeat(39)));
    }

    #[test]
    fn server_id_binds_every_input() {
        let nonce = Nonce([7; 32]);
        let client = SecretKey::from_bytes(&[1; 32]).public();
        let rdv = SecretKey::from_bytes(&[2; 32]).public();
        let id = session_server_id(&nonce, client, rdv);
        assert_eq!(id, session_server_id(&nonce, client, rdv));
        assert_ne!(id, session_server_id(&Nonce([8; 32]), client, rdv));
        assert_ne!(id, session_server_id(&nonce, rdv, client));
        assert!(id.len() <= 41);
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(Nonce::random(), Nonce::random());
    }
}

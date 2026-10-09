//! The Mojang session challenge used to prove a UUID to the rendezvous, and
//! the offline UUIDs that need no proof.

use iroh_base::EndpointId;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use uuid::Uuid;

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

/// The UUID an offline-mode server gives the player `name`:
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`, an
/// MD5 name-based UUID (version 3) without a namespace.
pub fn offline_uuid(name: &str) -> Uuid {
    let hash = Md5::new()
        .chain_update(b"OfflinePlayer:")
        .chain_update(name.as_bytes())
        .finalize();
    uuid::Builder::from_md5_bytes(hash.into()).into_uuid()
}

/// Whether `uuid` is the offline UUID of `name`. Online-mode servers only use
/// account UUIDs (version 4), which never match. Checking the name, not just
/// the version, stops an unverified ticket from pairing someone's offline
/// UUID with another name.
pub fn is_offline_player(uuid: Uuid, name: &str) -> bool {
    uuid == offline_uuid(name)
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

    /// From Java's `UUID.nameUUIDFromBytes` (OpenJDK 25), names as UTF-8.
    const JAVA_OFFLINE_UUIDS: [(&str, &str); 5] = [
        ("Notch", "b50ad385-829d-3141-a216-7e7d7539ba7f"),
        ("jeb_", "a762f560-4fce-3236-812a-b80efff0b62b"),
        ("Steve", "5627dd98-e6be-3c21-b8a8-e92344183641"),
        ("a", "52428a0e-1e30-3cb1-976c-e728b2614047"),
        (
            "\u{dc}n\u{ef}c\u{f6}d\u{e9}_\u{540d}\u{524d}",
            "e1197f6d-e1fd-32af-9d16-58fea814fbc3",
        ),
    ];

    #[test]
    fn offline_uuids_match_java() {
        for (name, expected) in JAVA_OFFLINE_UUIDS {
            let uuid = offline_uuid(name);
            assert_eq!(uuid.to_string(), expected, "{name}");
            assert!(is_offline_player(uuid, name));
        }
        assert!(!is_offline_player(offline_uuid("alice"), "Alice"));
        assert!(!is_offline_player(Uuid::new_v4(), "alice"));
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(Nonce::random(), Nonce::random());
    }
}

//! The Mojang session challenge used to prove a UUID to the rendezvous.

use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};

/// Random challenge sent by the rendezvous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nonce(pub [u8; 32]);

impl Nonce {
    pub fn random() -> Self {
        todo!()
    }
}

/// The `serverId` both sides pass to Mojang: the client to `joinServer`, the
/// rendezvous to `hasJoined`.
///
/// `mc_hex_digest(SHA-1("yakvc-auth-v1" ‖ nonce ‖ client ‖ rendezvous))`, where
/// `mc_hex_digest` is Minecraft's signed two's-complement hex form.
pub fn session_server_id(nonce: &Nonce, client: EndpointId, rendezvous: EndpointId) -> String {
    let _ = (nonce, client, rendezvous);
    todo!()
}

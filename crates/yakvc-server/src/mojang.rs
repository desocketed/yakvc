//! Client for Mojang's session server (`hasJoined`).

use std::time::Duration;

use yakvc_shared::Uuid;

/// A Mojang session server. [`SessionServer::default`] is Mojang's; tests
/// point it at a fake.
#[derive(Debug, Clone)]
pub struct SessionServer {
    _base_url: String,
}

/// The profile `hasJoined` confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub uuid: Uuid,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MojangError {
    #[error("rate limited by Mojang")]
    RateLimited { retry_after: Option<Duration> },
    #[error("Mojang session server unavailable: {0}")]
    Unavailable(String),
}

impl SessionServer {
    pub fn new(base_url: impl Into<String>) -> Self {
        SessionServer {
            _base_url: base_url.into(),
        }
    }

    /// `GET {base}/session/minecraft/hasJoined`. `Ok(None)` when Mojang does
    /// not confirm the join. 5 s timeout, one retry.
    pub async fn has_joined(
        &self,
        name: &str,
        server_id: &str,
    ) -> Result<Option<Profile>, MojangError> {
        let _ = (name, server_id);
        todo!()
    }
}

impl Default for SessionServer {
    fn default() -> Self {
        SessionServer::new("https://sessionserver.mojang.com")
    }
}

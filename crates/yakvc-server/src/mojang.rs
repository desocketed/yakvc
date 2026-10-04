//! Client for Mojang's session server (`hasJoined`).

use std::time::Duration;

use reqwest::{StatusCode, Url};
use serde::Deserialize;
use yakvc_shared::Uuid;

const TIMEOUT: Duration = Duration::from_secs(5);

/// A Mojang session server. [`SessionServer::default`] is Mojang's; tests
/// point it at a fake.
#[derive(Debug, Clone)]
pub struct SessionServer {
    base_url: String,
    http: reqwest::Client,
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

/// The part of the `hasJoined` response we use.
#[derive(Deserialize)]
struct HasJoinedResponse {
    id: Uuid,
    name: String,
}

impl SessionServer {
    pub fn new(base_url: impl Into<String>) -> Self {
        // reqwest is built without its own crypto provider because iroh
        // already brings ring; make ring the process default. This fails
        // harmlessly if something installed a default first.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .expect("an HTTP client with default settings always builds");
        SessionServer {
            base_url: base_url.into(),
            http,
        }
    }

    /// `GET {base}/session/minecraft/hasJoined`. `Ok(None)` when Mojang does
    /// not confirm the join. 5 s timeout, one retry.
    pub async fn has_joined(
        &self,
        name: &str,
        server_id: &str,
    ) -> Result<Option<Profile>, MojangError> {
        let url = Url::parse_with_params(
            &format!("{}/session/minecraft/hasJoined", self.base_url),
            [("username", name), ("serverId", server_id)],
        )
        .map_err(|e| MojangError::Unavailable(format!("bad session server URL: {e}")))?;

        match self.request(url.clone()).await {
            Err(MojangError::Unavailable(_)) => self.request(url).await,
            result => result,
        }
    }

    async fn request(&self, url: Url) -> Result<Option<Profile>, MojangError> {
        let unavailable = |e: reqwest::Error| MojangError::Unavailable(e.to_string());
        let response = self.http.get(url).send().await.map_err(unavailable)?;
        match response.status() {
            StatusCode::OK => {
                let body = response.bytes().await.map_err(unavailable)?;
                let profile: HasJoinedResponse = serde_json::from_slice(&body)
                    .map_err(|e| MojangError::Unavailable(format!("bad response: {e}")))?;
                Ok(Some(Profile {
                    uuid: profile.id,
                    name: profile.name,
                }))
            }
            // Mojang answers "not joined" with an empty 204.
            StatusCode::NO_CONTENT => Ok(None),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok())
                    .map(Duration::from_secs);
                Err(MojangError::RateLimited { retry_after })
            }
            status => Err(MojangError::Unavailable(format!("HTTP {status}"))),
        }
    }
}

impl Default for SessionServer {
    fn default() -> Self {
        SessionServer::new("https://sessionserver.mojang.com")
    }
}

/// A scripted stand-in for Mojang's session server, for tests.
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves HTTP on localhost. Each request gets the next scripted
    /// response; once the script runs out, it answers like Mojang does for
    /// players that have joined: with the profile of the `username` asked
    /// about, under `uuid_for(username)`.
    #[derive(Debug, Clone)]
    pub struct FakeMojang {
        pub url: String,
        script: Arc<Mutex<Vec<String>>>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    /// The fake's UUID for a player name: the name's bytes, zero padded.
    pub fn uuid_for(name: &str) -> yakvc_shared::Uuid {
        let mut bytes = [0; 16];
        for (byte, c) in bytes.iter_mut().zip(name.bytes()) {
            *byte = c;
        }
        yakvc_shared::Uuid::from_bytes(bytes)
    }

    pub fn response(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    impl FakeMojang {
        pub async fn start() -> FakeMojang {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let fake = FakeMojang {
                url: format!("http://{}", listener.local_addr().unwrap()),
                script: Arc::default(),
                requests: Arc::default(),
            };
            let server = fake.clone();
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let mut buf = vec![0; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request.split(' ').nth(1).unwrap_or("").to_string();
                    let reply = server.reply_to(&path);
                    server.requests.lock().unwrap().push(path);
                    let _ = stream.write_all(reply.as_bytes()).await;
                }
            });
            fake
        }

        /// Queues responses for the next requests, in order.
        pub fn script(&self, responses: impl IntoIterator<Item = String>) {
            self.script.lock().unwrap().extend(responses);
        }

        /// Request paths (with query) received so far.
        pub fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn reply_to(&self, path: &str) -> String {
            let mut script = self.script.lock().unwrap();
            if !script.is_empty() {
                return script.remove(0);
            }
            let name = path
                .split(['?', '&'])
                .find_map(|kv| kv.strip_prefix("username="))
                .unwrap_or("");
            let body = format!(r#"{{"id":"{}","name":"{name}"}}"#, uuid_for(name).simple());
            response("200 OK", "", &body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeMojang, response, uuid_for};
    use super::*;

    #[tokio::test]
    async fn confirmed_join_returns_the_profile() {
        let mojang = FakeMojang::start().await;
        let server = SessionServer::new(&mojang.url);
        let profile = server.has_joined("Alice_1", "-1f").await.unwrap();
        assert_eq!(
            profile,
            Some(Profile {
                uuid: uuid_for("Alice_1"),
                name: "Alice_1".into()
            })
        );
        assert_eq!(
            mojang.requests(),
            ["/session/minecraft/hasJoined?username=Alice_1&serverId=-1f"]
        );
    }

    #[tokio::test]
    async fn no_content_means_not_joined() {
        let mojang = FakeMojang::start().await;
        mojang.script([response("204 No Content", "", "")]);
        let server = SessionServer::new(&mojang.url);
        assert_eq!(server.has_joined("alice", "1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn rate_limit_reports_retry_after_and_is_not_retried() {
        let mojang = FakeMojang::start().await;
        mojang.script([response("429 Too Many Requests", "Retry-After: 42\r\n", "")]);
        let server = SessionServer::new(&mojang.url);
        let err = server.has_joined("alice", "1").await.unwrap_err();
        assert!(matches!(
            err,
            MojangError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(42)
        ));
        assert_eq!(mojang.requests().len(), 1);

        mojang.script([response("429 Too Many Requests", "", "")]);
        let err = server.has_joined("alice", "1").await.unwrap_err();
        assert!(matches!(
            err,
            MojangError::RateLimited { retry_after: None }
        ));
    }

    #[tokio::test]
    async fn server_errors_are_retried_once() {
        let mojang = FakeMojang::start().await;
        let server = SessionServer::new(&mojang.url);

        mojang.script([response("503 Service Unavailable", "", "")]);
        assert!(server.has_joined("alice", "1").await.unwrap().is_some());
        assert_eq!(mojang.requests().len(), 2);

        mojang.script([
            response("500 Internal Server Error", "", ""),
            response("200 OK", "", "not json"),
        ]);
        let err = server.has_joined("alice", "1").await.unwrap_err();
        assert!(matches!(err, MojangError::Unavailable(_)));
        assert_eq!(mojang.requests().len(), 4);
    }

    #[tokio::test]
    async fn unreachable_server_is_unavailable() {
        // Nothing can listen on port 0, so the connection is always refused.
        // (Binding a free port and dropping it races with other tests.)
        let server = SessionServer::new("http://127.0.0.1:0".to_owned());
        let err = server.has_joined("alice", "1").await.unwrap_err();
        assert!(matches!(err, MojangError::Unavailable(_)));
    }
}

//! Client for Mojang's session server (`hasJoined`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::{StatusCode, Url};
use serde::Deserialize;
use yakvc_shared::Uuid;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Mojang's services discovery document. authlib 10 takes its `hasJoined`
/// URL from here too, so we follow the game if Mojang ever moves it.
const DISCOVERY_URL: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// Where the document keeps the `hasJoined` URL.
const VERIFY_POINTER: &str = "/discovery/session/endpoints/verify/uri";

/// Used until discovery succeeds. The document named this URL on 2026-10-04.
const FALLBACK_VERIFY_URL: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// A Mojang session server. [`SessionServer::default`] is Mojang's, found
/// through the discovery document; tests point [`SessionServer::new`] at a
/// fake.
#[derive(Debug, Clone)]
pub struct SessionServer {
    /// The full `hasJoined` URL. Clones share it, so a refresh by the
    /// discovery loop reaches the copy auth uses.
    verify_url: Arc<Mutex<String>>,
    /// Where to refresh `verify_url` from; `None` keeps it fixed.
    discovery_url: Option<String>,
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
    /// A session server that verifies at
    /// `{base_url}/session/minecraft/hasJoined`, with no discovery.
    pub fn new(base_url: impl Into<String>) -> Self {
        let verify_url = format!("{}/session/minecraft/hasJoined", base_url.into());
        SessionServer::build(verify_url, None)
    }

    fn build(verify_url: String, discovery_url: Option<String>) -> Self {
        // reqwest is built without its own crypto provider because iroh
        // already brings ring; make ring the process default. This fails
        // harmlessly if something installed a default first.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .expect("an HTTP client with default settings always builds");
        SessionServer {
            verify_url: Arc::new(Mutex::new(verify_url)),
            discovery_url,
            http,
        }
    }

    /// Refreshes the `hasJoined` URL from the discovery document now and
    /// every 24 h. Returns at once if the URL is fixed.
    pub(crate) async fn discovery_loop(self) {
        if self.discovery_url.is_none() {
            return;
        }
        let mut interval = tokio::time::interval(DISCOVERY_INTERVAL);
        loop {
            interval.tick().await;
            self.discover().await;
        }
    }

    /// Takes the `hasJoined` URL from the discovery document. If that fails
    /// the URL stays as it was: the fallback, or the last one discovered.
    async fn discover(&self) {
        let Some(discovery_url) = &self.discovery_url else {
            return;
        };
        if let Some(url) = self.fetch_verify_url(discovery_url).await {
            *self.verify_url.lock().unwrap() = url;
        }
    }

    async fn fetch_verify_url(&self, discovery_url: &str) -> Option<String> {
        let response = self.http.get(discovery_url).send().await.ok()?;
        if response.status() != StatusCode::OK {
            return None;
        }
        let body = response.bytes().await.ok()?;
        let document: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let url = document.pointer(VERIFY_POINTER)?.as_str()?;
        Url::parse(url).ok()?;
        Some(url.to_owned())
    }

    /// `GET <verify URL>?username=..&serverId=..`. `Ok(None)` when Mojang
    /// does not confirm the join. 5 s timeout, one retry.
    pub async fn has_joined(
        &self,
        name: &str,
        server_id: &str,
    ) -> Result<Option<Profile>, MojangError> {
        let verify_url = self.verify_url.lock().unwrap().clone();
        let url =
            Url::parse_with_params(&verify_url, [("username", name), ("serverId", server_id)])
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
        SessionServer::build(FALLBACK_VERIFY_URL.into(), Some(DISCOVERY_URL.into()))
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

    /// A session server that discovers its verify URL from `fake`.
    fn discovering(fake: &FakeMojang) -> SessionServer {
        let discovery_url = format!("{}/minecraft/client", fake.url);
        SessionServer::build(FALLBACK_VERIFY_URL.into(), Some(discovery_url))
    }

    fn verify_url(server: &SessionServer) -> String {
        server.verify_url.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn discovery_sets_the_verify_url() {
        let fake = FakeMojang::start().await;
        // The shape of the real document, cut down.
        let document = format!(
            r#"{{"environment":"prod","discovery":{{"session":{{"endpoints":{{
                "join":{{"uri":"https://sessionserver.mojang.com/session/minecraft/join"}},
                "verify":{{"uri":"{}/v2/hasJoined"}}}}}}}}}}"#,
            fake.url
        );
        fake.script([response("200 OK", "", &document)]);
        let server = discovering(&fake);
        server.discover().await;
        assert_eq!(verify_url(&server), format!("{}/v2/hasJoined", fake.url));

        assert!(server.has_joined("alice", "1").await.unwrap().is_some());
        assert_eq!(
            fake.requests(),
            [
                "/minecraft/client",
                "/v2/hasJoined?username=alice&serverId=1"
            ]
        );
    }

    #[tokio::test]
    async fn malformed_discovery_keeps_the_previous_url() {
        let fake = FakeMojang::start().await;
        let server = discovering(&fake);
        for body in [
            "not json",
            r#"{"discovery":{"session":{}}}"#,
            r#"{"discovery":{"session":{"endpoints":{"verify":{"uri":"not a url"}}}}}"#,
        ] {
            fake.script([response("200 OK", "", body)]);
            server.discover().await;
            assert_eq!(verify_url(&server), FALLBACK_VERIFY_URL, "{body}");
        }
        fake.script([response("503 Service Unavailable", "", "")]);
        server.discover().await;
        assert_eq!(verify_url(&server), FALLBACK_VERIFY_URL);
    }

    #[tokio::test]
    async fn unreachable_discovery_keeps_the_fallback() {
        let server = SessionServer::build(
            FALLBACK_VERIFY_URL.into(),
            Some("http://127.0.0.1:0/minecraft/client".into()),
        );
        server.discover().await;
        assert_eq!(verify_url(&server), FALLBACK_VERIFY_URL);
    }

    #[tokio::test]
    async fn an_explicit_url_is_never_rediscovered() {
        let fake = FakeMojang::start().await;
        let server = SessionServer::new(&fake.url);
        // Returns at once rather than looping.
        server.clone().discovery_loop().await;
        assert!(fake.requests().is_empty());
        assert_eq!(
            verify_url(&server),
            format!("{}/session/minecraft/hasJoined", fake.url)
        );
    }

    #[test]
    fn default_starts_from_the_fallback() {
        let server = SessionServer::default();
        assert_eq!(verify_url(&server), FALLBACK_VERIFY_URL);
        assert_eq!(server.discovery_url.as_deref(), Some(DISCOVERY_URL));
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

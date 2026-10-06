//! In-process tests of the rendezvous protocol: a real server and minimal
//! clients speaking `yakvc/rdv/1` over loopback.

use std::time::{Duration, SystemTime};

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, presets};
use iroh::{Endpoint, RelayMode};
use yakvc_shared::rdv::{self, ClientMsg, CloseCode, Hello, ServerMsg};
use yakvc_shared::wire::{read_msg, write_msg};
use yakvc_shared::{
    EndpointAddr, EndpointId, IssuerKey, PairToken, RelayUrl, SecretKey, SignedTicket,
    TicketVerifier, Uuid,
};

use crate::mojang::fake::{FakeMojang, response, uuid_for};
use crate::{Limits, RelayOptions, Server, ServerBuilder, SessionServer};

const TIMEOUT: Duration = Duration::from_secs(10);

fn builder() -> ServerBuilder {
    Server::builder(SecretKey::generate(), IssuerKey::generate())
        .bind("127.0.0.1:0".parse().unwrap())
}

async fn dev_server() -> Server {
    builder().insecure_dev_auth().spawn().await.unwrap()
}

async fn mojang_server(mojang: &FakeMojang) -> Server {
    builder()
        .session_server(SessionServer::new(&mojang.url))
        .spawn()
        .await
        .unwrap()
}

fn relay_options() -> RelayOptions {
    RelayOptions {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        quic_bind: None,
        open: false,
    }
}

/// A rendezvous client reduced to sending and receiving messages.
struct Client {
    name: String,
    endpoint: Endpoint,
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl Client {
    async fn connect(server: &Server, name: &str) -> Client {
        let endpoint = loopback_endpoint().await;
        Client::connect_with(endpoint, server.endpoint_addr(), name).await
    }

    /// Connects through the server's relay only, like a `relay_only` client.
    async fn connect_via_relay(server: &Server, name: &str) -> Client {
        let url = server.relay_url().unwrap();
        let endpoint = relay_only_endpoint(url.clone()).await;
        let addr = EndpointAddr::new(server.endpoint_id()).with_relay_url(url);
        Client::connect_with(endpoint, addr, name).await
    }

    async fn connect_with(endpoint: Endpoint, addr: EndpointAddr, name: &str) -> Client {
        let conn = tokio::time::timeout(TIMEOUT, endpoint.connect(addr, rdv::ALPN))
            .await
            .expect("connect timed out")
            .unwrap();
        let (send, recv) = conn.open_bi().await.unwrap();
        Client {
            name: name.into(),
            endpoint,
            conn,
            send,
            recv,
        }
    }

    fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    fn uuid(&self) -> Uuid {
        uuid_for(&self.name)
    }

    fn hello(&self, cached_ticket: Option<SignedTicket>) -> ClientMsg {
        ClientMsg::Hello(Hello {
            mod_version: "test".into(),
            uuid: self.uuid(),
            name: self.name.clone(),
            addr: EndpointAddr::new(self.id()),
            cached_ticket,
        })
    }

    async fn send(&mut self, msg: ClientMsg) {
        write_msg(&mut self.send, &msg).await.unwrap();
    }

    async fn recv(&mut self) -> ServerMsg {
        tokio::time::timeout(TIMEOUT, read_msg(&mut self.recv))
            .await
            .expect("no message from the server")
            .unwrap()
            .expect("server ended the stream")
    }

    async fn expect_registered(&mut self) -> SignedTicket {
        match self.recv().await {
            ServerMsg::Registered(ticket) => ticket,
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    /// Registers with a dev-auth server.
    async fn register(&mut self) -> SignedTicket {
        self.send(self.hello(None)).await;
        self.expect_registered().await
    }

    /// Runs the challenge against the fake Mojang, which confirms any join.
    async fn answer_challenge(&mut self) {
        assert!(matches!(self.recv().await, ServerMsg::Challenge(_)));
        self.send(ClientMsg::Joined).await;
    }

    fn token_for(&self, other: &Client) -> PairToken {
        PairToken::new(self.uuid(), other.uuid())
    }

    async fn expect_peer_available(&mut self, peer: &Client) {
        match self.recv().await {
            ServerMsg::PeerAvailable { ticket, addr } => {
                assert_eq!(addr.id, peer.id());
                let ticket = TicketVerifier::new([ticket.issuer()])
                    .accept_dev(true)
                    .verify(&ticket, SystemTime::now())
                    .unwrap();
                assert_eq!(ticket.endpoint_id, peer.id());
                assert_eq!(ticket.uuid, peer.uuid());
            }
            other => panic!("expected PeerAvailable, got {other:?}"),
        }
    }

    async fn close_code(&self) -> u64 {
        let err = tokio::time::timeout(TIMEOUT, self.conn.closed())
            .await
            .expect("server kept the connection open");
        match err {
            ConnectionError::ApplicationClosed(close) => close.error_code.into(),
            other => panic!("unexpected close: {other:?}"),
        }
    }
}

async fn loopback_endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap()
}

async fn relay_only_endpoint(url: RelayUrl) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::custom([url]))
        .bind()
        .await
        .unwrap()
}

async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn code(code: CloseCode) -> u64 {
    code as u64
}

#[tokio::test]
async fn dev_clients_that_see_each_other_are_matched() {
    let server = dev_server().await;
    let mut alice = Client::connect(&server, "alice").await;
    let mut bob = Client::connect(&server, "bob").await;
    let mut carol = Client::connect(&server, "carol").await;
    let ticket = alice.register().await;
    bob.register().await;
    carol.register().await;

    let verifier = TicketVerifier::new([server.issuer_id()]).accept_dev(true);
    let ticket = verifier.verify(&ticket, SystemTime::now()).unwrap();
    assert!(ticket.dev);
    assert_eq!(ticket.endpoint_id, alice.id());
    assert_eq!(ticket.uuid, alice.uuid());

    // Alice sees Bob and Carol; Bob sees Alice; Carol sees nobody.
    let tokens = vec![alice.token_for(&bob), alice.token_for(&carol)];
    alice.send(ClientMsg::SetPairs(tokens)).await;
    bob.send(ClientMsg::SetPairs(vec![bob.token_for(&alice)]))
        .await;
    alice.expect_peer_available(&bob).await;
    bob.expect_peer_available(&alice).await;
    assert_eq!(server.stats().sessions, 3);
    assert_eq!(server.stats().matches, 1);

    // Bob leaves; Alice is told.
    bob.conn.close(0u32.into(), b"");
    match alice.recv().await {
        ServerMsg::PeerGone(id) => assert_eq!(id, bob.id()),
        other => panic!("expected PeerGone, got {other:?}"),
    }
    wait_for("bob's session to end", || server.stats().sessions == 2).await;
    assert_eq!(server.stats().matches, 0);
    drop(carol);
}

#[tokio::test]
async fn address_updates_reach_matches() {
    let server = dev_server().await;
    let mut alice = Client::connect(&server, "alice").await;
    let mut bob = Client::connect(&server, "bob").await;
    alice.register().await;
    bob.register().await;
    alice
        .send(ClientMsg::AddPairs(vec![alice.token_for(&bob)]))
        .await;
    bob.send(ClientMsg::AddPairs(vec![bob.token_for(&alice)]))
        .await;
    alice.expect_peer_available(&bob).await;
    bob.expect_peer_available(&alice).await;

    let new_addr = EndpointAddr::new(alice.id()).with_ip_addr("127.0.0.1:9".parse().unwrap());
    alice.send(ClientMsg::UpdateAddr(new_addr.clone())).await;
    match bob.recv().await {
        ServerMsg::PeerAvailable { addr, .. } => assert_eq!(addr, new_addr),
        other => panic!("expected PeerAvailable, got {other:?}"),
    }
}

#[tokio::test]
async fn hello_for_another_endpoint_is_a_protocol_error() {
    let server = dev_server().await;
    let mut client = Client::connect(&server, "alice").await;
    let ClientMsg::Hello(mut hello) = client.hello(None) else {
        unreachable!()
    };
    hello.addr = EndpointAddr::new(SecretKey::generate().public());
    client.send(ClientMsg::Hello(hello)).await;
    assert_eq!(client.close_code().await, code(CloseCode::ProtocolError));
}

#[tokio::test]
async fn updates_before_registering_are_a_protocol_error() {
    let server = dev_server().await;
    let mut client = Client::connect(&server, "alice").await;
    client.send(ClientMsg::AddPairs(vec![])).await;
    assert_eq!(client.close_code().await, code(CloseCode::ProtocolError));
}

/// An address with one IP address more than the rendezvous accepts.
fn oversize_addr(id: EndpointId) -> EndpointAddr {
    let mut addr = EndpointAddr::new(id);
    for port in 0..=rdv::MAX_IP_ADDRS as u16 {
        addr = addr.with_ip_addr(([127, 0, 0, 1], port).into());
    }
    addr
}

#[tokio::test]
async fn oversize_hello_address_is_a_protocol_error() {
    let server = dev_server().await;
    let mut client = Client::connect(&server, "alice").await;
    let ClientMsg::Hello(mut hello) = client.hello(None) else {
        unreachable!()
    };
    hello.addr = oversize_addr(client.id());
    client.send(ClientMsg::Hello(hello)).await;
    assert_eq!(client.close_code().await, code(CloseCode::ProtocolError));
}

#[tokio::test]
async fn oversize_address_update_closes_only_the_sender() {
    let server = dev_server().await;
    let mut alice = Client::connect(&server, "alice").await;
    let mut bob = Client::connect(&server, "bob").await;
    let mut carol = Client::connect(&server, "carol").await;
    alice.register().await;
    bob.register().await;
    carol.register().await;
    alice
        .send(ClientMsg::AddPairs(vec![alice.token_for(&bob)]))
        .await;
    bob.send(ClientMsg::AddPairs(vec![bob.token_for(&alice)]))
        .await;
    alice.expect_peer_available(&bob).await;
    bob.expect_peer_available(&alice).await;

    alice
        .send(ClientMsg::UpdateAddr(oversize_addr(alice.id())))
        .await;
    assert_eq!(alice.close_code().await, code(CloseCode::ProtocolError));

    // Bob's session carries on: he hears that Alice left and gets new matches.
    match bob.recv().await {
        ServerMsg::PeerGone(id) => assert_eq!(id, alice.id()),
        other => panic!("expected PeerGone, got {other:?}"),
    }
    bob.send(ClientMsg::AddPairs(vec![bob.token_for(&carol)]))
        .await;
    carol
        .send(ClientMsg::AddPairs(vec![carol.token_for(&bob)]))
        .await;
    bob.expect_peer_available(&carol).await;
}

#[tokio::test]
async fn unregistered_connections_are_capped() {
    let limits = Limits {
        max_unregistered: 1,
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect(&server, "alice").await;

    let endpoint = loopback_endpoint().await;
    let connect = endpoint.connect(server.endpoint_addr(), rdv::ALPN);
    let refused = tokio::time::timeout(TIMEOUT, connect)
        .await
        .expect("connect timed out");
    assert!(refused.is_err());

    // Registering frees the slot.
    alice.register().await;
    let mut bob = Client::connect(&server, "bob").await;
    bob.register().await;
}

#[tokio::test]
async fn too_many_pairs_closes_the_session() {
    let limits = Limits {
        max_pairs: 2,
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut client = Client::connect(&server, "alice").await;
    client.register().await;
    let uuid = client.uuid();
    let token = |n| PairToken::new(uuid, Uuid::from_u128(n));
    client
        .send(ClientMsg::SetPairs(vec![token(1), token(2)]))
        .await;
    client.send(ClientMsg::AddPairs(vec![token(3)])).await;
    assert_eq!(client.close_code().await, code(CloseCode::LimitExceeded));
}

#[tokio::test]
async fn pair_updates_are_rate_limited() {
    let limits = Limits {
        pair_updates_per_sec: 3,
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut client = Client::connect(&server, "alice").await;
    client.register().await;
    for _ in 0..4 {
        client.send(ClientMsg::AddPairs(vec![])).await;
    }
    assert_eq!(client.close_code().await, code(CloseCode::RateLimited));
}

#[tokio::test]
async fn mojang_challenge_issues_a_real_ticket() {
    let mojang = FakeMojang::start().await;
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    let ticket = alice.expect_registered().await;

    let ticket = TicketVerifier::new([server.issuer_id()])
        .verify(&ticket, SystemTime::now())
        .unwrap();
    assert!(!ticket.dev);
    assert_eq!(ticket.uuid, alice.uuid());
    let remaining = ticket.remaining(SystemTime::now());
    assert!(
        remaining > Duration::from_secs(23 * 3600) && remaining <= Duration::from_secs(24 * 3600)
    );
    assert_eq!(mojang.requests().len(), 1);
    assert!(mojang.requests()[0].contains("username=alice&serverId="));
}

#[tokio::test]
async fn cached_ticket_skips_the_challenge() {
    let mojang = FakeMojang::start().await;
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    let ticket = alice.expect_registered().await;

    // Reconnect from the same endpoint with the cached ticket.
    let conn = alice
        .endpoint
        .connect(server.endpoint_addr(), rdv::ALPN)
        .await;
    let (send, recv) = conn.as_ref().unwrap().open_bi().await.unwrap();
    alice.conn = conn.unwrap();
    alice.send = send;
    alice.recv = recv;
    alice.send(alice.hello(Some(ticket.clone()))).await;
    let reused = alice.expect_registered().await;
    assert_eq!(reused.to_bytes(), ticket.to_bytes());
    assert_eq!(mojang.requests().len(), 1);
    wait_for("the old session to be replaced", || {
        server.stats().sessions == 1
    })
    .await;
}

#[tokio::test]
async fn wrong_uuid_fails_auth() {
    let mojang = FakeMojang::start().await;
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    let ClientMsg::Hello(mut hello) = alice.hello(None) else {
        unreachable!()
    };
    hello.uuid = uuid_for("notch");
    alice.send(ClientMsg::Hello(hello)).await;
    alice.answer_challenge().await;
    assert_eq!(alice.close_code().await, code(CloseCode::AuthFailed));
}

#[tokio::test]
async fn unconfirmed_join_fails_auth() {
    let mojang = FakeMojang::start().await;
    mojang.script([response("204 No Content", "", "")]);
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    assert_eq!(alice.close_code().await, code(CloseCode::AuthFailed));
}

#[tokio::test]
async fn mojang_rate_limit_answers_retry_after() {
    let mojang = FakeMojang::start().await;
    mojang.script([response("429 Too Many Requests", "Retry-After: 60\r\n", "")]);
    let server = mojang_server(&mojang).await;

    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    assert!(matches!(
        alice.recv().await,
        ServerMsg::RetryAfter { secs: 60 }
    ));
    assert_eq!(alice.close_code().await, code(CloseCode::RateLimited));

    // Others are told to wait without being challenged or calling Mojang.
    let mut bob = Client::connect(&server, "bob").await;
    bob.send(bob.hello(None)).await;
    assert!(matches!(
        bob.recv().await,
        ServerMsg::RetryAfter { secs: 59 | 60 }
    ));
    assert_eq!(mojang.requests().len(), 1);
}

#[tokio::test]
async fn auth_attempts_are_rate_limited_per_endpoint() {
    let mojang = FakeMojang::start().await;
    let limits = Limits {
        auth_per_endpoint_per_min: 1,
        ..Limits::default()
    };
    let server = builder()
        .session_server(SessionServer::new(&mojang.url))
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    alice.expect_registered().await;

    alice.send(ClientMsg::Renew).await;
    assert_eq!(alice.close_code().await, code(CloseCode::RateLimited));
}

#[tokio::test]
async fn challenges_beyond_the_mojang_budget_are_told_to_retry() {
    let mojang = FakeMojang::start().await;
    let limits = Limits {
        mojang_checks_per_min: 1,
        ..Limits::default()
    };
    let server = builder()
        .session_server(SessionServer::new(&mojang.url))
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    alice.expect_registered().await;

    // A fresh key gets past the per-EndpointId limit, but not the budget.
    let mut bob = Client::connect(&server, "bob").await;
    bob.send(bob.hello(None)).await;
    assert!(matches!(
        bob.recv().await,
        ServerMsg::RetryAfter { secs: 59 | 60 }
    ));
    assert_eq!(bob.close_code().await, code(CloseCode::RateLimited));
    assert_eq!(mojang.requests().len(), 1);
}

#[tokio::test]
async fn challenges_without_a_known_ip_share_a_budget() {
    let mojang = FakeMojang::start().await;
    let limits = Limits {
        unknown_ip_challenges_per_min: 1,
        ..Limits::default()
    };
    let server = builder()
        .session_server(SessionServer::new(&mojang.url))
        .relay(relay_options())
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect_via_relay(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    alice.expect_registered().await;

    let mut bob = Client::connect_via_relay(&server, "bob").await;
    bob.send(bob.hello(None)).await;
    assert!(matches!(
        bob.recv().await,
        ServerMsg::RetryAfter { secs: 59 | 60 }
    ));

    // Direct connections are covered by the per-IP limit instead.
    let mut carol = Client::connect(&server, "carol").await;
    carol.send(carol.hello(None)).await;
    carol.answer_challenge().await;
    carol.expect_registered().await;
}

#[tokio::test]
async fn renew_issues_a_new_ticket_while_pairs_keep_flowing() {
    let mojang = FakeMojang::start().await;
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    alice.expect_registered().await;

    alice.send(ClientMsg::Renew).await;
    assert!(matches!(alice.recv().await, ServerMsg::Challenge(_)));
    // Updates sent while the player calls joinServer are still handled.
    alice.send(ClientMsg::AddPairs(vec![])).await;
    alice.send(ClientMsg::Joined).await;
    alice.expect_registered().await;
    assert_eq!(mojang.requests().len(), 2);
    assert_eq!(server.stats().sessions, 1);
}

#[tokio::test]
async fn renew_during_rate_limit_keeps_the_session() {
    let mojang = FakeMojang::start().await;
    let server = mojang_server(&mojang).await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.send(alice.hello(None)).await;
    alice.answer_challenge().await;
    alice.expect_registered().await;

    mojang.script([response("429 Too Many Requests", "Retry-After: 5\r\n", "")]);
    alice.send(ClientMsg::Renew).await;
    alice.answer_challenge().await;
    assert!(matches!(
        alice.recv().await,
        ServerMsg::RetryAfter { secs: 5 }
    ));
    alice.send(ClientMsg::AddPairs(vec![])).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.stats().sessions, 1);
}

#[tokio::test]
async fn relay_only_client_reaches_the_rendezvous_through_the_relay() {
    let server = builder()
        .insecure_dev_auth()
        .relay(relay_options())
        .spawn()
        .await
        .unwrap();
    assert!(server.endpoint_addr().relay_urls().next().is_some());
    let mut alice = Client::connect_via_relay(&server, "alice").await;
    alice.register().await;
    assert_eq!(server.stats().sessions, 1);
}

fn relay_disconnects(server: &Server) -> u64 {
    let relay = server.relay.as_ref().unwrap();
    relay.metrics().server.disconnects.get()
}

#[tokio::test]
async fn relay_drops_clients_that_never_register() {
    let limits = Limits {
        relay_grace: Duration::from_millis(300),
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .relay(relay_options())
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let lurker = relay_only_endpoint(server.relay_url().unwrap()).await;
    tokio::time::timeout(TIMEOUT, lurker.online())
        .await
        .unwrap();
    wait_for("the lurker to be disconnected", || {
        relay_disconnects(&server) >= 1
    })
    .await;
}

#[tokio::test]
async fn open_relay_keeps_clients_without_a_session() {
    let limits = Limits {
        relay_grace: Duration::from_millis(300),
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .relay(RelayOptions {
            open: true,
            ..relay_options()
        })
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let caller = relay_only_endpoint(server.relay_url().unwrap()).await;
    tokio::time::timeout(TIMEOUT, caller.online())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(relay_disconnects(&server), 0);
}

#[tokio::test]
async fn relay_keeps_registered_clients_and_drops_them_when_the_session_ends() {
    let limits = Limits {
        relay_grace: Duration::from_secs(2),
        ..Limits::default()
    };
    let server = builder()
        .insecure_dev_auth()
        .relay(relay_options())
        .limits(limits)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect_via_relay(&server, "alice").await;
    alice.register().await;

    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(relay_disconnects(&server), 0);

    alice.send.finish().unwrap();
    wait_for("alice's relay connection to drop", || {
        relay_disconnects(&server) >= 1
    })
    .await;
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let server = builder()
        .insecure_dev_auth()
        .metrics(addr)
        .spawn()
        .await
        .unwrap();
    let mut alice = Client::connect(&server, "alice").await;
    alice.register().await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("\nyakvc_sessions 1\n"));
    assert!(response.contains("\nyakvc_auth_ok_total 1\n"));
}

#[tokio::test]
async fn shutdown_closes_sessions() {
    let server = dev_server().await;
    let mut alice = Client::connect(&server, "alice").await;
    alice.register().await;
    server.shutdown().await;
    assert_eq!(alice.close_code().await, code(CloseCode::ShuttingDown));
}

/// A `[::]` bind serves IPv4 too, for the rendezvous (where iroh needs a
/// second socket) and the relay's listener (dual-stack on its own).
#[tokio::test]
async fn wildcard_ipv6_binds_serve_both_families() {
    let port = std::net::UdpSocket::bind("[::]:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server = Server::builder(SecretKey::generate(), IssuerKey::generate())
        .bind(format!("[::]:{port}").parse().unwrap())
        .insecure_dev_auth()
        .relay(RelayOptions {
            http_bind: "[::]:0".parse().unwrap(),
            ..relay_options()
        })
        .spawn()
        .await
        .unwrap();

    for (client_bind, server_ip) in [("127.0.0.1:0", "127.0.0.1"), ("[::1]:0", "::1")] {
        let endpoint = Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr(client_bind)
            .unwrap()
            .bind()
            .await
            .unwrap();
        let server_addr = std::net::SocketAddr::new(server_ip.parse().unwrap(), port);
        let addr = EndpointAddr::new(server.endpoint_id()).with_ip_addr(server_addr);
        let mut client = Client::connect_with(endpoint, addr, server_ip).await;
        client.register().await;
    }

    // The relay URL dials the `[::]` listener over 127.0.0.1.
    let mut relayed = Client::connect_via_relay(&server, "relayed").await;
    relayed.register().await;
    let relay_port = server.relay_url().unwrap().port().unwrap();
    tokio::net::TcpStream::connect(("::1", relay_port))
        .await
        .expect("relay listens on IPv6");
}

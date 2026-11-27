//! The public room list end to end: an in-process server with the list on,
//! and clients speaking `yakvc/rooms/1` through `yakvc_client::rooms`.

use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::presets;
use yakvc_client::rooms::{Refusal, Room, RoomList, RoomsError};
use yakvc_proto::{EndpointAddr, IssuerKey, SecretKey};
use yakvc_server::{RoomOptions, Server};

const TIMEOUT: Duration = Duration::from_secs(10);

async fn server(options: Option<RoomOptions>) -> Server {
    let mut builder = Server::builder(SecretKey::generate(), IssuerKey::generate())
        .bind("127.0.0.1:0".parse().unwrap())
        .insecure_dev_auth();
    if let Some(options) = options {
        builder = builder.rooms(options);
    }
    builder.spawn().await.unwrap()
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

async fn connect(server: &Server) -> Result<RoomList, RoomsError> {
    connect_to(server.endpoint_addr()).await
}

/// Each client on its own endpoint, which must outlive its list: closing
/// the endpoint ends the connection without telling the server.
async fn connect_to(addr: EndpointAddr) -> Result<RoomList, RoomsError> {
    let endpoint = loopback_endpoint().await;
    let list = RoomList::connect(&endpoint, addr).await;
    // Leaked: a test's few endpoints go when the process does.
    std::mem::forget(endpoint);
    list
}

fn room(label: &str, code: &str) -> Room {
    Room {
        label: label.to_owned(),
        code: code.to_owned(),
    }
}

/// Waits until `list` shows exactly `rooms`.
async fn wait_for(list: &RoomList, rooms: &[Room]) {
    let mut watch = list.watch();
    let shown = tokio::time::timeout(TIMEOUT, watch.wait_for(|now| now == rooms)).await;
    assert!(
        matches!(shown, Ok(Ok(_))),
        "list never became {rooms:?}, is {:?}",
        list.rooms()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_room_lives_as_long_as_its_connection() {
    let server = server(Some(RoomOptions::default())).await;
    let host = connect(&server).await.unwrap();
    let lobby = room("Lobby", "code-a");
    host.announce(lobby.clone()).await.unwrap();

    // A client that connects later gets it in the first list.
    let guest = connect(&server).await.unwrap();
    assert_eq!(guest.rooms(), vec![lobby.clone()]);

    // Changes reach it as they happen.
    let other = room("Other", "code-b");
    host.announce(other.clone()).await.unwrap();
    wait_for(&guest, &[lobby.clone(), other]).await;
    host.withdraw("code-b").await.unwrap();
    wait_for(&guest, &[lobby]).await;

    host.close().await;
    wait_for(&guest, &[]).await;

    // Dropping the list without closing takes the rooms too.
    let host = connect(&server).await.unwrap();
    host.announce(room("Again", "code-c")).await.unwrap();
    wait_for(&guest, &[room("Again", "code-c")]).await;
    drop(host);
    wait_for(&guest, &[]).await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_reach_the_announcer() {
    let server = server(Some(RoomOptions {
        max_rooms_per_connection: 1,
        ..RoomOptions::default()
    }))
    .await;
    let first = connect(&server).await.unwrap();
    let second = connect(&server).await.unwrap();
    first.announce(room("Mine", "code")).await.unwrap();

    let taken = second.announce(room("Also mine", "code")).await;
    assert!(
        matches!(taken, Err(RoomsError::Refused(Refusal::Taken))),
        "{taken:?}"
    );
    let many = first.announce(room("Second", "code-2")).await;
    let many_refused = matches!(many, Err(RoomsError::Refused(Refusal::TooManyRooms)));
    assert!(many_refused, "{many:?}");
    let long = first.announce(room(&"x".repeat(65), "code")).await;
    assert!(
        matches!(long, Err(RoomsError::Refused(Refusal::Invalid))),
        "{long:?}"
    );

    // Relabelling one's own room is not a second room.
    first.announce(room("Renamed", "code")).await.unwrap();
    wait_for(&second, &[room("Renamed", "code")]).await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_per_ip_are_capped() {
    let server = server(Some(RoomOptions {
        max_connections_per_ip: 2,
        ..RoomOptions::default()
    }))
    .await;
    let _first = connect(&server).await.unwrap();
    let second = connect(&server).await.unwrap();
    assert!(connect(&server).await.is_err());
    drop(second);
    // The slot comes back once the server has seen the connection go.
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while connect(&server).await.is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "slot never came back"
        );
    }
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_are_capped() {
    let server = server(Some(RoomOptions {
        max_connections: 1,
        ..RoomOptions::default()
    }))
    .await;
    let _first = connect(&server).await.unwrap();
    assert!(connect(&server).await.is_err());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_off_unless_asked_for() {
    let off = server(None).await;
    assert!(connect(&off).await.is_err());
    off.shutdown().await;

    let disabled = server(Some(RoomOptions {
        enabled: false,
        ..RoomOptions::default()
    }))
    .await;
    assert!(connect(&disabled).await.is_err());
    disabled.shutdown().await;
}

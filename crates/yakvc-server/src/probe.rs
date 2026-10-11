//! A health check that signs in like a real client: through the relay, by
//! hostname, as an offline player. It declines the challenge, so the server
//! makes no Mojang call and no account is involved.

use std::time::Duration;

use anyhow::{Context, bail};
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode};
use yakvc_proto::rdv::{self, ClientMsg, Hello, ServerMsg};
use yakvc_proto::wire::{read_msg, write_msg};
use yakvc_proto::{EndpointAddr, EndpointId, RelayUrl, offline_uuid};

/// Long enough for a relay connection and a sign-in over a slow link.
const TIMEOUT: Duration = Duration::from_secs(20);

const NAME: &str = "YakProbe";

/// Signs in to the rendezvous `id` through `relay` and returns once it is
/// registered.
pub async fn probe(id: EndpointId, relay: RelayUrl) -> anyhow::Result<()> {
    tokio::time::timeout(TIMEOUT, sign_in(id, relay))
        .await
        .context("timed out")?
}

async fn sign_in(id: EndpointId, relay: RelayUrl) -> anyhow::Result<()> {
    // Relay only, as clients reach the built-in server: no IP address.
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::custom([relay.clone()]))
        .bind()
        .await?;
    let addr = EndpointAddr::new(id).with_relay_url(relay);
    let conn = endpoint
        .connect(addr, rdv::ALPN)
        .await
        .context("connecting")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let hello = Hello {
        mod_version: concat!("probe-", env!("CARGO_PKG_VERSION")).into(),
        uuid: offline_uuid(NAME),
        name: NAME.into(),
        addr: EndpointAddr::new(endpoint.id()),
        cached_ticket: None,
    };
    write_msg(&mut send, &ClientMsg::Hello(hello)).await?;
    loop {
        let msg: ServerMsg = read_msg(&mut recv)
            .await?
            .context("server ended the stream")?;
        match msg {
            ServerMsg::Challenge(_) => write_msg(&mut send, &ClientMsg::Decline).await?,
            ServerMsg::Registered(_) => break,
            other => bail!("unexpected reply: {other:?}"),
        }
    }
    conn.close(0u32.into(), b"probe done");
    endpoint.close().await;
    Ok(())
}

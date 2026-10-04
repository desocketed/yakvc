//! Rendezvous server: authenticates clients with a Mojang session challenge,
//! issues tickets, matches pair tokens and optionally embeds an `iroh-relay`.
//!
//! [`Server::builder`] is the whole API; the binary only parses arguments and
//! loads [`Config`], and `yakvc-testkit` builds servers in-process.

mod auth;
mod config;
mod limits;
mod matcher;
mod metrics;
mod mojang;
mod rdv;
mod relay;
mod server;
mod sessions;

pub use crate::config::{Config, ConfigError};
pub use crate::mojang::{MojangError, Profile, SessionServer};
pub use crate::server::{Limits, RelayOptions, RelayTls, Server, ServerBuilder, SpawnError, Stats};

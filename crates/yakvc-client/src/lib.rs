//! The Yak VC voice engine.
//!
//! [`Engine`] is the whole client: Java (through `yakvc-ffi`) and the CLI feed
//! it identity, tab list, world snapshots and input, and read [`Event`]s back.
//! It owns its Tokio runtime and audio thread; every method is non-blocking
//! and callable from any thread.
//!
//! Internally it is split into [`net`] (Iroh endpoint, rendezvous session,
//! verified peers, protocol routing) and `voice` (the one [`net::Protocol`]
//! in v1). `net` knows nothing about audio.

mod config;
mod engine;
mod event;
pub mod net;
#[cfg(feature = "sim")]
pub mod sim;
mod voice;
mod world;

pub use yakvc_audio::{DeviceChoice, DeviceInfo, Devices, FrameSink, FrameSource, StreamStats};
pub use yakvc_shared::{EndpointAddr, Uuid};

pub use crate::config::{Activation, AudioConfig, Config, ConfigError, RendezvousConfig};
pub use crate::engine::{Engine, EngineBuilder, NetReport, PeerAudio, PeerInfo, StartError};
pub use crate::event::{Event, Events, JoinId, PeerState, RendezvousState};
pub use crate::world::{Input, Pose, Vec3, World};

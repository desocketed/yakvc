//! The Yak VC voice engine.
//!
//! Split into `net` (Iroh endpoint, rendezvous session, peers, protocol
//! routing) and `voice` (send/receive loop, recipient selection, spatial
//! input). `net` knows nothing about audio.

pub mod net;
pub mod voice;

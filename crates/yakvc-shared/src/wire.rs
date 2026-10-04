//! Framing for control streams: a varint length prefix, then postcard bytes.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};

/// Largest accepted message. Fits a full `SetPairs` (2,048 tokens).
pub const MAX_MESSAGE_LEN: usize = 128 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("message of {0} bytes exceeds the limit")]
    TooLarge(usize),
    #[error("malformed message")]
    Decode(#[from] postcard::Error),
}

/// Writes one framed message.
pub async fn write_msg<M: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    msg: &M,
) -> Result<(), WireError> {
    let _ = (stream, msg);
    todo!()
}

/// Reads one framed message. Returns `None` if the stream ended cleanly
/// between messages.
pub async fn read_msg<M: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Option<M>, WireError> {
    let _ = stream;
    todo!()
}

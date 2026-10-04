//! Framing for control streams: a varint length prefix, then postcard bytes.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
    let body = postcard::to_stdvec(msg)?;
    if body.len() > MAX_MESSAGE_LEN {
        return Err(WireError::TooLarge(body.len()));
    }
    let mut frame = encode_varint(body.len());
    frame.extend_from_slice(&body);
    stream.write_all(&frame).await?;
    stream.flush().await?;
    Ok(())
}

/// Reads one framed message. Returns `None` if the stream ended cleanly
/// between messages.
pub async fn read_msg<M: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Option<M>, WireError> {
    let Some(len) = read_len(stream).await? else {
        return Ok(None);
    };
    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    Ok(Some(postcard::from_bytes(&body)?))
}

/// LEB128, the same varint postcard uses inside messages.
fn encode_varint(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Reads the length prefix. Stops as soon as the length is known to be over
/// the limit, so a hostile prefix can't make us allocate or read further.
async fn read_len(stream: &mut (impl AsyncRead + Unpin)) -> Result<Option<usize>, WireError> {
    let mut len: usize = 0;
    let mut shift = 0;
    loop {
        let byte = match stream.read_u8().await {
            Ok(byte) => byte,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && shift == 0 => {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        len |= usize::from(byte & 0x7f) << shift;
        if len > MAX_MESSAGE_LEN {
            return Err(WireError::TooLarge(len));
        }
        if byte & 0x80 == 0 {
            return Ok(Some(len));
        }
        shift += 7;
        // Three bytes hold any allowed length, so a longer prefix is padding
        // (0x80 0x80 ...) that no encoder produces.
        if shift > 14 {
            return Err(WireError::TooLarge(len));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pair::PairToken;
    use crate::rdv::ClientMsg;
    use uuid::Uuid;

    async fn encode(msg: &ClientMsg) -> Vec<u8> {
        let mut buf = Vec::new();
        write_msg(&mut buf, msg).await.unwrap();
        buf
    }

    fn tokens(n: u128) -> Vec<PairToken> {
        (0..n)
            .map(|i| PairToken::new(Uuid::from_u128(0), Uuid::from_u128(i)))
            .collect()
    }

    #[tokio::test]
    async fn messages_round_trip_back_to_back() {
        let mut buf = encode(&ClientMsg::Joined).await;
        buf.extend(encode(&ClientMsg::AddPairs(tokens(3))).await);

        let mut reader = &buf[..];
        let first: ClientMsg = read_msg(&mut reader).await.unwrap().unwrap();
        let second: ClientMsg = read_msg(&mut reader).await.unwrap().unwrap();
        assert!(matches!(first, ClientMsg::Joined));
        assert!(matches!(second, ClientMsg::AddPairs(t) if t == tokens(3)));
        assert!(read_msg::<ClientMsg>(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn full_set_pairs_fits() {
        let buf = encode(&ClientMsg::SetPairs(tokens(2048))).await;
        let msg: ClientMsg = read_msg(&mut &buf[..]).await.unwrap().unwrap();
        assert!(matches!(msg, ClientMsg::SetPairs(t) if t.len() == 2048));
    }

    #[tokio::test]
    async fn oversized_write_is_refused() {
        let mut buf = Vec::new();
        let err = write_msg(&mut buf, &ClientMsg::SetPairs(tokens(5000))).await;
        assert!(matches!(err, Err(WireError::TooLarge(_))));
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn oversized_prefix_is_refused_before_reading_the_body() {
        let prefix = encode_varint(MAX_MESSAGE_LEN + 1);
        let err = read_msg::<ClientMsg>(&mut &prefix[..]).await;
        assert!(matches!(err, Err(WireError::TooLarge(n)) if n == MAX_MESSAGE_LEN + 1));

        // Exactly the limit passes the prefix check; the missing body is then
        // an I/O error.
        let prefix = encode_varint(MAX_MESSAGE_LEN);
        let err = read_msg::<ClientMsg>(&mut &prefix[..]).await;
        assert!(matches!(err, Err(WireError::Io(_))));
    }

    #[tokio::test]
    async fn padded_prefix_is_refused() {
        let prefix = [0x80u8; 16];
        let err = read_msg::<ClientMsg>(&mut &prefix[..]).await;
        assert!(matches!(err, Err(WireError::TooLarge(_))));
    }

    #[tokio::test]
    async fn truncated_message_is_an_io_error() {
        let buf = encode(&ClientMsg::AddPairs(tokens(1))).await;
        let err = read_msg::<ClientMsg>(&mut &buf[..buf.len() - 1]).await;
        assert!(matches!(err, Err(WireError::Io(_))));

        // Cut inside a two-byte prefix.
        let prefix = encode_varint(300);
        let err = read_msg::<ClientMsg>(&mut &prefix[..1]).await;
        assert!(matches!(err, Err(WireError::Io(_))));
    }

    #[tokio::test]
    async fn garbage_is_a_decode_error() {
        let buf = [1u8, 0xff];
        let err = read_msg::<ClientMsg>(&mut &buf[..]).await;
        assert!(matches!(err, Err(WireError::Decode(_))));
    }

    #[test]
    fn varint_matches_postcard() {
        for n in [0usize, 1, 127, 128, 300, MAX_MESSAGE_LEN] {
            let postcard = postcard::to_stdvec(&(n as u64)).unwrap();
            assert_eq!(encode_varint(n), postcard, "{n}");
        }
    }
}

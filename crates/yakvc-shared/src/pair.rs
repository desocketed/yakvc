//! Pair tokens: how two clients find out they see each other in the tab list
//! without telling the rendezvous anyone else's UUID in the clear.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// `SHA-256("yakvc-pair-v1" ‖ min(a, b) ‖ max(a, b))`. Symmetric:
/// `PairToken::new(a, b) == PairToken::new(b, a)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PairToken([u8; 32]);

impl PairToken {
    pub fn new(a: Uuid, b: Uuid) -> Self {
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        let hash = Sha256::new()
            .chain_update(b"yakvc-pair-v1")
            .chain_update(low.as_bytes())
            .chain_update(high.as_bytes())
            .finalize();
        PairToken(hash.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetric_and_distinct() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        assert_eq!(PairToken::new(a, b), PairToken::new(b, a));
        assert_ne!(PairToken::new(a, b), PairToken::new(a, c));
        assert_ne!(PairToken::new(a, b), PairToken::new(b, c));
    }

    #[test]
    fn matches_the_documented_formula() {
        let a = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let b = Uuid::from_u128(5);
        let mut input = b"yakvc-pair-v1".to_vec();
        input.extend_from_slice(b.as_bytes()); // b < a
        input.extend_from_slice(a.as_bytes());
        let expected: [u8; 32] = Sha256::digest(&input).into();
        assert_eq!(PairToken::new(a, b), PairToken(expected));
    }
}

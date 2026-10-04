//! Pair tokens: how two clients find out they see each other in the tab list
//! without telling the rendezvous anyone else's UUID in the clear.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `SHA-256("yakvc-pair-v1" ‖ min(a, b) ‖ max(a, b))`. Symmetric:
/// `PairToken::new(a, b) == PairToken::new(b, a)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PairToken([u8; 32]);

impl PairToken {
    pub fn new(a: Uuid, b: Uuid) -> Self {
        let _ = (a, b);
        todo!()
    }
}

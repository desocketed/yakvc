//! Group voice chat: a group's random key, the id and member proofs made
//! from it, and what members tell each other. See DESIGN.md, "Group voice
//! chat".

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Longest group label, in characters.
pub const MAX_LABEL_CHARS: usize = 32;

/// A group's public id, which the voice menu lists and joins by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupId(pub [u8; 32]);

/// Lowercase hex, as the voice menu shows and passes it back.
impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl GroupId {
    /// Parses the 64 hex characters [`Display`](std::fmt::Display) writes.
    pub fn from_hex(hex: &str) -> Option<GroupId> {
        if hex.len() != 64 || !hex.is_ascii() {
            return None;
        }
        let mut id = [0; 32];
        for (byte, pair) in id.iter_mut().zip(hex.as_bytes().chunks(2)) {
            let pair = std::str::from_utf8(pair).ok()?;
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(GroupId(id))
    }
}

/// What makes a group: a random key that every member holds, so none of
/// them has any power the others lack.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupKey(pub [u8; 32]);

impl GroupKey {
    /// A new group's key. `rand::random` draws from a CSPRNG seeded by the
    /// OS, so nobody can guess another group's key.
    pub fn random() -> GroupKey {
        GroupKey(rand::random())
    }

    pub fn id(&self) -> GroupId {
        let id = Sha256::new()
            .chain_update(b"yakvc-group-id-v2")
            .chain_update(self.0)
            .finalize();
        GroupId(id.into())
    }

    /// Proves that `member` holds the key. Bound to the member's UUID, so
    /// another player can't reuse it, and meaningless without the key, so
    /// it doesn't reveal which group it belongs to.
    pub fn proof(&self, member: Uuid) -> [u8; 32] {
        Sha256::new()
            .chain_update(b"yakvc-group-member-v2")
            .chain_update(self.0)
            .chain_update(member.as_bytes())
            .finalize()
            .into()
    }

    /// Whether `from` is in this group, by its announcement.
    pub fn has_member(&self, from: Uuid, announce: &GroupAnnounce) -> bool {
        announce.proof == self.proof(from)
    }
}

/// Never printed, since knowing it is membership.
impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupKey").finish_non_exhaustive()
    }
}

/// Which group a peer is in, as it tells every peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAnnounce {
    /// [`GroupKey::proof`] for the sender.
    pub proof: [u8; 32],
    /// Set for a public group, which anyone may join.
    pub public: Option<PublicGroup>,
}

/// A public group's key and label, announced so others can join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicGroup {
    pub key: GroupKey,
    pub label: String,
}

/// What members share about their group. Any member may change it; the
/// highest `(version, by)` wins, so every member ends up with the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupState {
    pub version: u64,
    pub public: bool,
    /// Empty for none; at most [`MAX_LABEL_CHARS`].
    pub label: String,
    /// Who made this version. Travels with the state, because mates pass it
    /// on and the tie-break must be the same on every member.
    pub by: Uuid,
}

impl GroupState {
    /// A new group's state: private, no label.
    pub fn new(by: Uuid) -> GroupState {
        GroupState {
            version: 0,
            public: false,
            label: String::new(),
            by,
        }
    }

    /// Whether this state replaces `other`.
    pub fn newer_than(&self, other: &GroupState) -> bool {
        (self.version, self.by) > (other.version, other.by)
    }

    /// The label fits. Peers' states are checked with this before use.
    pub fn is_valid(&self) -> bool {
        label_fits(&self.label)
    }
}

pub fn label_fits(label: &str) -> bool {
    label.chars().count() <= MAX_LABEL_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_random_and_ids_differ() {
        let (a, b) = (GroupKey::random(), GroupKey::random());
        assert_ne!(a, b);
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), a.clone().id());
    }

    #[test]
    fn ids_round_trip_through_hex() {
        let id = GroupKey::random().id();
        assert_eq!(GroupId::from_hex(&id.to_string()), Some(id));
        assert_eq!(GroupId::from_hex("00"), None);
        assert_eq!(GroupId::from_hex(&"zz".repeat(32)), None);
        assert_eq!(GroupId::from_hex(&"é".repeat(32)), None);
    }

    #[test]
    fn only_a_members_own_proof_counts() {
        let group = GroupKey::random();
        let (alice, bob) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let announce = GroupAnnounce {
            proof: group.proof(alice),
            public: None,
        };
        assert!(group.has_member(alice, &announce));
        // Bob copying Alice's announcement proves nothing about Bob.
        assert!(!group.has_member(bob, &announce));
        // Another key gives another proof.
        assert!(!GroupKey::random().has_member(alice, &announce));
    }

    #[test]
    fn the_highest_version_then_uuid_wins() {
        let (alice, bob) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let base = GroupState::new(alice);
        let next = GroupState {
            version: 1,
            ..GroupState::new(alice)
        };
        assert!(next.newer_than(&base));
        assert!(!base.newer_than(&next));
        assert!(!base.newer_than(&base));
        let tie = GroupState {
            version: 1,
            ..GroupState::new(bob)
        };
        assert!(tie.newer_than(&next));
        assert!(!next.newer_than(&tie));
    }

    #[test]
    fn labels_must_fit() {
        assert!(label_fits(""));
        assert!(label_fits(&"é".repeat(MAX_LABEL_CHARS)));
        assert!(!label_fits(&"x".repeat(MAX_LABEL_CHARS + 1)));
    }
}

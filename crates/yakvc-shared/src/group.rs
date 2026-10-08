//! Group voice chat: how a group's name and password become its key, its
//! public id and its members' proofs. See DESIGN.md, "Group voice chat".

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Longest group name, in characters.
pub const MAX_NAME_CHARS: usize = 32;

/// A group's public id, which peers announce and the voice menu lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupId(pub [u8; 32]);

/// Lowercase hex, as the voice menu shows and compares it.
impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// What only members know: the key derived from the name and password.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupKey {
    key: [u8; 32],
    name: String,
}

impl GroupKey {
    /// The group with this name and password (empty for an open group).
    /// `None` if the name is empty or too long once trimmed.
    pub fn new(name: &str, password: &str) -> Option<GroupKey> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return None;
        }
        let key = Sha256::new()
            .chain_update(b"yakvc-group-key-v1")
            .chain_update((name.len() as u32).to_be_bytes())
            .chain_update(name.as_bytes())
            .chain_update(password.as_bytes())
            .finalize();
        Some(GroupKey {
            key: key.into(),
            name: name.to_owned(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> GroupId {
        let id = Sha256::new()
            .chain_update(b"yakvc-group-id-v1")
            .chain_update(self.key)
            .finalize();
        GroupId(id.into())
    }

    /// Proves that `member` knows the key. Bound to the member's UUID, so
    /// another player can't reuse it.
    pub fn proof(&self, member: Uuid) -> [u8; 32] {
        Sha256::new()
            .chain_update(b"yakvc-group-member-v1")
            .chain_update(self.key)
            .chain_update(member.as_bytes())
            .finalize()
            .into()
    }

    /// The announcement members send their peers.
    pub fn announce(&self, member: Uuid, locked: bool) -> GroupAnnounce {
        GroupAnnounce {
            id: self.id(),
            name: self.name.clone(),
            locked,
            proof: self.proof(member),
        }
    }

    /// Whether `from` is in this group, by its announcement.
    pub fn has_member(&self, from: Uuid, announce: &GroupAnnounce) -> bool {
        announce.id == self.id() && announce.proof == self.proof(from)
    }
}

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupKey")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Which group a peer is in, as it tells its peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAnnounce {
    pub id: GroupId,
    pub name: String,
    /// The group has a password.
    pub locked: bool,
    /// [`GroupKey::proof`] for the sender.
    pub proof: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_password_make_the_group() {
        let a = GroupKey::new("Miners", "pw").unwrap();
        assert_eq!(a.id(), GroupKey::new("  Miners ", "pw").unwrap().id());
        assert_ne!(a.id(), GroupKey::new("Miners", "other").unwrap().id());
        assert_ne!(a.id(), GroupKey::new("Miners", "").unwrap().id());
        // The length prefix keeps name and password apart.
        assert_ne!(
            GroupKey::new("ab", "c").unwrap().id(),
            GroupKey::new("a", "bc").unwrap().id()
        );
    }

    #[test]
    fn names_must_fit() {
        assert!(GroupKey::new("   ", "").is_none());
        assert!(GroupKey::new(&"x".repeat(MAX_NAME_CHARS), "").is_some());
        assert!(GroupKey::new(&"x".repeat(MAX_NAME_CHARS + 1), "").is_none());
    }

    #[test]
    fn only_a_members_own_proof_counts() {
        let group = GroupKey::new("Miners", "pw").unwrap();
        let (alice, bob) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let announce = group.announce(alice, true);
        assert!(group.has_member(alice, &announce));
        // Bob copying Alice's announcement proves nothing about Bob.
        assert!(!group.has_member(bob, &announce));
        // Knowing the public id without the key isn't enough either.
        let guess = GroupKey::new("Miners", "guess").unwrap();
        let forged = GroupAnnounce {
            id: group.id(),
            ..guess.announce(bob, true)
        };
        assert!(!group.has_member(bob, &forged));
    }
}

//! Group voice chat (DESIGN.md "Group voice chat"): our group, its shared
//! state and invites, as plain rules over [`VoiceState`]. There is no
//! kicking: a member who doesn't want someone in the group leaves or mutes
//! them. The caller sends the events these return; the peer tasks send the
//! messages [`VoiceState::news_for`] picks.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use yakvc_shared::group::label_fits;
use yakvc_shared::voice::VoiceMsg;
use yakvc_shared::{GroupAnnounce, GroupId, GroupKey, GroupState, PublicGroup, Uuid};

use super::rules::VoiceState;
use crate::config::InvitesFrom;
use crate::event::{Event, GroupNoticeKind};

/// How long a received invite can be accepted.
const INVITE_LIFETIME: Duration = Duration::from_secs(120);

/// Why a group action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GroupError {
    #[error("no invite from that player in the last two minutes")]
    NoInvite,
    #[error("no public group with that id")]
    UnknownGroup,
    #[error("not in a group")]
    NotInGroup,
    #[error("the label is longer than 32 characters")]
    LabelTooLong,
}

/// The group we are in.
#[derive(Debug, Clone)]
pub(crate) struct OwnGroup {
    pub key: GroupKey,
    pub state: GroupState,
}

/// An invite we may accept.
#[derive(Debug, Clone)]
pub(crate) struct Invite {
    pub key: GroupKey,
    pub state: GroupState,
    pub received: Instant,
}

/// The group we invite players to while we are in none. We join it only
/// once one of them does, and reuse it for further invites until then.
#[derive(Debug, Clone)]
pub(crate) struct PendingGroup {
    pub key: GroupKey,
    pub state: GroupState,
    pub created: Instant,
}

/// An invite we sent, for the peer task to deliver.
#[derive(Debug, Clone)]
pub(crate) struct SentInvite {
    pub key: GroupKey,
    pub state: GroupState,
    pub sent: Instant,
}

/// A group someone on the server is in, for the voice menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInfo {
    pub id: GroupId,
    /// Empty for none.
    pub label: String,
    pub public: bool,
    /// We are in it.
    pub mine: bool,
    /// Members whose proof checks out, us included for our own group.
    pub members: Vec<Uuid>,
}

/// What a peer task has told its peer, so it sends only what changed.
#[derive(Debug, Clone)]
pub(crate) struct Told {
    pub wants_audio: bool,
    pub group: Option<GroupAnnounce>,
    /// Our group's state, as last sent while the peer was a group mate.
    pub state: Option<GroupState>,
    /// When the latest invite we delivered to the peer was made.
    pub invite: Option<Instant>,
}

impl Default for Told {
    /// What a peer assumes before we say anything: that we want its audio
    /// and are in no group.
    fn default() -> Self {
        Told {
            wants_audio: true,
            group: None,
            state: None,
            invite: None,
        }
    }
}

impl OwnGroup {
    pub(crate) fn new(key: GroupKey, state: GroupState) -> OwnGroup {
        OwnGroup { key, state }
    }

    pub(crate) fn id(&self) -> GroupId {
        self.key.id()
    }
}

impl VoiceState {
    /// Joins `group` (or leaves, with `None`). The mates found right after
    /// are already members, so they are not announced as joining.
    pub(crate) fn set_group(&mut self, group: Option<OwnGroup>) {
        self.group = group;
        self.group_mates.clear();
        self.mates_of = None;
        // Whatever we invited players to before, we are past it now.
        self.pending = None;
    }

    /// Invites `target` to our group, or while we are in none, to a new
    /// group we join once someone accepts. The peer task delivers it.
    pub(crate) fn invite(&mut self, target: Uuid, now: Instant) {
        let (key, state) = match &self.group {
            Some(own) => (own.key.clone(), own.state.clone()),
            None => {
                let fresh = |p: &PendingGroup| now.duration_since(p.created) < INVITE_LIFETIME;
                if !self.pending.as_ref().is_some_and(fresh) {
                    let me = self.own_uuid.unwrap_or_default();
                    self.pending = Some(PendingGroup {
                        key: GroupKey::random(),
                        state: GroupState::new(me),
                        created: now,
                    });
                }
                let pending = self.pending.as_ref().expect("just made");
                (pending.key.clone(), pending.state.clone())
            }
        };
        self.invites_sent
            .retain(|_, sent| now.duration_since(sent.sent) < INVITE_LIFETIME);
        self.invites_sent.insert(
            target,
            SentInvite {
                key,
                state,
                sent: now,
            },
        );
    }

    /// Joins the group we invited players to, once one of `announced`
    /// proves it accepted. Unlike other joins, the members found next are
    /// reported as joining, since they just did.
    pub(crate) fn join_pending(&mut self, announced: &[(Uuid, GroupAnnounce)]) {
        let Some(pending) = &self.pending else {
            return;
        };
        if self.group.is_some()
            || !announced
                .iter()
                .any(|(uuid, announce)| pending.key.has_member(*uuid, announce))
        {
            return;
        }
        let group = OwnGroup::new(pending.key.clone(), pending.state.clone());
        let id = group.id();
        self.set_group(Some(group));
        self.mates_of = Some(id);
    }

    /// The peers among `announced` that are in our group.
    pub(crate) fn mates_among(&self, announced: &[(Uuid, GroupAnnounce)]) -> HashSet<Uuid> {
        let Some(own) = &self.group else {
            return HashSet::new();
        };
        announced
            .iter()
            .filter(|(uuid, announce)| own.key.has_member(*uuid, announce))
            .filter(|(uuid, _)| Some(*uuid) != self.own_uuid)
            .map(|(uuid, _)| *uuid)
            .collect()
    }

    /// Replaces the group mates, reporting who joined and left unless our
    /// own group just changed. Returns whether they changed.
    pub(crate) fn update_mates(&mut self, mates: HashSet<Uuid>, events: &mut Vec<Event>) -> bool {
        let id = self.group.as_ref().map(OwnGroup::id);
        if self.mates_of.is_some() && self.mates_of == id {
            for &uuid in mates.difference(&self.group_mates) {
                events.push(notice(GroupNoticeKind::Joined, uuid, Uuid::nil()));
            }
            for &uuid in self.group_mates.difference(&mates) {
                events.push(notice(GroupNoticeKind::Left, uuid, Uuid::nil()));
            }
        }
        self.mates_of = id;
        let changed = mates != self.group_mates;
        self.group_mates = mates;
        changed
    }

    /// What we announce to every peer.
    pub(crate) fn announcement(&self) -> Option<GroupAnnounce> {
        let group = self.group.as_ref()?;
        let public = group.state.public.then(|| PublicGroup {
            key: group.key.clone(),
            label: group.state.label.clone(),
        });
        Some(GroupAnnounce {
            proof: group.key.proof(self.own_uuid?),
            public,
        })
    }

    pub(crate) fn is_group_mate(&self, peer: Uuid) -> bool {
        self.group.is_some() && self.group_mates.contains(&peer)
    }

    /// Changes our group's shared state, as the newest version.
    pub(crate) fn change_group_state(
        &mut self,
        change: impl FnOnce(&mut GroupState),
    ) -> Result<(), GroupError> {
        let me = self.own_uuid.unwrap_or_default();
        let group = self.group.as_mut().ok_or(GroupError::NotInGroup)?;
        let mut state = group.state.clone();
        change(&mut state);
        if !state.is_valid() {
            return Err(GroupError::LabelTooLong);
        }
        if (state.public, &state.label) != (group.state.public, &group.state.label) {
            state.version = group.state.version + 1;
            state.by = me;
            group.state = state;
        }
        Ok(())
    }

    /// A group mate's state for our group: adopted if newer, with a notice
    /// for each visible change. Returns whether it was adopted, so it can be
    /// passed on.
    pub(crate) fn receive_group_state(
        &mut self,
        from: Uuid,
        state: GroupState,
        events: &mut Vec<Event>,
    ) -> bool {
        if !self.is_group_mate(from) || !state.is_valid() {
            return false;
        }
        let Some(group) = &mut self.group else {
            return false;
        };
        if !state.newer_than(&group.state) {
            return false;
        }
        if state.public != group.state.public {
            let kind = if state.public {
                GroupNoticeKind::MadePublic
            } else {
                GroupNoticeKind::MadePrivate
            };
            events.push(notice(kind, Uuid::nil(), state.by));
        }
        if state.label != group.state.label {
            events.push(notice(GroupNoticeKind::LabelChanged, Uuid::nil(), state.by));
        }
        group.state = state;
        true
    }

    /// Whether `peer` may invite us, by the `invites` setting. Friends are
    /// the `friends_only` list, so with no list nobody is a friend.
    fn may_invite_us(&self, peer: Uuid) -> bool {
        match self.invites_from {
            InvitesFrom::Everyone => self.is_friend(peer),
            InvitesFrom::Friends => self
                .friends_only
                .as_ref()
                .is_some_and(|friends| friends.contains(&peer)),
            InvitesFrom::Nobody => false,
        }
    }

    /// Remembers an invite from `from`, if it may invite us.
    pub(crate) fn receive_invite(
        &mut self,
        from: Uuid,
        key: GroupKey,
        state: GroupState,
        now: Instant,
        events: &mut Vec<Event>,
    ) {
        let id = key.id();
        let already_in = self.group.as_ref().is_some_and(|g| g.id() == id);
        if already_in || !state.is_valid() || self.muted(from) || !self.may_invite_us(from) {
            return;
        }
        self.invites.insert(
            from,
            Invite {
                key,
                state,
                received: now,
            },
        );
        events.push(Event::Invite { from });
    }

    /// Joins the group `from` last invited us to.
    pub(crate) fn accept_invite(&mut self, from: Uuid, now: Instant) -> Result<(), GroupError> {
        let invite = self
            .invites
            .remove(&from)
            .filter(|invite| now.duration_since(invite.received) < INVITE_LIFETIME)
            .ok_or(GroupError::NoInvite)?;
        self.set_group(Some(OwnGroup::new(invite.key, invite.state)));
        Ok(())
    }

    /// Joins the public group `id`, whose key `announced` holds.
    pub(crate) fn join_public(
        &mut self,
        id: GroupId,
        announced: &[(Uuid, GroupAnnounce)],
    ) -> Result<(), GroupError> {
        if self.group.as_ref().is_some_and(|g| g.id() == id) {
            return Ok(());
        }
        let (announcer, public) = announced
            .iter()
            .filter_map(|(uuid, announce)| Some((*uuid, announce.public.as_ref()?)))
            .find(|(_, public)| public.key.id() == id && label_fits(&public.label))
            .ok_or(GroupError::UnknownGroup)?;
        // A stand-in until the members send theirs: every public group has
        // passed version 0, so the members' state wins.
        let state = GroupState {
            version: 0,
            public: true,
            label: public.label.clone(),
            by: announcer,
        };
        self.set_group(Some(OwnGroup::new(public.key.clone(), state)));
        Ok(())
    }

    /// Our group plus the public groups `announced`, merged by id, in a
    /// stable order: ours first, then by label and id.
    pub(crate) fn groups(&self, announced: &[(Uuid, GroupAnnounce)]) -> Vec<GroupInfo> {
        let mut groups = Vec::new();
        if let (Some(own), Some(me)) = (&self.group, self.own_uuid) {
            let mut members: Vec<Uuid> = self.group_mates.iter().copied().chain([me]).collect();
            members.sort();
            groups.push(GroupInfo {
                id: own.id(),
                label: own.state.label.clone(),
                public: own.state.public,
                mine: true,
                members,
            });
        }
        let mut public: Vec<GroupInfo> = Vec::new();
        for (uuid, announce) in announced {
            let Some(group) = &announce.public else {
                continue;
            };
            let id = group.key.id();
            // Our own group is listed above, from our mates.
            if groups.first().is_some_and(|own| own.id == id)
                || !group.key.has_member(*uuid, announce)
                || !label_fits(&group.label)
            {
                continue;
            }
            match public.iter_mut().find(|g| g.id == id) {
                Some(known) => known.members.push(*uuid),
                None => public.push(GroupInfo {
                    id,
                    label: group.label.clone(),
                    public: true,
                    mine: false,
                    members: vec![*uuid],
                }),
            }
        }
        for group in &mut public {
            group.members.sort();
        }
        public.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        groups.extend(public);
        groups
    }

    /// The messages `peer` hasn't been sent yet: whether we want its audio,
    /// our group, for a group mate our group's state, and our latest invite
    /// to it unless it is older than `now` allows.
    pub(crate) fn news_for(&self, peer: Uuid, told: &mut Told, now: Instant) -> Vec<VoiceMsg> {
        let mut news = Vec::new();
        let wants_audio = self.wants_audio_from(peer);
        if wants_audio != told.wants_audio {
            news.push(VoiceMsg::ReceiveState { wants_audio });
            told.wants_audio = wants_audio;
        }
        let group = self.announcement();
        if group != told.group {
            news.push(VoiceMsg::Group(group.clone()));
            told.group = group;
        }
        match &self.group {
            Some(own) if self.is_group_mate(peer) => {
                if told.state.as_ref() != Some(&own.state) {
                    news.push(VoiceMsg::GroupState(own.state.clone()));
                    told.state = Some(own.state.clone());
                }
            }
            _ => told.state = None,
        }
        if let Some(invite) = self.invites_sent.get(&peer)
            && told.invite != Some(invite.sent)
            && now.duration_since(invite.sent) < INVITE_LIFETIME
        {
            news.push(VoiceMsg::Invite {
                key: invite.key.clone(),
                state: invite.state.clone(),
            });
            told.invite = Some(invite.sent);
        }
        news
    }
}

fn notice(kind: GroupNoticeKind, uuid: Uuid, by: Uuid) -> Event {
    Event::GroupNotice { kind, uuid, by }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AudioConfig, InvitesFrom};
    use crate::engine::PeerAudio;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    /// Client `n`, outside any group.
    fn client(n: u8) -> VoiceState {
        let mut state = VoiceState::new(AudioConfig::default(), 48.0, None);
        state.own_uuid = Some(uuid(n));
        state
    }

    /// Puts `state` in a new group of its own.
    fn start_group(state: &mut VoiceState) {
        let me = state.own_uuid.unwrap();
        state.set_group(Some(OwnGroup::new(GroupKey::random(), GroupState::new(me))));
    }

    fn announce(state: &VoiceState) -> (Uuid, GroupAnnounce) {
        (state.own_uuid.unwrap(), state.announcement().unwrap())
    }

    /// Clients 1..=n in one group, each seeing every other as a mate.
    fn group_of(n: u8) -> Vec<VoiceState> {
        let mut first = client(1);
        start_group(&mut first);
        let own = first.group.clone().unwrap();
        let mut clients = vec![first];
        for i in 2..=n {
            let mut state = client(i);
            state.set_group(Some(OwnGroup::new(own.key.clone(), own.state.clone())));
            clients.push(state);
        }
        let announced: Vec<_> = clients.iter().map(announce).collect();
        for state in &mut clients {
            let mates = state.mates_among(&announced);
            state.update_mates(mates, &mut Vec::new());
        }
        clients
    }

    #[test]
    fn mates_need_a_proof_for_our_key() {
        let clients = group_of(2);
        let mut stranger = client(3);
        start_group(&mut stranger);
        let announced = [announce(&clients[1]), announce(&stranger)];
        assert_eq!(clients[0].mates_among(&announced), HashSet::from([uuid(2)]));
        // A copied proof doesn't make the copier a mate.
        let copied = [(uuid(3), clients[1].announcement().unwrap())];
        assert!(clients[0].mates_among(&copied).is_empty());
    }

    #[test]
    fn mates_joining_and_leaving_are_reported_but_not_on_our_own_join() {
        let mut clients = group_of(2);
        let mut events = Vec::new();
        // Joining reported nothing (see `group_of`); a newcomer is reported.
        let mut carol = client(3);
        carol.set_group(clients[0].group.clone());
        let announced = [announce(&clients[1]), announce(&carol)];
        let mates = clients[0].mates_among(&announced);
        assert!(clients[0].update_mates(mates, &mut events));
        assert_eq!(
            events,
            [notice(GroupNoticeKind::Joined, uuid(3), Uuid::nil())]
        );
        events.clear();
        clients[0].update_mates(HashSet::from([uuid(3)]), &mut events);
        assert_eq!(
            events,
            [notice(GroupNoticeKind::Left, uuid(2), Uuid::nil())]
        );
    }

    #[test]
    fn private_groups_announce_only_a_proof() {
        let mut clients = group_of(1);
        let me = &mut clients[0];
        assert_eq!(me.announcement().unwrap().public, None);
        me.change_group_state(|s| {
            s.public = true;
            s.label = "Miners".into();
        })
        .unwrap();
        let public = me.announcement().unwrap().public.unwrap();
        assert_eq!(public.key, me.group.as_ref().unwrap().key);
        assert_eq!(public.label, "Miners");
        assert_eq!(me.group.as_ref().unwrap().state.version, 1);
        assert_eq!(
            me.change_group_state(|s| s.label = "x".repeat(33)),
            Err(GroupError::LabelTooLong)
        );
        // No change, no new version.
        me.change_group_state(|_| {}).unwrap();
        assert_eq!(me.group.as_ref().unwrap().state.version, 1);
    }

    #[test]
    fn group_state_merges_by_version_and_reports_changes() {
        let mut clients = group_of(3);
        clients[1].change_group_state(|s| s.public = true).unwrap();
        let newer = clients[1].group.as_ref().unwrap().state.clone();
        let mut events = Vec::new();
        assert!(clients[0].receive_group_state(uuid(2), newer.clone(), &mut events));
        assert_eq!(
            events,
            [notice(GroupNoticeKind::MadePublic, Uuid::nil(), uuid(2))]
        );
        // Passed on again, it is no longer news.
        assert!(!clients[0].receive_group_state(uuid(3), newer.clone(), &mut events));
        // Carol changed the label at the same version: the higher UUID wins.
        let tie = GroupState {
            public: false,
            label: "Carol's".into(),
            by: uuid(3),
            ..newer
        };
        events.clear();
        assert!(clients[0].receive_group_state(uuid(3), tie, &mut events));
        assert_eq!(
            events,
            [
                notice(GroupNoticeKind::MadePrivate, Uuid::nil(), uuid(3)),
                notice(GroupNoticeKind::LabelChanged, Uuid::nil(), uuid(3)),
            ]
        );
        // Not from a stranger.
        let forged = GroupState {
            version: 99,
            ..GroupState::new(uuid(9))
        };
        assert!(!clients[0].receive_group_state(uuid(9), forged, &mut events));
    }

    #[test]
    fn invites_follow_the_setting_and_expire() {
        let mut alice = client(1);
        start_group(&mut alice);
        let own = alice.group.clone().unwrap();
        let now = Instant::now();
        let mut bob = client(2);
        let mut events = Vec::new();
        bob.receive_invite(
            uuid(1),
            own.key.clone(),
            own.state.clone(),
            now,
            &mut events,
        );
        assert_eq!(events, [Event::Invite { from: uuid(1) }]);
        let late = now + INVITE_LIFETIME;
        assert_eq!(bob.accept_invite(uuid(1), late), Err(GroupError::NoInvite));

        bob.receive_invite(
            uuid(1),
            own.key.clone(),
            own.state.clone(),
            now,
            &mut events,
        );
        bob.accept_invite(uuid(1), now).unwrap();
        assert_eq!(bob.group.as_ref().unwrap().id(), own.id());

        for setting in [InvitesFrom::Nobody, InvitesFrom::Friends] {
            let mut carol = client(3);
            carol.invites_from = setting;
            events.clear();
            carol.receive_invite(
                uuid(1),
                own.key.clone(),
                own.state.clone(),
                now,
                &mut events,
            );
            assert!(events.is_empty(), "{setting:?}");
        }
        let mut carol = client(3);
        carol.invites_from = InvitesFrom::Friends;
        carol.friends_only = Some(vec![uuid(1)]);
        carol.receive_invite(
            uuid(1),
            own.key.clone(),
            own.state.clone(),
            now,
            &mut events,
        );
        assert_eq!(events, [Event::Invite { from: uuid(1) }]);
        // A muted player's invites are dropped too.
        let mut dave = client(4);
        dave.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 1.0,
                muted: true,
            },
        );
        events.clear();
        dave.receive_invite(uuid(1), own.key, own.state, now, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn public_groups_are_listed_and_joinable_private_ones_are_not() {
        let mut clients = group_of(2);
        let mut events = Vec::new();
        let mut carol = client(3);
        let announced = [announce(&clients[0]), announce(&clients[1])];
        assert!(carol.groups(&announced).is_empty());
        let id = clients[0].group.as_ref().unwrap().id();
        assert_eq!(
            carol.join_public(id, &announced),
            Err(GroupError::UnknownGroup)
        );

        for state in &mut clients {
            state.change_group_state(|s| s.public = true).unwrap();
        }
        let announced = [announce(&clients[0]), announce(&clients[1])];
        let listed = carol.groups(&announced);
        assert_eq!(
            listed,
            [GroupInfo {
                id,
                label: String::new(),
                public: true,
                mine: false,
                members: vec![uuid(1), uuid(2)],
            }]
        );
        carol.join_public(id, &announced).unwrap();
        let mates = carol.mates_among(&announced);
        carol.update_mates(mates, &mut events);
        let mine = &carol.groups(&announced)[0];
        assert!(mine.mine);
        assert_eq!(mine.members, [uuid(1), uuid(2), uuid(3)]);
    }

    #[test]
    fn news_sends_each_change_once() {
        let mut clients = group_of(2);
        let now = Instant::now();
        let mut told = Told::default();
        let news = clients[0].news_for(uuid(2), &mut told, now);
        assert!(matches!(
            news[..],
            [VoiceMsg::Group(Some(_)), VoiceMsg::GroupState(_)]
        ));
        assert!(clients[0].news_for(uuid(2), &mut told, now).is_empty());

        clients[0].change_group_state(|s| s.public = true).unwrap();
        let news = clients[0].news_for(uuid(2), &mut told, now);
        assert!(matches!(
            news[..],
            [VoiceMsg::Group(Some(_)), VoiceMsg::GroupState(_)]
        ));

        // A player outside the group is told we don't want its audio.
        let mut stranger = Told::default();
        let news = clients[0].news_for(uuid(5), &mut stranger, now);
        assert!(matches!(
            news[..],
            [
                VoiceMsg::ReceiveState { wants_audio: false },
                VoiceMsg::Group(Some(_))
            ]
        ));
        // An invite goes once, only to its target, and not once expired.
        clients[0].invite(uuid(5), now);
        let news = clients[0].news_for(uuid(5), &mut stranger, now);
        assert!(matches!(news[..], [VoiceMsg::Invite { .. }]));
        assert!(clients[0].news_for(uuid(5), &mut stranger, now).is_empty());
        let late = now + INVITE_LIFETIME;
        assert_eq!(
            clients[0]
                .news_for(uuid(5), &mut Told::default(), late)
                .len(),
            2
        );
    }

    #[test]
    fn an_inviter_outside_a_group_joins_once_the_invitee_does() {
        let now = Instant::now();
        let mut alice = client(1);
        alice.invite(uuid(2), now);
        alice.invite(uuid(3), now + Duration::from_secs(1));
        assert!(alice.group.is_none(), "still in proximity voice");
        // Both invites carry the same pending key.
        let to_bob = alice.invites_sent[&uuid(2)].key.clone();
        assert_eq!(alice.invites_sent[&uuid(3)].key, to_bob);

        let mut bob = client(2);
        let mut events = Vec::new();
        let state = alice.invites_sent[&uuid(2)].state.clone();
        bob.receive_invite(uuid(1), to_bob.clone(), state, now, &mut events);
        bob.accept_invite(uuid(1), now).unwrap();

        // Bob's announcement proves the key: Alice joins and sees him join.
        let announced = [announce(&bob)];
        alice.join_pending(&announced);
        assert_eq!(alice.group.as_ref().unwrap().key, to_bob);
        let mates = alice.mates_among(&announced);
        events.clear();
        alice.update_mates(mates, &mut events);
        assert_eq!(
            events,
            [notice(GroupNoticeKind::Joined, uuid(2), Uuid::nil())]
        );

        // After two minutes a new invite gets a new key.
        let mut carol = client(3);
        carol.invite(uuid(4), now);
        let first = carol.pending.clone().unwrap().key;
        carol.invite(uuid(4), now + INVITE_LIFETIME);
        assert_ne!(carol.pending.unwrap().key, first);
    }
}

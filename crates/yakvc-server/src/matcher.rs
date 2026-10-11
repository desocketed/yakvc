//! Matches sessions whose pair tokens show they see each other.
//!
//! Two sessions match when both hold the token for exactly their own pair of
//! UUIDs. Holding someone else's token does nothing, so a client cannot match
//! with players it doesn't see by sending made-up tokens.
//!
//! The matcher only keeps state and says who to tell what; the caller delivers
//! the returned [`Notice`]s.

use std::collections::{HashMap, HashSet};

use yakvc_proto::rdv::ServerMsg;
use yakvc_proto::{EndpointAddr, EndpointId, PairToken, SignedTicket, Uuid};

/// A message for one session.
#[derive(Debug, Clone)]
pub(crate) struct Notice {
    pub to: EndpointId,
    pub msg: ServerMsg,
}

#[derive(Debug, Default)]
pub(crate) struct Matcher {
    sessions: HashMap<EndpointId, Session>,
    /// Which sessions hold each token, by UUID. Only one other UUID can
    /// match a token, so finding it costs one hash per UUID, however many
    /// sessions share a UUID.
    holders: HashMap<PairToken, HashMap<Uuid, Vec<EndpointId>>>,
    matches: usize,
    /// Hashes computed so far; see [`hashes`](Self::hashes).
    hashes: u64,
}

#[derive(Debug)]
struct Session {
    uuid: Uuid,
    ticket: SignedTicket,
    addr: EndpointAddr,
    tokens: HashSet<PairToken>,
    /// Current matches, by the token that made each one, so dropping a
    /// token ends its match without hashing.
    matched: HashMap<PairToken, Vec<EndpointId>>,
}

impl Matcher {
    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Pairs currently matched.
    pub fn matches(&self) -> usize {
        self.matches
    }

    /// Hashes computed since the matcher was created. Adding a token costs
    /// one hash per other UUID already holding it: one for a real pair,
    /// but as many as an attacker likes when they hold one token under
    /// many UUIDs, so the caller charges each session for the work its
    /// updates cause.
    pub fn hashes(&self) -> u64 {
        self.hashes
    }

    #[cfg(test)]
    pub fn pair_count(&self, id: EndpointId) -> usize {
        self.sessions.get(&id).map_or(0, |s| s.tokens.len())
    }

    /// How many tokens the session would hold after `add_pairs(tokens)`:
    /// duplicates and tokens it already holds count once.
    pub fn pair_count_after_adding(&self, id: EndpointId, tokens: &[PairToken]) -> usize {
        let Some(session) = self.sessions.get(&id) else {
            return 0;
        };
        let new: HashSet<&PairToken> = tokens
            .iter()
            .filter(|token| !session.tokens.contains(token))
            .collect();
        session.tokens.len() + new.len()
    }

    /// Adds a session with no tokens, replacing any earlier one for `id`.
    pub fn register(
        &mut self,
        id: EndpointId,
        uuid: Uuid,
        ticket: SignedTicket,
        addr: EndpointAddr,
    ) -> Vec<Notice> {
        let notices = self.unregister(id);
        let session = Session {
            uuid,
            ticket,
            addr,
            tokens: HashSet::new(),
            matched: HashMap::new(),
        };
        self.sessions.insert(id, session);
        notices
    }

    /// Removes a session; its matches get `PeerGone`.
    pub fn unregister(&mut self, id: EndpointId) -> Vec<Notice> {
        let mut notices = Vec::new();
        let Some(tokens) = self.sessions.get(&id).map(|s| s.tokens.clone()) else {
            return notices;
        };
        for token in tokens {
            self.remove_token(id, token, &mut notices);
        }
        self.sessions.remove(&id);
        notices
    }

    /// Replaces the session's token set.
    pub fn set_pairs(&mut self, id: EndpointId, tokens: Vec<PairToken>) -> Vec<Notice> {
        let Some(session) = self.sessions.get(&id) else {
            return Vec::new();
        };
        let new: HashSet<PairToken> = tokens.into_iter().collect();
        let removed: Vec<PairToken> = session.tokens.difference(&new).copied().collect();
        let added: Vec<PairToken> = new.difference(&session.tokens).copied().collect();
        let mut notices = self.remove_pairs(id, removed);
        notices.extend(self.add_pairs(id, added));
        notices
    }

    pub fn add_pairs(&mut self, id: EndpointId, tokens: Vec<PairToken>) -> Vec<Notice> {
        let mut notices = Vec::new();
        if !self.sessions.contains_key(&id) {
            return notices;
        }
        for token in tokens {
            self.add_token(id, token, &mut notices);
        }
        notices
    }

    pub fn remove_pairs(&mut self, id: EndpointId, tokens: Vec<PairToken>) -> Vec<Notice> {
        let mut notices = Vec::new();
        for token in tokens {
            self.remove_token(id, token, &mut notices);
        }
        notices
    }

    /// Records a new address and re-announces the session to its matches.
    pub fn update_addr(&mut self, id: EndpointId, addr: EndpointAddr) -> Vec<Notice> {
        let Some(session) = self.sessions.get_mut(&id) else {
            return Vec::new();
        };
        session.addr = addr;
        let session = &self.sessions[&id];
        session
            .matched
            .values()
            .flatten()
            .map(|&peer| Notice {
                to: peer,
                msg: peer_available(session),
            })
            .collect()
    }

    /// Records a renewed ticket. Current matches get it from the client
    /// directly (`TicketUpdate`); later matches get it from us.
    pub fn update_ticket(&mut self, id: EndpointId, ticket: SignedTicket) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.ticket = ticket;
        }
    }

    fn add_token(&mut self, id: EndpointId, token: PairToken, notices: &mut Vec<Notice>) {
        let session = self.sessions.get_mut(&id).expect("caller checked");
        if !session.tokens.insert(token) {
            return;
        }
        let uuid = session.uuid;
        let holders = self.holders.entry(token).or_default();
        let others = holders.len() - usize::from(holders.contains_key(&uuid));
        let matching: Vec<EndpointId> = holders
            .iter()
            .filter(|&(&other, _)| other != uuid && PairToken::new(uuid, other) == token)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        holders.entry(uuid).or_default().push(id);
        self.hashes += others as u64;

        for other in matching {
            self.link(id, other, token, notices);
        }
    }

    fn remove_token(&mut self, id: EndpointId, token: PairToken, notices: &mut Vec<Notice>) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !session.tokens.remove(&token) {
            return;
        }
        let uuid = session.uuid;
        let matched = session.matched.remove(&token).unwrap_or_default();

        let holders = self.holders.get_mut(&token).expect("token was held");
        let ids = holders.get_mut(&uuid).expect("token was held");
        ids.retain(|&h| h != id);
        if ids.is_empty() {
            holders.remove(&uuid);
        }
        if holders.is_empty() {
            self.holders.remove(&token);
        }

        for peer in matched {
            self.unlink(id, peer, token, notices);
        }
    }

    /// Starts the match `token` makes between `a` and `b`. A pair has one
    /// token and each side adds it once, so no match is made twice.
    fn link(&mut self, a: EndpointId, b: EndpointId, token: PairToken, notices: &mut Vec<Notice>) {
        for (from, to) in [(a, b), (b, a)] {
            let matched = &mut self.sessions.get_mut(&from).unwrap().matched;
            matched.entry(token).or_default().push(to);
        }
        self.matches += 1;
        notices.push(Notice {
            to: a,
            msg: peer_available(&self.sessions[&b]),
        });
        notices.push(Notice {
            to: b,
            msg: peer_available(&self.sessions[&a]),
        });
    }

    /// Ends the match `token` made between `a` and `b`. The caller has
    /// already forgotten it on `a`'s side.
    fn unlink(
        &mut self,
        a: EndpointId,
        b: EndpointId,
        token: PairToken,
        notices: &mut Vec<Notice>,
    ) {
        let matched = &mut self.sessions.get_mut(&b).unwrap().matched;
        let peers = matched.get_mut(&token).expect("matches are symmetric");
        peers.retain(|&peer| peer != a);
        if peers.is_empty() {
            matched.remove(&token);
        }
        self.matches -= 1;
        notices.push(Notice {
            to: a,
            msg: ServerMsg::PeerGone(b),
        });
        notices.push(Notice {
            to: b,
            msg: ServerMsg::PeerGone(a),
        });
    }
}

fn peer_available(session: &Session) -> ServerMsg {
    ServerMsg::PeerAvailable {
        ticket: session.ticket.clone(),
        addr: session.addr.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use yakvc_proto::{IssuerKey, SecretKey, TicketBody};

    use super::*;

    struct Player {
        id: EndpointId,
        uuid: Uuid,
    }

    fn player(n: u8) -> Player {
        Player {
            id: SecretKey::from_bytes(&[n; 32]).public(),
            uuid: Uuid::from_u128(n.into()),
        }
    }

    fn join(m: &mut Matcher, p: &Player) -> Vec<Notice> {
        let body = TicketBody::new(
            p.uuid,
            "player".into(),
            p.id,
            true,
            SystemTime::now(),
            Duration::from_secs(60),
        );
        let ticket = IssuerKey::from_bytes(&[0; 32]).sign(&body);
        m.register(p.id, p.uuid, ticket, EndpointAddr::new(p.id))
    }

    /// The tokens a client sends for its tab list.
    fn sees(me: &Player, others: &[&Player]) -> Vec<PairToken> {
        others
            .iter()
            .map(|o| PairToken::new(me.uuid, o.uuid))
            .collect()
    }

    /// Notices as `(to, "available" | "gone", peer)`, sorted for comparison.
    fn summary(notices: &[Notice]) -> Vec<(EndpointId, &'static str, EndpointId)> {
        let mut out: Vec<_> = notices
            .iter()
            .map(|n| match &n.msg {
                ServerMsg::PeerAvailable { addr, .. } => (n.to, "available", addr.id),
                ServerMsg::PeerGone(id) => (n.to, "gone", *id),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        out.sort();
        out
    }

    fn pair(
        kind: &'static str,
        a: &Player,
        b: &Player,
    ) -> Vec<(EndpointId, &'static str, EndpointId)> {
        let mut v = vec![(a.id, kind, b.id), (b.id, kind, a.id)];
        v.sort();
        v
    }

    #[test]
    fn mutual_tokens_match_once() {
        let (a, b) = (player(1), player(2));
        let mut m = Matcher::default();
        join(&mut m, &a);
        join(&mut m, &b);

        assert!(m.set_pairs(a.id, sees(&a, &[&b])).is_empty());
        let notices = m.set_pairs(b.id, sees(&b, &[&a]));
        assert_eq!(summary(&notices), pair("available", &a, &b));
        assert_eq!(m.matches(), 1);

        // Sending the same token again changes nothing.
        assert!(m.add_pairs(b.id, sees(&b, &[&a])).is_empty());
        assert_eq!(m.matches(), 1);
    }

    #[test]
    fn one_sided_visibility_does_not_match() {
        let (a, b, c) = (player(1), player(2), player(3));
        let mut m = Matcher::default();
        for p in [&a, &b, &c] {
            join(&mut m, p);
        }
        // a sees b and c, b sees a, c sees nobody.
        m.set_pairs(a.id, sees(&a, &[&b, &c]));
        let notices = m.set_pairs(b.id, sees(&b, &[&a]));
        assert_eq!(summary(&notices), pair("available", &a, &b));
        assert!(m.set_pairs(c.id, vec![]).is_empty());
        assert_eq!(m.matches(), 1);
    }

    #[test]
    fn someone_elses_token_does_not_match() {
        let (a, b, eve) = (player(1), player(2), player(3));
        let mut m = Matcher::default();
        for p in [&a, &b, &eve] {
            join(&mut m, p);
        }
        // Eve claims the a/b token; only a and b themselves match on it.
        let ab = PairToken::new(a.uuid, b.uuid);
        m.add_pairs(eve.id, vec![ab]);
        m.add_pairs(a.id, vec![ab]);
        let notices = m.add_pairs(b.id, vec![ab]);
        assert_eq!(summary(&notices), pair("available", &a, &b));
        assert_eq!(m.matches(), 1);
    }

    #[test]
    fn same_uuid_never_matches_itself() {
        let a = player(1);
        let twin = Player {
            id: SecretKey::from_bytes(&[9; 32]).public(),
            uuid: a.uuid,
        };
        let mut m = Matcher::default();
        join(&mut m, &a);
        join(&mut m, &twin);
        let token = PairToken::new(a.uuid, a.uuid);
        m.add_pairs(a.id, vec![token]);
        assert!(m.add_pairs(twin.id, vec![token]).is_empty());
    }

    #[test]
    fn sessions_sharing_a_uuid_each_match_and_are_cleaned_up() {
        let (a, b) = (player(1), player(2));
        let a2 = Player {
            id: SecretKey::from_bytes(&[9; 32]).public(),
            uuid: a.uuid,
        };
        let mut m = Matcher::default();
        for p in [&a, &a2, &b] {
            join(&mut m, p);
        }
        m.add_pairs(a.id, sees(&a, &[&b]));
        m.add_pairs(a2.id, sees(&a2, &[&b]));
        let notices = m.add_pairs(b.id, sees(&b, &[&a]));
        let mut expected = pair("available", &a, &b);
        expected.extend(pair("available", &a2, &b));
        expected.sort();
        assert_eq!(summary(&notices), expected);
        assert_eq!(m.holders.len(), 1);
        assert_eq!(m.holders.values().next().unwrap().len(), 2, "two UUIDs");

        m.unregister(a.id);
        assert_eq!(m.matches(), 1);
        m.unregister(a2.id);
        m.unregister(b.id);
        assert!(m.holders.is_empty());
    }

    #[test]
    fn removing_a_token_ends_the_match() {
        let (a, b) = (player(1), player(2));
        let mut m = Matcher::default();
        join(&mut m, &a);
        join(&mut m, &b);
        m.add_pairs(a.id, sees(&a, &[&b]));
        m.add_pairs(b.id, sees(&b, &[&a]));

        let notices = m.remove_pairs(a.id, sees(&a, &[&b]));
        assert_eq!(summary(&notices), pair("gone", &a, &b));
        assert_eq!(m.matches(), 0);

        // Seeing each other again re-matches.
        let notices = m.add_pairs(a.id, sees(&a, &[&b]));
        assert_eq!(summary(&notices), pair("available", &a, &b));
    }

    #[test]
    fn set_pairs_applies_only_the_difference() {
        let (a, b, c) = (player(1), player(2), player(3));
        let mut m = Matcher::default();
        for p in [&a, &b, &c] {
            join(&mut m, p);
        }
        m.set_pairs(b.id, sees(&b, &[&a]));
        m.set_pairs(c.id, sees(&c, &[&a]));
        m.set_pairs(a.id, sees(&a, &[&b]));

        // a now sees c instead of b: one match ends, one starts, and nothing
        // is said about pairs that didn't change.
        let notices = m.set_pairs(a.id, sees(&a, &[&c]));
        let mut expected = pair("gone", &a, &b);
        expected.extend(pair("available", &a, &c));
        expected.sort();
        assert_eq!(summary(&notices), expected);
        assert_eq!(m.pair_count(a.id), 1);
        assert_eq!(m.matches(), 1);
    }

    #[test]
    fn unregister_tells_matches_and_cleans_up() {
        let (a, b, c) = (player(1), player(2), player(3));
        let mut m = Matcher::default();
        for p in [&a, &b, &c] {
            join(&mut m, p);
        }
        m.set_pairs(a.id, sees(&a, &[&b, &c]));
        m.set_pairs(b.id, sees(&b, &[&a]));
        m.set_pairs(c.id, sees(&c, &[&a]));
        assert_eq!(m.matches(), 2);

        let notices = m.unregister(a.id);
        let mut expected = pair("gone", &a, &b);
        expected.extend(pair("gone", &a, &c));
        expected.sort();
        assert_eq!(summary(&notices), expected);
        assert_eq!(m.matches(), 0);
        assert_eq!(m.sessions(), 2);
        assert_eq!(m.pair_count(a.id), 0);
        // Only b's and c's own tokens are still indexed.
        assert_eq!(m.holders.len(), 2);
    }

    #[test]
    fn re_register_replaces_the_session() {
        let (a, b) = (player(1), player(2));
        let mut m = Matcher::default();
        join(&mut m, &a);
        join(&mut m, &b);
        m.set_pairs(a.id, sees(&a, &[&b]));
        m.set_pairs(b.id, sees(&b, &[&a]));

        let notices = join(&mut m, &a);
        assert_eq!(summary(&notices), pair("gone", &a, &b));
        assert_eq!(m.sessions(), 2);
        assert_eq!(m.pair_count(a.id), 0);
    }

    #[test]
    fn address_change_is_re_announced_to_matches() {
        let (a, b, c) = (player(1), player(2), player(3));
        let mut m = Matcher::default();
        for p in [&a, &b, &c] {
            join(&mut m, p);
        }
        m.set_pairs(a.id, sees(&a, &[&b]));
        m.set_pairs(b.id, sees(&b, &[&a]));

        let new_addr = EndpointAddr::new(a.id).with_ip_addr("127.0.0.1:9".parse().unwrap());
        let notices = m.update_addr(a.id, new_addr.clone());
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].to, b.id);
        assert!(
            matches!(&notices[0].msg, ServerMsg::PeerAvailable { addr, .. } if *addr == new_addr)
        );
    }

    #[test]
    fn hashes_are_counted_per_other_holder_and_never_on_removal() {
        let (a, b) = (player(1), player(2));
        let puppets: Vec<Player> = (10..20).map(player).collect();
        let mut m = Matcher::default();
        for p in [&a, &b].into_iter().chain(&puppets) {
            join(&mut m, p);
        }
        let ab = PairToken::new(a.uuid, b.uuid);
        // The first holder of a token finds nobody to check against.
        m.add_pairs(puppets[0].id, vec![ab]);
        assert_eq!(m.hashes(), 0);
        for (n, p) in puppets.iter().enumerate().skip(1) {
            m.add_pairs(p.id, vec![ab]);
            assert_eq!(m.hashes(), (n * (n + 1) / 2) as u64);
        }
        let so_far = m.hashes();
        m.add_pairs(a.id, vec![ab]);
        assert_eq!(m.hashes(), so_far + 10, "one per puppet UUID");
        m.add_pairs(b.id, vec![ab]);
        assert_eq!(m.hashes(), so_far + 21);
        assert_eq!(m.matches(), 1);

        let hashed = m.hashes();
        let notices = m.set_pairs(a.id, vec![]);
        assert_eq!(summary(&notices), pair("gone", &a, &b));
        for p in &puppets {
            m.unregister(p.id);
        }
        assert_eq!(m.hashes(), hashed, "dropping tokens costs no hashes");
        assert_eq!(m.matches(), 0);
    }

    #[test]
    fn unknown_sessions_are_ignored() {
        let a = player(1);
        let mut m = Matcher::default();
        assert!(m.add_pairs(a.id, sees(&a, &[&player(2)])).is_empty());
        assert!(m.unregister(a.id).is_empty());
        assert!(m.update_addr(a.id, EndpointAddr::new(a.id)).is_empty());
        assert!(m.holders.is_empty());
    }
}

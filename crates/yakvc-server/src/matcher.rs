//! Matches sessions whose pair tokens show they see each other.
//!
//! Two sessions match when both hold the token for exactly their own pair of
//! UUIDs. Holding someone else's token does nothing, so a client cannot match
//! with players it doesn't see by sending made-up tokens.
//!
//! The matcher only keeps state and says who to tell what; the caller delivers
//! the returned [`Notice`]s.

use std::collections::{HashMap, HashSet};

use yakvc_shared::rdv::ServerMsg;
use yakvc_shared::{EndpointAddr, EndpointId, PairToken, SignedTicket, Uuid};

/// A message for one session.
#[derive(Debug, Clone)]
pub(crate) struct Notice {
    pub to: EndpointId,
    pub msg: ServerMsg,
}

#[derive(Debug, Default)]
pub(crate) struct Matcher {
    sessions: HashMap<EndpointId, Session>,
    /// Which sessions hold each token.
    holders: HashMap<PairToken, Vec<EndpointId>>,
    matches: usize,
}

#[derive(Debug)]
struct Session {
    uuid: Uuid,
    ticket: SignedTicket,
    addr: EndpointAddr,
    tokens: HashSet<PairToken>,
    matched: HashSet<EndpointId>,
}

impl Matcher {
    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Pairs currently matched.
    pub fn matches(&self) -> usize {
        self.matches
    }

    pub fn pair_count(&self, id: EndpointId) -> usize {
        self.sessions.get(&id).map_or(0, |s| s.tokens.len())
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
            matched: HashSet::new(),
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
            .iter()
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
        let others = holders.clone();
        holders.push(id);

        for other in others {
            let other_uuid = self.sessions[&other].uuid;
            if other_uuid != uuid && PairToken::new(uuid, other_uuid) == token {
                self.link(id, other, notices);
            }
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
        let matched: Vec<EndpointId> = session.matched.iter().copied().collect();

        let holders = self.holders.get_mut(&token).expect("token was held");
        holders.retain(|&h| h != id);
        if holders.is_empty() {
            self.holders.remove(&token);
        }

        for peer in matched {
            if PairToken::new(uuid, self.sessions[&peer].uuid) == token {
                self.unlink(id, peer, notices);
            }
        }
    }

    fn link(&mut self, a: EndpointId, b: EndpointId, notices: &mut Vec<Notice>) {
        if !self.sessions.get_mut(&a).unwrap().matched.insert(b) {
            return;
        }
        self.sessions.get_mut(&b).unwrap().matched.insert(a);
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

    fn unlink(&mut self, a: EndpointId, b: EndpointId, notices: &mut Vec<Notice>) {
        if !self.sessions.get_mut(&a).unwrap().matched.remove(&b) {
            return;
        }
        self.sessions.get_mut(&b).unwrap().matched.remove(&a);
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

    use yakvc_shared::{IssuerKey, SecretKey, TicketBody};

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
    fn unknown_sessions_are_ignored() {
        let a = player(1);
        let mut m = Matcher::default();
        assert!(m.add_pairs(a.id, sees(&a, &[&player(2)])).is_empty());
        assert!(m.unregister(a.id).is_empty());
        assert!(m.update_addr(a.id, EndpointAddr::new(a.id)).is_empty());
        assert!(m.holders.is_empty());
    }
}

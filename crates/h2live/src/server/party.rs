//! Parties, and who's online. Everyone signed in is in a party: their own
//! at first, which they lead. Members invite others in, and anyone can
//! join a party that's open, up to 16 people with splitscreen guests. The
//! leader can remove members, hand over the lead and make the party invite
//! only; if the leader leaves, the member who has been in it longest leads
//! it. Everyone signed in sees everyone else on the same program (the game,
//! or the launcher), and their party; parties never mix the two. A party
//! that changes while it searches has to search again, and no one can join
//! one playing a match.

use super::{Party, Pc, Server};
use h2net::live::{
    self, Activity, ClientKind, OnlinePlayer, PartyInfo, PartyMember, Privacy, ToPc, MAX_PARTY,
};
use std::collections::{HashMap, HashSet};

/// Unanswered invites a party keeps, and removed members it keeps out.
const INVITES: usize = 32;
const BOOTED: usize = 32;
/// The ONLINE list goes at most this often (seconds).
const ONLINE_EVERY: f64 = 1.0;

/// What players are told.
const PARTY_FULL: &str = "THE PARTY IS FULL";
const PARTY_GONE: &str = "THAT PARTY HAS BROKEN UP";
const INVITE_ONLY: &str = "THAT PARTY IS INVITE ONLY";
const REMOVED: &str = "YOU WERE REMOVED FROM THE PARTY";
const IN_A_MATCH: &str = "THAT PARTY IS IN A MATCH";
/// The game's players and the launcher's can't party together.
pub(super) const OTHER_PROGRAM: &str = "THAT PLAYER IS ON ANOTHER VERSION OF THE GAME";

impl Party {
    fn new(leader: u64) -> Party {
        Party {
            leader,
            privacy: Privacy::Open,
            members: vec![leader],
            invited: Vec::new(),
            booted: Vec::new(),
            activity: Activity::Lobby,
            playlist: live::QUICKMATCH,
            previous_map: None,
            custom_map: String::new(),
        }
    }

    fn has_invited(&self, account: u64) -> bool {
        self.invited.iter().any(|&(to, _)| to == account)
    }
}

impl Server {
    /// Put `account` in a party of its own, which it leads.
    pub(super) fn new_party(&mut self, account: u64) {
        self.party_ids += 1;
        let id = self.party_ids;
        self.parties.insert(id, Party::new(account));
        if let Some(k) = self.pc_of(account) {
            self.pcs[k].party = id;
        }
        self.party_changed(id);
    }

    /// Party `id` changed: its members are told, and everyone online sees.
    pub(super) fn party_changed(&mut self, id: u64) {
        if !self.changed_parties.contains(&id) {
            self.changed_parties.push(id);
        }
        self.online_changed = true;
    }

    /// Take `account` out of party `id`. If they led it, the member there
    /// longest leads it now (and any custom game they hosted is over); if
    /// they were the last, it's gone.
    pub(super) fn remove_member(&mut self, id: u64, account: u64) {
        let Some(party) = self.parties.get_mut(&id) else {
            return;
        };
        party.members.retain(|&a| a != account);
        match party.members.first() {
            Some(&first) => {
                if party.leader == account {
                    party.leader = first;
                    if party.activity == Activity::Custom {
                        self.close_custom(id);
                    }
                } else if party.activity == Activity::Custom {
                    // Their way into the custom game, if they hadn't come.
                    self.unlink(id, Some(account));
                }
                self.party_changed(id);
                self.party_changed_searching(id);
            }
            None => {
                match self.parties[&id].activity {
                    Activity::Searching => {
                        self.matchmaker.cancel(id);
                    }
                    Activity::Custom => self.close_custom(id),
                    _ => {}
                }
                self.parties.remove(&id);
            }
        }
    }

    /// The splitscreen guests on a player's PC.
    fn guests(&self, account: u64) -> usize {
        self.pc_of(account)
            .map_or(0, |k| usize::from(self.pcs[k].guests))
    }

    /// People in `party`, guests too.
    fn size(&self, party: &Party) -> usize {
        party.members.iter().map(|&a| 1 + self.guests(a)).sum()
    }

    /// The party `account` is in, and its number.
    pub(super) fn party_of(&self, account: u64) -> Option<(u64, &Party)> {
        let id = self.pcs[self.pc_of(account)?].party;
        Some((id, self.parties.get(&id)?))
    }

    /// The party `account` leads, if they lead one, and its number.
    pub(super) fn led_by(&mut self, account: u64) -> Option<(u64, &mut Party)> {
        let id = self.pcs[self.pc_of(account)?].party;
        let party = self.parties.get_mut(&id)?;
        (party.leader == account).then_some((id, party))
    }

    pub(super) fn notice(&mut self, account: u64, text: &str) {
        self.tell(account, &ToPc::Notice(text.into()));
    }

    /// `me` asks `who` into their party.
    pub(super) fn invite(&mut self, me: u64, who: u64) {
        let Some(from) = self.accounts.get(&me).map(|a| a.gamertag.clone()) else {
            return;
        };
        if self.pc_of(who).is_none() {
            return;
        }
        if self.client_of(who) != self.client_of(me) {
            return self.notice(me, OTHER_PROGRAM);
        }
        let Some(id) = self.pc_of(me).map(|k| self.pcs[k].party) else {
            return;
        };
        let Some(party) = self.parties.get_mut(&id) else {
            return;
        };
        if party.members.contains(&who) {
            return;
        }
        party.invited.retain(|&(to, _)| to != who);
        let dropped = (party.invited.len() == INVITES).then(|| party.invited.remove(0).0);
        party.invited.push((who, me));
        self.tell(who, &ToPc::Invited { party: id, from });
        // An invite-only party may be one they can join now (and, for the
        // oldest invite dropped to make room, one they no longer can).
        self.friends_changed(who);
        if let Some(dropped) = dropped {
            self.friends_changed(dropped);
        }
    }

    /// Whether `me` could join party `id` now, and if not what they're told
    /// (nothing, if it's their own party already): the party is there, isn't
    /// playing a match, is on their program, is open and hasn't removed them
    /// or else has invited them, and has room for them and their guests.
    /// JOIN_PARTY goes by this, and so does the `joinable` flag of a
    /// FRIENDS entry, so the two never differ.
    pub(super) fn can_join(&self, me: u64, id: u64) -> Result<(), &'static str> {
        let Some(party) = self.parties.get(&id) else {
            return Err(PARTY_GONE);
        };
        if party.members.contains(&me) {
            return Err("");
        }
        if party.activity == Activity::Playing {
            return Err(IN_A_MATCH);
        }
        if self.client_of(party.leader) != self.client_of(me) {
            return Err(OTHER_PROGRAM);
        }
        let closed = party.privacy == Privacy::InviteOnly || party.booted.contains(&me);
        if closed && !party.has_invited(me) {
            return Err(INVITE_ONLY);
        }
        if self.size(party) + 1 + self.guests(me) > MAX_PARTY {
            return Err(PARTY_FULL);
        }
        Ok(())
    }

    /// `me` joins party `id`, leaving their own, if they can (`can_join`):
    /// by invite, or because it's open. In a custom game, they're linked to
    /// its host.
    pub(super) fn join_party(&mut self, me: u64, id: u64) {
        match self.can_join(me, id) {
            Ok(()) => {}
            Err("") => return,
            Err(why) => return self.notice(me, why),
        }
        let Some(k) = self.pc_of(me) else {
            return;
        };
        self.remove_member(self.pcs[k].party, me);
        if let Some(party) = self.parties.get_mut(&id) {
            party.members.push(me);
            party.invited.retain(|&(to, _)| to != me);
            party.booted.retain(|&a| a != me);
        }
        self.pcs[k].party = id;
        self.party_changed(id);
        self.party_changed_searching(id);
        if let Some(party) = self.parties.get(&id) {
            if party.activity == Activity::Custom {
                self.join_custom(id, party.leader, me);
            }
        }
    }

    /// `me` turns down an invite to party `id`; whoever asked hears.
    pub(super) fn decline(&mut self, me: u64, id: u64) {
        let Some(party) = self.parties.get_mut(&id) else {
            return;
        };
        let Some(i) = party.invited.iter().position(|&(to, _)| to == me) else {
            return;
        };
        let (_, from) = party.invited.remove(i);
        // Without the invite, an invite-only party can't be joined.
        self.friends_changed(me);
        let Some(name) = self.accounts.get(&me).map(|a| a.gamertag.clone()) else {
            return;
        };
        self.notice(from, &format!("{name} DECLINED YOUR INVITE"));
    }

    /// `me` leaves their party for one of their own.
    pub(super) fn leave_party(&mut self, me: u64) {
        let Some((id, party)) = self.party_of(me) else {
            return;
        };
        if party.members.len() > 1 {
            self.remove_member(id, me);
            self.new_party(me);
        }
    }

    /// The leader removes `who`, who's on their own again (and needs an
    /// invite to come back).
    pub(super) fn kick(&mut self, me: u64, who: u64) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if who == me || !party.members.contains(&who) {
            return;
        }
        let forgotten = (party.booted.len() == BOOTED).then(|| party.booted.remove(0));
        party.booted.push(who);
        // The oldest removed player dropped to make room may join it again.
        if let Some(forgotten) = forgotten {
            self.friends_changed(forgotten);
        }
        self.remove_member(id, who);
        self.new_party(who);
        self.notice(who, REMOVED);
    }

    /// The leader makes `who` leader (ending any custom game they host).
    pub(super) fn promote(&mut self, me: u64, who: u64) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if party.members.contains(&who) && who != me {
            party.leader = who;
            if party.activity == Activity::Custom {
                self.close_custom(id);
            }
            self.party_changed(id);
        }
    }

    pub(super) fn set_privacy(&mut self, me: u64, privacy: Privacy) {
        if let Some((id, party)) = self.led_by(me) {
            party.privacy = privacy;
            self.party_changed(id);
        }
    }

    /// `me` now plays with `guests` splitscreen guests, if the party has
    /// room for them.
    pub(super) fn set_guests(&mut self, me: u64, guests: u8) {
        let Some((id, party)) = self.party_of(me) else {
            return;
        };
        let others = self.size(party) - 1 - self.guests(me);
        if others + 1 + usize::from(guests) > MAX_PARTY {
            return self.notice(me, PARTY_FULL);
        }
        if let Some(k) = self.pc_of(me) {
            self.pcs[k].guests = guests;
        }
        self.party_changed(id);
        self.party_changed_searching(id);
    }

    /// Tell the members of every party that changed how it is now.
    pub(super) fn send_parties(&mut self) {
        for id in std::mem::take(&mut self.changed_parties) {
            let Some(party) = self.parties.get(&id) else {
                continue;
            };
            let members = party.members.clone();
            let (kind, body) = ToPc::Party(self.party_info(id, party)).write();
            for pc in &mut self.pcs {
                if pc.account.is_some_and(|a| members.contains(&a)) {
                    pc.conn.send(kind, &body);
                }
            }
        }
    }

    fn party_info(&self, id: u64, party: &Party) -> PartyInfo {
        let playlist = self.playlists.iter().find(|p| p.id == party.playlist);
        let members = party.members.iter().filter_map(|&a| {
            let account = self.accounts.get(&a)?;
            // Their level in the ranked playlist the party searches, plays
            // or just played, if it does; otherwise their best.
            let level = match playlist.filter(|p| p.ranked) {
                Some(p) => account.stats(&p.key).map_or(1, |s| s.rank.level),
                None => account.best_level(),
            };
            Some(PartyMember {
                account: a,
                gamertag: account.gamertag.clone(),
                look: account.look,
                best: account.best_level(),
                level,
                guests: self.guests(a) as u8,
            })
        });
        PartyInfo {
            id,
            leader: party.leader,
            privacy: party.privacy,
            activity: party.activity,
            playlist: party.playlist,
            members: members.collect(),
            maps: self.shared_maps(party),
        }
    }

    /// The maps every member of `party` has, the same file, as the
    /// longest-standing member's PC names them.
    pub(super) fn shared_maps(&self, party: &Party) -> Vec<String> {
        let pcs: Vec<&Pc> = party
            .members
            .iter()
            .filter_map(|&a| Some(&self.pcs[self.pc_of(a)?]))
            .collect();
        let Some((first, others)) = pcs.split_first() else {
            return Vec::new();
        };
        let has: Vec<HashSet<(String, u64)>> = others
            .iter()
            .map(|pc| {
                let maps = pc.maps.iter();
                maps.map(|(name, hash)| (name.to_ascii_lowercase(), *hash))
                    .collect()
            })
            .collect();
        let mut seen = HashSet::new();
        let mut shared = Vec::new();
        for (name, hash) in &first.maps {
            let map = (name.to_ascii_lowercase(), *hash);
            if has.iter().all(|maps| maps.contains(&map)) && seen.insert(map.0) {
                shared.push(name.clone());
            }
        }
        shared
    }

    /// Send everyone signed in the ONLINE list of those on the same program
    /// as them, if it changed, at most once a second.
    pub(super) fn send_online(&mut self, now: f64) {
        if !self.online_changed || now - self.online_sent < ONLINE_EVERY {
            return;
        }
        let mut sizes: HashMap<u64, usize> = HashMap::new();
        for pc in self.pcs.iter().filter(|pc| pc.account.is_some()) {
            *sizes.entry(pc.party).or_default() += 1 + usize::from(pc.guests);
        }
        for client in [ClientKind::Viewer, ClientKind::Launcher] {
            self.send_online_to(client, &sizes);
        }
        self.online_changed = false;
        self.online_sent = now;
    }

    /// Send everyone signed in from `client` the ONLINE list of those on
    /// it, with the people in each party (`sizes`).
    fn send_online_to(&mut self, client: ClientKind, sizes: &HashMap<u64, usize>) {
        let on = |pc: &&Pc| pc.account.is_some() && pc.client == client;
        if !self.pcs.iter().any(|pc| on(&pc)) {
            return;
        }
        let players = self.pcs.iter().filter(on).filter_map(|pc| {
            let account = self.accounts.get(&pc.account?)?;
            let party = self.parties.get(&pc.party)?;
            let size = sizes.get(&pc.party).copied().unwrap_or(1);
            let open = party.privacy == Privacy::Open;
            Some(OnlinePlayer {
                account: account.id,
                gamertag: account.gamertag.clone(),
                look: account.look,
                best: account.best_level(),
                activity: party.activity,
                party: pc.party,
                open,
                size: size as u8,
                openings: if open {
                    MAX_PARTY.saturating_sub(size) as u8
                } else {
                    0
                },
            })
        });
        let (kind, body) = ToPc::Online(players.collect()).write();
        for pc in self.pcs.iter_mut().filter(|pc| on(&&**pc)) {
            pc.conn.send(kind, &body);
        }
    }
}

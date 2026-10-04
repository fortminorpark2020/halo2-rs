//! The host's pre-game lobby, as joined PCs see it: what game is next and
//! who's in.

use h2sim::game::{Look, Malformed, Reader, Writer};

/// Most players a lobby lists.
const MAX_PLAYERS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lobby {
    /// The map's file name (without `.map`).
    pub map: String,
    /// The game type, score to win and game options as the host's menus
    /// show them.
    pub game_type: String,
    pub score: String,
    pub options: String,
    /// Teams matter (a team game).
    pub teams: bool,
    pub players: Vec<LobbyPlayer>,
    pub bots: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyPlayer {
    pub name: String,
    pub look: Look,
    pub team: u8,
    /// Playing on another PC (not the host).
    pub remote: bool,
}

impl Lobby {
    pub fn write(&self, w: &mut Writer) {
        w.str(&self.map);
        w.str(&self.game_type);
        w.str(&self.score);
        w.str(&self.options);
        w.bool(self.teams);
        w.u8(self.bots);
        let players = &self.players[..self.players.len().min(MAX_PLAYERS)];
        w.u8(players.len() as u8);
        for p in players {
            w.str(&p.name);
            p.look.write(w);
            w.u8(p.team);
            w.bool(p.remote);
        }
    }

    pub fn read(r: &mut Reader) -> Result<Lobby, Malformed> {
        let mut lobby = Lobby {
            map: r.str()?,
            game_type: r.str()?,
            score: r.str()?,
            options: r.str()?,
            teams: r.bool()?,
            bots: r.u8()?,
            players: Vec::new(),
        };
        let n = r.u8()? as usize;
        if n > MAX_PLAYERS {
            return Err(Malformed);
        }
        for _ in 0..n {
            lobby.players.push(LobbyPlayer {
                name: h2sim::game::clean_name(&r.str()?),
                look: Look::read(r)?,
                team: r.u8()?,
                remote: r.bool()?,
            });
        }
        Ok(lobby)
    }
}

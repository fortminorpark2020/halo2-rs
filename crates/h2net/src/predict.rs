//! A joined PC's own Spartans answer its controls at once, as Halo 2's did
//! for players away from the host. Each tick's controls go to the host
//! numbered, and are run here too; each snapshot says the number of the
//! last ones the host ran for each player. When one comes, our players are
//! put where the host has them, and the controls it hasn't run yet are run
//! again from there. What's shown eases over to where that puts them,
//! unless it's far (a teleporter, a respawn). Only Spartans on foot move
//! here: in a vehicle, or dead, they're shown as the host has them. If the
//! host goes quiet, our players stand where they are until it answers.

use glam::Vec3;
use h2sim::player::Player;
use h2sim::{Command, Game, World};
use std::collections::VecDeque;

/// Controls kept to run again: two seconds' worth.
const MAX_KEPT: usize = 120;
/// Each tick, what's shown keeps this share of how far it is from where
/// our controls really take our player.
const EASE: f32 = 0.8;
/// Farther than this, it goes there at once.
const SNAP: f32 = 1.0;
/// After this many ticks without a snapshot (half a second), our players
/// stand where they are, and our controls don't go, until the host
/// answers: it stands them still too, so nothing needs putting right.
const HOLD: u32 = 30;

#[derive(Default)]
pub(crate) struct Prediction {
    /// Controls sent, numbered, back to the last the host ran for our
    /// players.
    sent: VecDeque<(u32, Vec<(usize, Command)>)>,
    /// The number of the last controls sent.
    number: u32,
    /// From the latest snapshot not yet taken into account: the number of
    /// the last controls the host ran for each player on a joined PC.
    ran: Option<Vec<(usize, u32)>>,
    /// Our players moved here.
    moved: Vec<Moved>,
    /// Ticks since the last snapshot.
    quiet: u32,
}

struct Moved {
    player: usize,
    /// Where our controls take them.
    body: Player,
    /// How far from there they're shown.
    off: Vec3,
}

/// Whether a player moves here by our controls: alive, on foot, and in a
/// game still on.
fn movable(game: &Game, player: usize) -> bool {
    game.players
        .get(player)
        .is_some_and(|p| p.alive && p.seat.is_none() && p.actor.is_none())
        && !game.over()
}

impl Prediction {
    /// The number for the next controls sent.
    pub fn next_number(&mut self) -> u32 {
        self.number = self.number.wrapping_add(1);
        self.number
    }

    /// A new game: nothing sent before it counts.
    pub fn clear(&mut self) {
        self.sent.clear();
        self.ran = None;
        self.moved.clear();
        self.quiet = 0;
    }

    /// Whether the host has gone quiet (see `HOLD`).
    pub fn held(&self) -> bool {
        self.quiet >= HOLD
    }

    /// How far from where our controls take `player` they're shown, if
    /// they move here.
    pub fn off(&self, player: usize) -> Option<Vec3> {
        self.moved
            .iter()
            .find(|m| m.player == player)
            .map(|m| m.off)
    }

    /// What a snapshot says: the number of the last controls the host ran
    /// for each player on a joined PC.
    pub fn ran(&mut self, ran: Vec<(usize, u32)>) {
        self.ran = Some(ran);
    }

    /// A tick here: move our players by `commands`, just sent as `number`,
    /// and keep them to run again.
    pub fn tick(
        &mut self,
        game: &mut Game,
        world: &World,
        number: u32,
        commands: &[(usize, Command)],
    ) {
        self.quiet += 1;
        for &(p, cmd) in commands {
            if !movable(game, p) {
                self.moved.retain(|m| m.player != p);
                continue;
            }
            let k = match self.moved.iter().position(|m| m.player == p) {
                Some(k) => k,
                None => {
                    self.moved.push(Moved {
                        player: p,
                        body: game.players[p].body.clone(),
                        off: Vec3::ZERO,
                    });
                    self.moved.len() - 1
                }
            };
            let m = &mut self.moved[k];
            game.players[p].body.clone_from(&m.body);
            game.walk(world, p, &cmd);
            let shown = &mut game.players[p];
            m.body.clone_from(&shown.body);
            m.off *= EASE;
            shown.body.position += m.off;
        }
        self.sent.push_back((number, commands.to_vec()));
        if self.sent.len() > MAX_KEPT {
            self.sent.pop_front();
        }
    }

    /// After a snapshot: `players`, ours, where the host has them, moved on
    /// by the controls it hasn't run yet.
    pub fn predict(&mut self, game: &mut Game, world: &World, players: &[usize]) {
        let Some(ran) = self.ran.take() else {
            return;
        };
        self.quiet = 0;
        self.moved.retain(|m| players.contains(&m.player));
        let ran_for = |p: usize| ran.iter().find(|r| r.0 == p).map_or(0, |r| r.1);
        for &p in players {
            if !movable(game, p) {
                self.moved.retain(|m| m.player != p);
                continue;
            }
            let last = ran_for(p);
            // As the host left them: jumping only if they let go first.
            let held = self.sent.iter().find(|s| s.0 == last);
            game.players[p].body.jump_held =
                held.is_some_and(|s| s.1.iter().any(|c| c.0 == p && c.1.jump));
            for (_, commands) in self.sent.iter().filter(|s| after(s.0, last)) {
                if let Some((_, cmd)) = commands.iter().find(|c| c.0 == p) {
                    game.walk(world, p, cmd);
                }
            }
            let shown = &mut game.players[p];
            let body = shown.body.clone();
            let before = self.moved.iter().position(|m| m.player == p);
            // Shown where it was, then eased over (unless far off).
            let off = before.map_or(Vec3::ZERO, |k| {
                let m = &self.moved[k];
                m.body.position + m.off - body.position
            });
            let off = if off.length() > SNAP { Vec3::ZERO } else { off };
            shown.body.position += off;
            let m = Moved {
                player: p,
                body,
                off,
            };
            match before {
                Some(k) => self.moved[k] = m,
                None => self.moved.push(m),
            }
        }
        // What the host has run for all our players is no longer needed
        // (but the last, which says whether jump was held).
        let oldest = players.iter().map(|&p| ran_for(p)).min();
        if let Some(oldest) = oldest {
            while self.sent.front().is_some_and(|s| after(oldest, s.0)) {
                self.sent.pop_front();
            }
        }
    }
}

/// Whether controls numbered `a` were sent after `b`.
fn after(a: u32, b: u32) -> bool {
    a.wrapping_sub(b).wrapping_sub(1) < u32::MAX / 2
}

//! Juggernaut: the first kill makes the killer the Juggernaut, tougher and
//! faster than everyone else. Only the Juggernaut's kills score, and
//! killing the Juggernaut scores and takes the role.

use super::{Event, Game, GameType};
use blam_cache::physics::PlayerMovement;

/// The Juggernaut takes this share of the damage dealt to them.
const DAMAGE_TAKEN: f32 = 0.35;
/// And moves this much faster.
const SPEED: f32 = 1.3;

impl Game {
    pub(super) fn juggernaut_damage(&self, victim: usize, amount: f32) -> f32 {
        if self.rules.game_type == GameType::Juggernaut && self.juggernaut == Some(victim) {
            amount * DAMAGE_TAKEN
        } else {
            amount
        }
    }

    /// Score a death in Juggernaut and pass the role on.
    pub(super) fn juggernaut_kill(&mut self, victim: usize, killer: Option<usize>) {
        let killed_juggernaut = self.juggernaut == Some(victim);
        match killer.filter(|&k| k != victim) {
            Some(k) if killed_juggernaut || self.juggernaut.is_none() => {
                self.players[k].score += 1;
                self.make_juggernaut(k);
                self.check_win(k);
            }
            Some(k) if self.juggernaut == Some(k) => {
                self.players[k].score += 1;
                self.check_win(k);
            }
            Some(_) => {}
            // The Juggernaut died by their own hand: the next kill decides.
            None if killed_juggernaut => self.juggernaut = None,
            None => {}
        }
    }

    fn make_juggernaut(&mut self, player: usize) {
        self.juggernaut = Some(player);
        self.players[player].body.movement = self.movement_of(player);
        self.events.push(Event::Juggernaut { player });
    }

    /// How a player moves: the Juggernaut faster than the rest. (A joined
    /// PC needs to know, to move its own players as the host does.)
    pub(super) fn movement_of(&self, player: usize) -> PlayerMovement {
        let mut m = self.movement;
        if self.rules.game_type == GameType::Juggernaut && self.juggernaut == Some(player) {
            m.run_forward *= SPEED;
            m.run_backward *= SPEED;
            m.run_sideways *= SPEED;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use crate::game::tests::game;
    use crate::game::{Event, GameType, Reader, Writer};

    #[test]
    fn killing_the_juggernaut_takes_the_role() {
        let mut g = game();
        g.rules.game_type = GameType::Juggernaut;
        g.rules.score_to_win = 3;
        for _ in 0..3 {
            g.add_player();
        }
        let kill = |g: &mut crate::Game, victim: usize, killer: usize| {
            g.damage(victim, Some(killer), 10_000.0, false);
            g.respawn(victim);
        };
        // An ordinary kill with no Juggernaut yet: the killer becomes it.
        kill(&mut g, 1, 0);
        assert_eq!(g.juggernaut, Some(0));
        assert_eq!(g.players[0].score, 1);
        assert!(g
            .events
            .iter()
            .any(|e| matches!(e, Event::Juggernaut { player: 0 })));
        // Others killing each other score nothing.
        kill(&mut g, 2, 1);
        assert_eq!(g.players[1].score, 0);
        // The Juggernaut is hard to hurt.
        let shield = g.players[0].shield;
        g.damage(0, Some(1), 20.0, false);
        assert!(shield - g.players[0].shield < 10.0);
        // Killing the Juggernaut scores and takes over.
        kill(&mut g, 0, 2);
        assert_eq!(g.juggernaut, Some(2));
        assert_eq!(g.players[2].score, 1);
        kill(&mut g, 1, 2);
        kill(&mut g, 0, 2);
        assert_eq!(g.winner, Some(2));
    }

    #[test]
    fn a_joined_pc_knows_the_juggernaut_runs_faster() {
        let mut g = game();
        g.rules.game_type = GameType::Juggernaut;
        for _ in 0..2 {
            g.add_player();
        }
        g.damage(1, Some(0), 10_000.0, false);
        let mut w = Writer::default();
        g.write_state(&mut w);
        let mut joined = game();
        joined.read_state(&mut Reader::new(&w.0)).unwrap();
        let speed = |g: &crate::Game, p: usize| g.players[p].body.movement;
        assert!(speed(&g, 0).run_forward > speed(&g, 1).run_forward);
        for p in 0..2 {
            assert_eq!(speed(&joined, p), speed(&g, p));
        }
    }
}

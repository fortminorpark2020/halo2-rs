//! Campaign missions in play: placing the scenario's squads as actors.

use crate::scene::Scene;
use blam_cache::ai::AiTeam;
use h2sim::bot::ActorMind;
use h2sim::game::ActorSpawn;
use h2sim::{Bot, Game};

/// The team of a squad that says which side it's on.
fn squad_team(team: AiTeam) -> Option<u8> {
    match team {
        AiTeam::Player | AiTeam::Human => Some(0),
        AiTeam::Covenant | AiTeam::Prophet => Some(1),
        AiTeam::Flood => Some(2),
        AiTeam::Sentinel => Some(3),
        AiTeam::Heretic => Some(4),
        AiTeam::Default | AiTeam::Other(_) => None,
    }
}

/// Place a squad's actors at its starting locations (as many as the
/// difficulty calls for); returns them and their bots (none for a
/// braindead squad). Allies are those on `players_team`.
pub fn place_squad(
    game: &mut Game,
    scene: &Scene,
    squad: usize,
    difficulty: u8,
    players_team: u8,
) -> Vec<(usize, Option<Bot>)> {
    let ai = &scene.ai;
    let Some(s) = ai.squads.get(squad) else {
        return Vec::new();
    };
    let n = s.locations.len();
    let (normal, legendary) = (s.counts.0 as usize, s.counts.1 as usize);
    let count = match difficulty {
        0 | 1 => normal,
        2 => (normal + legendary).div_ceil(2),
        _ => legendary,
    };
    // No count: one at each starting location.
    let count = if count == 0 { n } else { count.min(n) };
    let always = s.locations.iter().filter(|l| l.always).count();
    let mut order: Vec<usize> = (0..n).filter(|&k| s.locations[k].always).collect();
    order.extend((0..n).filter(|&k| !s.locations[k].always));
    let weapon = |i: Option<u16>| i.and_then(|i| ai.weapons.get(i as usize).copied().flatten());
    let mut out = Vec::new();
    for &k in order.iter().take(count.max(always)) {
        let l = &s.locations[k];
        let Some(character) = l
            .character
            .or(s.character)
            .map(usize::from)
            .filter(|&c| c < ai.characters.len())
        else {
            continue;
        };
        let spawn = ActorSpawn {
            character,
            squad: squad as u16,
            position: l.position.into(),
            yaw: l.facing,
            weapon: weapon(l.weapon.or(s.weapon)),
            secondary: weapon(l.secondary.or(s.secondary)),
            difficulty,
            team: squad_team(s.team),
        };
        let Some(i) = game.spawn_actor(spawn) else {
            continue;
        };
        if s.braindead {
            out.push((i, None));
            continue;
        }
        let mut mind = ai.characters[character].mind;
        if difficulty >= 3 {
            mind.accuracy = ai.legendary_accuracy[character];
        }
        if s.blind {
            mind.sight = 0.0;
        }
        let ally = game.players[i].team == players_team;
        let post = spawn.position;
        let mind = ActorMind::new(mind, post, l.facing, squad as u16, ally);
        let seed = (i as u32).wrapping_mul(7919) ^ (squad as u32).wrapping_mul(104_729);
        out.push((i, Some(Bot::actor(seed, mind))));
    }
    out
}

/// The squads whose names start with any of `names` (comma separated;
/// "all" for every squad).
pub fn squads_named(scene: &Scene, names: &str) -> Vec<usize> {
    let names: Vec<&str> = names.split(',').map(str::trim).collect();
    (0..scene.ai.squads.len())
        .filter(|&k| {
            let n = &scene.ai.squads[k].name;
            names.iter().any(|p| *p == "all" || n.starts_with(p))
        })
        .collect()
}

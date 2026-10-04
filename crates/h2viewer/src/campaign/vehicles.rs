//! Squads' vehicles: the vehicles a squad's starting locations put its
//! actors in, and the mission scripts' vehicle functions: getting actors
//! in and out, loading dropships with troops and Ghosts and unloading them,
//! and flying dropships along their courses.

use super::commands::{AI_LOCATION, GIVE_UP};
use super::Ctx;
use blam_cache::ai::SeatType;
use glam::Vec3;
use h2sim::bot::Scripted;
use h2sim::script::{Obj, Value};
use h2sim::vehicle::{Drive, Flight, SeatRole};
use h2sim::{Game, World};
use std::collections::HashMap;

/// Seconds an actor told to get into a vehicle has to walk there before
/// it's put in anyway, and how close to the way in it gets in from.
const BOARD_TIME: f32 = 12.0;
const BOARD_REACH: f32 = 2.5;
/// Close enough to a point flown to, and to one flown by.
const FLOWN_TO: f32 = 1.5;
const FLOWN_BY: f32 = 4.0;
/// The slowest a script flies a dropship, metres per second at full speed.
const FLY_SPEED: f32 = 8.0;
/// A flier with more seats than this is a dropship: it hovers in place
/// while its pilot has nowhere to be.
const DROPSHIP_SEATS: usize = 2;
/// Vehicles at least this big (radius, metres) ride in a dropship's large
/// hold.
const LARGE_CARGO: f32 = 2.0;

#[derive(Debug, Default)]
pub(super) struct Rides {
    /// Actors walking to a vehicle to get in.
    boarding: HashMap<usize, Boarding>,
    /// Actors to get out of their vehicles (dropped from a dropship:
    /// landing unhurt), once the scripts have run.
    exits: Vec<(usize, bool)>,
}

#[derive(Debug, Clone)]
struct Boarding {
    vehicle: usize,
    /// The seats it may take.
    seats: Vec<usize>,
    time: f32,
}

/// A free seat of vehicle `v` for an actor starting in it as `seat` says.
pub(super) fn starting_seat(game: &Game, v: usize, seat: SeatType) -> Option<usize> {
    let veh = game.vehicles.get(v)?;
    let def = &game.vehicle_defs[veh.def];
    let free = |s: &usize| veh.riders[*s].is_none();
    let with = |role: SeatRole| {
        (0..def.seats.len())
            .filter(free)
            .find(|&s| def.seats[s].role == role && !is_cargo(&def.seats[s].animation))
    };
    match seat {
        SeatType::Default | SeatType::Driver => {
            if def.drive == Drive::Fixed {
                with(SeatRole::Gunner).or_else(|| (0..def.seats.len()).find(free))
            } else {
                with(SeatRole::Driver)
            }
        }
        SeatType::Gunner => with(SeatRole::Gunner),
        SeatType::Passenger | SeatType::SmallCargo | SeatType::LargeCargo => {
            with(SeatRole::Passenger)
        }
        SeatType::NoDriver | SeatType::NoVehicle => None,
    }
}

/// A seat that holds a vehicle (a Phantom's hold for Ghosts, or for a
/// Wraith), not a rider.
fn is_cargo(name: &str) -> bool {
    name.contains("_sc") || name.contains("_lc")
}

impl Ctx<'_> {
    /// The vehicle an object is, or rides in.
    fn vehicle_of(&self, o: Obj) -> Option<usize> {
        match o {
            Obj::Vehicle(v) => Some(v).filter(|&v| v < self.game.vehicles.len()),
            Obj::Unit(i) => self.game.players.get(i)?.seat.map(|(v, _)| v),
            Obj::Name(_) => None,
        }
    }

    /// The vehicle an actor drives.
    pub(super) fn driving(&self, actor: usize) -> Option<usize> {
        let (v, s) = self.game.players.get(actor)?.seat?;
        let def = &self.game.vehicle_defs[self.game.vehicles[v].def];
        (def.seats[s].role == SeatRole::Driver).then_some(v)
    }

    /// The seats of vehicle `v` a script's seat value names (`count << 16
    /// | first` of the scenario's seat mappings), or all of them.
    fn named_seats(&self, v: usize, mapping: Option<&Value>) -> Vec<usize> {
        let Some(veh) = self.game.vehicles.get(v) else {
            return Vec::new();
        };
        let def = &self.game.vehicle_defs[veh.def];
        let names: Option<Vec<&String>> = mapping
            .and_then(Value::handle)
            .filter(|&h| h != u32::MAX)
            .map(|h| {
                let (count, first) = ((h >> 16) as usize, (h & 0xFFFF) as usize);
                let all = &self.scene.ai.seat_mappings;
                all.iter()
                    .skip(first)
                    .take(count.max(1))
                    .flatten()
                    .collect()
            });
        (0..def.seats.len())
            .filter(|&s| {
                names
                    .as_ref()
                    .is_none_or(|n| n.contains(&&def.seats[s].animation))
            })
            .collect()
    }

    /// Whether seat `s` of vehicle `v` has someone, or a vehicle, in it.
    fn seat_taken(&self, v: usize, s: usize) -> bool {
        self.game.vehicles[v]
            .riders
            .get(s)
            .is_none_or(Option::is_some)
            || self
                .game
                .vehicles
                .iter()
                .any(|o| !o.destroyed && o.carrier == Some((v, s)))
    }

    /// Put a unit (or a vehicle, as cargo) in the first free one of
    /// `seats` of vehicle `v`.
    fn load(&mut self, o: Obj, v: usize, seats: &[usize]) {
        let Some(s) = seats.iter().copied().find(|&s| !self.seat_taken(v, s)) else {
            return;
        };
        match o {
            Obj::Unit(i) => self.game.enter_vehicle(i, v, s),
            Obj::Vehicle(c) if c != v => self.game.carry_vehicle(c, Some((v, s))),
            _ => {}
        }
    }

    /// Take a vehicle out of the level, with the actors in it.
    pub(super) fn remove_vehicle(&mut self, v: usize) {
        let Some(veh) = self.game.vehicles.get(v) else {
            return;
        };
        let riders: Vec<usize> = veh.riders.iter().flatten().copied().collect();
        for r in riders {
            self.game.erase_actor(r);
        }
        self.game.remove_vehicle(v);
    }

    /// Erase an actor; one driving its squad's own vehicle takes it (and
    /// whoever rides in it) along.
    pub(super) fn erase(&mut self, i: usize) {
        let own = self
            .driving(i)
            .filter(|v| self.scene.vehicles.squad_vehicles.values().any(|w| w == v));
        match own {
            Some(v) => self.remove_vehicle(v),
            None => self.game.erase_actor(i),
        }
    }

    /// Fly the vehicle the commanded actor drives to a point (stopping
    /// there, or on by it) and wait till it's there.
    pub(super) fn fly(&mut self, actor: usize, to: Vec3, near: Option<f32>, stop: bool) -> Value {
        let Some(v) = self.driving(actor) else {
            // A Sentinel flies itself.
            if self.game.players[actor].body.biped.flying {
                let near = near.unwrap_or(if stop { FLOWN_TO } else { FLOWN_BY });
                return self.go(actor, to, near);
            }
            return Value::Void;
        };
        let veh = &self.game.vehicles[v];
        let at = veh.origin(&self.game.vehicle_defs[veh.def]);
        let near = near.unwrap_or(if stop { FLOWN_TO } else { FLOWN_BY });
        let c = self.command_mut(actor);
        if c.fly.is_none_or(|(g, _)| g.distance(to) > 0.5) {
            c.fly = Some((to, stop));
            c.going = 0.0;
        }
        if at.distance(to) < near.max(0.5) || c.going > GIVE_UP {
            c.fly = None;
            if self.st.log {
                println!("command: {actor} flew to {to:.1}");
            }
            return Value::Void;
        }
        self.st.commands.waiting = true;
        Value::Void
    }

    /// The scripts' vehicle functions; `None` for any other function.
    pub(super) fn vehicle_call(&mut self, function: &str, args: &[Value]) -> Option<Value> {
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let objects = |k: usize| arg(k).objects().to_vec();
        let vehicles =
            |list: Vec<usize>| Value::Objects(list.into_iter().map(Obj::Vehicle).collect());
        Some(match function {
            "ai_vehicle_get" => {
                let v = self
                    .actors(&arg(0))
                    .into_iter()
                    .find_map(|i| self.vehicle_of(Obj::Unit(i)));
                vehicles(v.into_iter().collect())
            }
            "ai_vehicle_get_from_starting_location" => {
                let h = arg(0).handle().unwrap_or(u32::MAX);
                let placed = (h & (3 << 30) == AI_LOCATION)
                    .then(|| {
                        let key = (((h >> 16) & 0x3FFF) as u16, (h & 0xFFFF) as u16);
                        self.scene.vehicles.squad_vehicles.get(&key).copied()
                    })
                    .flatten()
                    .filter(|&v| !self.game.vehicles[v].destroyed);
                vehicles(placed.into_iter().collect())
            }
            "ai_vehicle_enter_immediate" | "ai_vehicle_enter" => {
                let Some(v) = objects(1).first().and_then(|&o| self.vehicle_of(o)) else {
                    return Some(Value::Void);
                };
                let seats = self.named_seats(v, args.get(2));
                for i in self.actors(&arg(0)) {
                    if self.game.players[i].seat.is_some_and(|(w, _)| w == v) {
                        continue;
                    }
                    if function == "ai_vehicle_enter_immediate" {
                        self.load(Obj::Unit(i), v, &seats);
                    } else {
                        let seats = seats.clone();
                        let b = Boarding {
                            vehicle: v,
                            seats,
                            time: 0.0,
                        };
                        self.st.rides.boarding.insert(i, b);
                    }
                }
                Value::Void
            }
            // Each of a squad's actors gets into the nearest of the
            // squad's own vehicles (its turrets, say).
            "ai_enter_squad_vehicles" => {
                for s in self.squads(&arg(0)) {
                    let own: Vec<usize> = self
                        .scene
                        .vehicles
                        .squad_vehicles
                        .iter()
                        .filter(|(k, _)| k.0 as usize == s)
                        .map(|(_, &v)| v)
                        .filter(|&v| self.game.vehicles.get(v).is_some_and(|w| !w.destroyed))
                        .collect();
                    let crew: Vec<usize> = self
                        .bots
                        .iter()
                        .filter(|(i, b)| {
                            let p = &self.game.players[*i];
                            b.actor.as_ref().is_some_and(|a| a.squad as usize == s)
                                && p.alive
                                && p.seat.is_none()
                                && !self.st.rides.boarding.contains_key(i)
                        })
                        .map(|(i, _)| *i)
                        .collect();
                    let mut taken: Vec<usize> = Vec::new();
                    for i in crew {
                        let at = self.game.players[i].body.position;
                        let far = |v: usize| {
                            let w = &self.game.vehicles[v];
                            w.origin(&self.game.vehicle_defs[w.def]).distance(at)
                        };
                        let free = own.iter().copied().filter(|v| !taken.contains(v));
                        let Some(v) = free.min_by(|&a, &b| far(a).total_cmp(&far(b))) else {
                            break;
                        };
                        taken.push(v);
                        let seats = self.named_seats(v, None);
                        let b = Boarding {
                            vehicle: v,
                            seats,
                            time: 0.0,
                        };
                        self.st.rides.boarding.insert(i, b);
                    }
                }
                Value::Void
            }
            "ai_vehicle_exit" => {
                for i in self.actors(&arg(0)) {
                    self.st.rides.boarding.remove(&i);
                    self.st.rides.exits.push((i, false));
                }
                Value::Void
            }
            "ai_place_in_vehicle" => {
                let carriers: Vec<usize> = {
                    let mut c: Vec<usize> = self
                        .actors(&arg(1))
                        .into_iter()
                        .filter_map(|i| self.vehicle_of(Obj::Unit(i)))
                        .collect();
                    c.dedup();
                    c
                };
                for q in self.squads(&arg(0)) {
                    for i in self.place(q, None, None) {
                        // In its own vehicle (a Ghost): into a hold;
                        // otherwise a seat.
                        let own = self.driving(i);
                        // A Ghost goes in a small hold, a Spectre or a
                        // Wraith in the large one.
                        let hold = own.map(|g| {
                            let def = &self.game.vehicle_defs[self.game.vehicles[g].def];
                            if def.radius < LARGE_CARGO {
                                "_sc"
                            } else {
                                "_lc"
                            }
                        });
                        for &c in &carriers {
                            let def = &self.game.vehicle_defs[self.game.vehicles[c].def];
                            let seats: Vec<usize> = (0..def.seats.len())
                                .filter(|&s| {
                                    let seat = &def.seats[s];
                                    match hold {
                                        Some(h) => seat.animation.contains(h),
                                        None => {
                                            seat.role == SeatRole::Passenger
                                                && !is_cargo(&seat.animation)
                                        }
                                    }
                                })
                                .filter(|&s| !self.seat_taken(c, s))
                                .collect();
                            if let Some(&s) = seats.first() {
                                match own {
                                    Some(g) => self.game.carry_vehicle(g, Some((c, s))),
                                    None => self.game.enter_vehicle(i, c, s),
                                }
                                break;
                            }
                        }
                    }
                }
                Value::Void
            }
            "vehicle_load_magic" => {
                if let Some(v) = objects(0).first().and_then(|&o| self.vehicle_of(o)) {
                    let seats = self.named_seats(v, args.get(1));
                    for o in objects(2) {
                        self.load(o, v, &seats);
                    }
                }
                Value::Void
            }
            "vehicle_unload" => {
                if let Some(&Obj::Vehicle(v)) = objects(0).first() {
                    for s in self.named_seats(v, args.get(1)) {
                        if let Some(r) = self.game.vehicles[v].riders.get(s).copied().flatten() {
                            self.st.rides.exits.push((r, true));
                        }
                        let held: Vec<usize> = (0..self.game.vehicles.len())
                            .filter(|&c| self.game.vehicles[c].carrier == Some((v, s)))
                            .collect();
                        for c in held {
                            self.game.carry_vehicle(c, None);
                        }
                    }
                }
                Value::Void
            }
            "vehicle_test_seat_list" | "vehicle_test_seat" => {
                let name = arg(1)
                    .handle()
                    .and_then(|h| self.scene.ai.string_ids.get(&h))
                    .cloned()
                    .unwrap_or_default();
                let v = objects(0).first().and_then(|&o| self.vehicle_of(o));
                let riders = objects(2);
                let seated = |o: &Obj| match (o, v) {
                    (&Obj::Unit(i), Some(v)) => self.game.players[i].seat.is_some_and(|(w, s)| {
                        let def = &self.game.vehicle_defs[self.game.vehicles[w].def];
                        w == v && def.seats[s].animation.starts_with(&name)
                    }),
                    _ => false,
                };
                Value::Bool(!riders.is_empty() && riders.iter().all(seated))
            }
            "vehicle_driver" | "vehicle_gunner" | "vehicle_riders" => {
                let role = match function {
                    "vehicle_driver" => Some(SeatRole::Driver),
                    "vehicle_gunner" => Some(SeatRole::Gunner),
                    _ => None,
                };
                let found: Vec<Obj> = objects(0)
                    .first()
                    .and_then(|&o| self.vehicle_of(o))
                    .map(|v| {
                        let veh = &self.game.vehicles[v];
                        let def = &self.game.vehicle_defs[veh.def];
                        (0..def.seats.len())
                            .filter(|&s| role.is_none_or(|r| def.seats[s].role == r))
                            .filter_map(|s| veh.riders[s].map(Obj::Unit))
                            .collect()
                    })
                    .unwrap_or_default();
                Value::Objects(found)
            }
            "ai_in_vehicle_count" => {
                let n = self
                    .actors(&arg(0))
                    .into_iter()
                    .filter(|&i| self.game.players[i].seat.is_some())
                    .count();
                Value::Real(n as f32)
            }
            "object_set_velocity" => {
                if let Some(&Obj::Vehicle(v)) = objects(0).first() {
                    let veh = &mut self.game.vehicles[v];
                    veh.velocity = veh.forward() * arg(1).num();
                    veh.asleep = false;
                }
                Value::Void
            }
            "ai_vehicle_reserve"
            | "ai_vehicle_reserve_seat"
            | "object_set_phantom_power"
            | "object_dynamic_simulation_disable"
            | "vehicle_hover"
            | "ai_vehicle_flip" => Value::Void,
            _ => return None,
        })
    }

    /// After the scripts have run: actors get in and out of vehicles,
    /// dropships fly their courses (or hover where they are), and those
    /// walking to a vehicle head for its door.
    pub(super) fn run_vehicles(&mut self, world: &World, dt: f32) {
        for (i, soft) in std::mem::take(&mut self.st.rides.exits) {
            if soft {
                self.game.unload(world, i);
            } else {
                self.game.exit(world, i);
            }
        }

        // Flying: along a command's course, else hovering.
        let pilots: Vec<(usize, usize)> = self
            .bots
            .iter()
            .filter_map(|&(i, _)| Some((i, self.driving(i)?)))
            .filter(|&(_, v)| {
                let def = &self.game.vehicle_defs[self.game.vehicles[v].def];
                def.drive == Drive::Fly
            })
            .collect();
        for (i, v) in pilots {
            let command = self.st.commands.flying(i);
            let veh = &self.game.vehicles[v];
            let def = &self.game.vehicle_defs[veh.def];
            let at = veh.origin(def);
            let speed = def.max_forward_speed.max(FLY_SPEED) * command.speed;
            let face = command.face.map(|p| {
                let to = p - at;
                to.y.atan2(to.x)
            });
            let course = match command.fly {
                Some((to, stop)) => Some(Flight {
                    to,
                    speed,
                    face,
                    stop,
                }),
                None if def.seats.len() > DROPSHIP_SEATS => Some(Flight {
                    to: veh.flight.map_or(at, |f| f.to),
                    speed,
                    face,
                    stop: true,
                }),
                None => None,
            };
            self.game.vehicles[v].flight = course;
        }

        // Walking to a vehicle to get in.
        let boarding: Vec<(usize, Boarding)> = self.st.rides.boarding.drain().collect();
        for (i, mut b) in boarding {
            let p = &self.game.players[i];
            let Some(veh) = self.game.vehicles.get(b.vehicle).filter(|v| !v.destroyed) else {
                continue;
            };
            if !p.alive || p.seat.is_some() {
                continue;
            }
            let Some(s) = b
                .seats
                .iter()
                .copied()
                .find(|&s| !self.seat_taken(b.vehicle, s))
            else {
                continue;
            };
            let def = &self.game.vehicle_defs[veh.def];
            let door = veh.to_world(def, def.seats[s].entry);
            b.time += dt;
            if door.distance(p.body.position + Vec3::Z * 0.5) < BOARD_REACH || b.time > BOARD_TIME {
                self.game.enter_vehicle(i, b.vehicle, s);
                continue;
            }
            if let Some((_, bot)) = self.bots.iter_mut().find(|(j, _)| *j == i) {
                if let Some(mind) = &mut bot.actor {
                    let s = mind.scripted.unwrap_or_default();
                    mind.scripted = Some(Scripted {
                        go_to: Some(door),
                        ..s
                    });
                }
            }
            self.st.rides.boarding.insert(i, b);
        }
    }
}

//! Mission scripts, run. Every startup, dormant and continuous script is a
//! thread that runs until it sleeps (`sleep`, `sleep_until`), and the
//! scheduler wakes it again when its time comes, 30 times a second as in
//! Halo 2. Control flow, sums and comparisons happen here; everything that
//! touches the level (placing squads, testing trigger volumes, moving
//! doors) goes to the `Host`.

use blam_cache::script::{value_type as vt, NodeKind, ScriptKind, Scripts};

/// Script ticks per second.
pub const TICKS_PER_SECOND: u32 = 30;
/// Asleep until something wakes it.
const NEVER: u32 = u32::MAX;
/// `sleep_until` tests its condition this often (in ticks) unless told.
const DEFAULT_PERIOD: u32 = 30;
/// Steps one thread may take in a tick before it's made to wait for the
/// next (a script stuck in a loop that never sleeps).
const BUDGET: usize = 200_000;
/// Calls nested deeper than this end the thread.
const MAX_DEPTH: usize = 256;
/// Global indices with this bit are the engine's own.
const ENGINE_GLOBAL: u32 = 0x8000;

/// Something in the level a script names: an object placed in the
/// scenario (by its name's index), or a unit in play (an index into the
/// game's players).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Obj {
    Name(u16),
    Unit(usize),
    /// A vehicle in play (an index into the game's vehicles).
    Vehicle(usize),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Void,
    Bool(bool),
    /// Every number (reals, shorts and longs).
    Real(f32),
    /// A thing in the scenario by its index (a squad, a trigger volume, a
    /// script...) or an enum's value; the low 16 bits are the index.
    Handle(u32),
    /// An object, or a list of them (one, or none, for a single object).
    Objects(Vec<Obj>),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Void => false,
            Value::Bool(b) => *b,
            Value::Real(r) => *r != 0.0,
            Value::Handle(h) => *h != u32::MAX,
            Value::Objects(o) => !o.is_empty(),
        }
    }

    pub fn num(&self) -> f32 {
        match self {
            Value::Bool(b) => *b as u8 as f32,
            Value::Real(r) => *r,
            Value::Handle(h) => *h as u16 as i16 as f32,
            Value::Objects(o) => o.len() as f32,
            Value::Void => 0.0,
        }
    }

    /// The index in a handle (none for "none").
    pub fn index(&self) -> Option<u16> {
        match self {
            Value::Handle(h) => Some(*h as u16).filter(|&i| i != u16::MAX),
            Value::Real(r) if *r >= 0.0 => Some(*r as u16),
            _ => None,
        }
    }

    pub fn handle(&self) -> Option<u32> {
        match self {
            Value::Handle(h) => Some(*h),
            _ => None,
        }
    }

    pub fn objects(&self) -> &[Obj] {
        match self {
            Value::Objects(o) => o,
            _ => &[],
        }
    }

    /// Equal as `=` sees it: numbers by value, handles by index.
    fn same(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Handle(a), Value::Handle(b)) => *a as u16 == *b as u16,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Objects(a), Value::Objects(b)) => a == b,
            _ => (self.num() - other.num()).abs() < 1e-6,
        }
    }
}

/// What a type's value is when nothing gives one.
pub fn default_value(value_type: u16) -> Value {
    match value_type {
        vt::BOOLEAN => Value::Bool(false),
        vt::REAL | vt::SHORT | vt::LONG => Value::Real(0.0),
        vt::OBJECT_LIST
        | vt::OBJECT
        | vt::UNIT
        | vt::VEHICLE
        | vt::WEAPON
        | vt::DEVICE
        | vt::SCENERY
        | vt::OBJECT_NAME..=vt::SCENERY_NAME => Value::Objects(Vec::new()),
        vt::VOID => Value::Void,
        _ => Value::Handle(u32::MAX),
    }
}

/// The level, as scripts see it.
pub trait Host {
    /// Do what an engine function does; `None` for one it doesn't know,
    /// which gives its type's default.
    fn call(&mut self, function: &str, args: &[Value], returns: u16) -> Option<Value>;

    /// A script's `print` (debug text the designers left in).
    fn print(&mut self, _text: &str) {}

    /// The actor the thread about to run commands (a command script's),
    /// for `ai_current_actor` and the `cs_` functions.
    fn set_actor(&mut self, _actor: Option<u32>) {}

    /// An engine global's value (`ai_current_actor`...), by name.
    fn engine_global(&mut self, _name: &str) -> Option<Value> {
        None
    }

    /// Whether the call just made is still being carried out (an actor
    /// on its way somewhere): the script waits a tick and calls again.
    fn waiting(&mut self) -> bool {
        false
    }
}

/// The forms evaluated here rather than by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Begin,
    BeginRandom,
    If,
    Set,
    And,
    Or,
    Sleep,
    SleepUntil,
    SleepForever,
    Wake,
    Print,
    /// A static script, by index.
    Script(u16),
    /// Its arguments evaluated, then worked out here.
    Builtin(Builtin),
    /// Its arguments evaluated, then handed to the host.
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Builtin {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Not,
    Unit,
    ListGet,
    ListCount,
    RandomRange,
    RealRandomRange,
    Pin,
}

fn form_of(name: &str) -> Form {
    use Builtin::*;
    Form::Builtin(match name {
        "begin" => return Form::Begin,
        "begin_random" => return Form::BeginRandom,
        "if" => return Form::If,
        "set" => return Form::Set,
        "and" => return Form::And,
        "or" => return Form::Or,
        "sleep" => return Form::Sleep,
        "sleep_until" => return Form::SleepUntil,
        "sleep_forever" => return Form::SleepForever,
        "wake" => return Form::Wake,
        "print" | "log_print" => return Form::Print,
        "+" => Add,
        "-" => Sub,
        "*" => Mul,
        "/" => Div,
        "min" => Min,
        "max" => Max,
        "=" => Eq,
        "!=" => Ne,
        "<" => Lt,
        ">" => Gt,
        "<=" => Le,
        ">=" => Ge,
        "not" => Not,
        "unit" | "object" | "vehicle" | "weapon" => Unit,
        "list_get" => ListGet,
        "list_count" => ListCount,
        "random_range" => RandomRange,
        "real_random_range" => RealRandomRange,
        "pin" => Pin,
        _ => return Form::Host,
    })
}

/// One call being evaluated.
#[derive(Debug, Clone)]
struct Frame {
    node: u16,
    /// The next argument to evaluate (or the stage a special form is at).
    pc: usize,
    values: Vec<Value>,
    /// `sleep_until`: the tick it gives up at.
    deadline: u32,
    /// `begin_random`: the order its forms run in.
    order: Vec<u16>,
}

impl Frame {
    fn new(node: u16) -> Frame {
        Frame {
            node,
            pc: 0,
            values: Vec::new(),
            deadline: NEVER,
            order: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct Thread {
    script: u16,
    kind: ScriptKind,
    stack: Vec<Frame>,
    /// The tick it runs again at.
    wake_at: u32,
    done: bool,
    /// The actor a command script commands.
    actor: Option<u32>,
}

/// What a frame's step leads to.
enum Step {
    /// Evaluate this node, then come back with its value.
    Push(u16),
    Return(Value),
    /// Sleep this many ticks (`NEVER`: until woken), then come back.
    Sleep(u32),
}

/// The mission's scripts in progress.
pub struct Vm {
    threads: Vec<Thread>,
    /// Each script's thread, if it has one.
    thread_of: Vec<Option<usize>>,
    globals: Vec<Value>,
    /// Command scripts waiting for their actor's current one to end.
    queued: Vec<(u32, u16)>,
    /// Each node's form (calls only), and its arguments.
    forms: Vec<Option<Form>>,
    args: Vec<Vec<u16>>,
    now: u32,
    rng: u32,
    /// Print what scripts print, and what goes wrong.
    pub log: bool,
}

impl Vm {
    /// Ready a mission's scripts to run: startup and continuous scripts
    /// from the first tick, dormant ones once woken. Globals get their
    /// starting values now.
    pub fn new(scripts: &Scripts, host: &mut dyn Host) -> Vm {
        let n = scripts.expressions.len();
        let mut forms = vec![None; n];
        let mut args = vec![Vec::new(); n];
        // Calls of scripts by name (some aren't marked as script calls).
        let by_name: std::collections::HashMap<&str, u16> = scripts
            .scripts
            .iter()
            .enumerate()
            .map(|(k, s)| (s.name.as_str(), k as u16))
            .collect();
        for (i, e) in scripts.expressions.iter().enumerate() {
            let i16 = i as u16;
            match e.kind {
                NodeKind::Call => {
                    let name = scripts.function_name(i16);
                    forms[i] = Some(match form_of(name) {
                        Form::Host => by_name.get(name).map_or(Form::Host, |&k| Form::Script(k)),
                        f => f,
                    });
                    args[i] = scripts.arguments(i16);
                }
                NodeKind::ScriptCall => {
                    forms[i] = Some(Form::Script(e.opcode));
                    args[i] = scripts.arguments(i16);
                }
                _ => {}
            }
        }
        let mut threads = Vec::new();
        let mut thread_of = vec![None; scripts.scripts.len()];
        for (k, s) in scripts.scripts.iter().enumerate() {
            let wake_at = match s.kind {
                ScriptKind::Startup | ScriptKind::Continuous => 0,
                ScriptKind::Dormant => NEVER,
                _ => continue,
            };
            let Some(root) = s.root else { continue };
            thread_of[k] = Some(threads.len());
            threads.push(Thread {
                script: k as u16,
                kind: s.kind,
                stack: vec![Frame::new(root)],
                wake_at,
                done: false,
                actor: None,
            });
        }
        let mut vm = Vm {
            threads,
            thread_of,
            globals: scripts
                .globals
                .iter()
                .map(|g| default_value(g.value_type))
                .collect(),
            queued: Vec::new(),
            forms,
            args,
            now: 0,
            rng: 0x2545_f491,
            log: false,
        };
        for (k, g) in scripts.globals.iter().enumerate() {
            if let Some(init) = g.init {
                let v = vm.evaluate(scripts, init, host);
                vm.globals[k] = v;
            }
        }
        vm
    }

    /// Script ticks so far.
    pub fn now(&self) -> u32 {
        self.now
    }

    /// A global's value, by name.
    pub fn global(&self, scripts: &Scripts, name: &str) -> Option<&Value> {
        let k = scripts.globals.iter().position(|g| g.name == name)?;
        self.globals.get(k)
    }

    /// Whether a script's thread is still to finish (or start).
    pub fn running(&self, script: usize) -> bool {
        self.thread_of
            .get(script)
            .copied()
            .flatten()
            .is_some_and(|t| !self.threads[t].done)
    }

    /// Wake a dormant script (or one asleep).
    pub fn wake(&mut self, script: usize) {
        if let Some(t) = self.thread_of.get(script).copied().flatten() {
            let now = self.now;
            let th = &mut self.threads[t];
            if !th.done {
                th.wake_at = th.wake_at.min(now);
            }
        }
    }

    /// One script tick: every thread whose time has come runs until it
    /// sleeps or ends.
    pub fn tick(&mut self, scripts: &Scripts, host: &mut dyn Host) {
        for t in 0..self.threads.len() {
            let th = &self.threads[t];
            if th.done || th.wake_at > self.now {
                continue;
            }
            host.set_actor(th.actor);
            self.run(scripts, t, host);
        }
        host.set_actor(None);
        self.now += 1;
    }

    /// Run a command script for an actor from the next tick, in place of
    /// the one it's running (or, `queue`d, once that one ends).
    pub fn command(&mut self, scripts: &Scripts, script: usize, actor: u32, queue: bool) {
        if queue && self.commanding(actor) {
            self.queued.push((actor, script as u16));
            return;
        }
        if !queue {
            self.stop_command(actor);
        }
        self.start_command(scripts, script, actor);
    }

    fn start_command(&mut self, scripts: &Scripts, script: usize, actor: u32) {
        let Some(root) = scripts.scripts.get(script).and_then(|s| s.root) else {
            return;
        };
        let thread = Thread {
            script: script as u16,
            kind: ScriptKind::CommandScript,
            stack: vec![Frame::new(root)],
            wake_at: self.now,
            done: false,
            actor: Some(actor),
        };
        // A finished command script's place, if there is one.
        match (0..self.threads.len()).find(|&t| {
            let th = &self.threads[t];
            th.done && th.actor.is_some()
        }) {
            Some(t) => self.threads[t] = thread,
            None => self.threads.push(thread),
        }
    }

    /// Stop an actor's command scripts (and those queued).
    pub fn stop_command(&mut self, actor: u32) {
        for th in &mut self.threads {
            if th.actor == Some(actor) && !th.done {
                th.done = true;
                th.stack.clear();
            }
        }
        self.queued.retain(|q| q.0 != actor);
    }

    /// Whether an actor is running a command script.
    pub fn commanding(&self, actor: u32) -> bool {
        self.threads
            .iter()
            .any(|th| th.actor == Some(actor) && !th.done)
    }

    /// Run a static script at once for its value (orders' triggers ask
    /// some); one that sleeps gives its type's default.
    pub fn call(&mut self, scripts: &Scripts, script: usize, host: &mut dyn Host) -> Value {
        match scripts.scripts.get(script).and_then(|s| s.root) {
            Some(root) => self.evaluate(scripts, root, host),
            None => Value::Void,
        }
    }

    /// Evaluate an expression to its value at once (globals' starting
    /// values); one that sleeps gives its type's default.
    fn evaluate(&mut self, scripts: &Scripts, node: u16, host: &mut dyn Host) -> Value {
        if let Some(v) = self.leaf(scripts, node, host) {
            return v;
        }
        let t = self.threads.len();
        self.threads.push(Thread {
            script: u16::MAX,
            kind: ScriptKind::Static,
            stack: vec![Frame::new(node)],
            wake_at: self.now,
            done: false,
            actor: None,
        });
        let v = self.run(scripts, t, host);
        self.threads.pop();
        v.unwrap_or_else(|| {
            default_value(scripts.expression(node).map_or(vt::VOID, |e| e.value_type))
        })
    }

    /// A value that needs no evaluating; engine globals come from the
    /// host.
    fn leaf(&self, scripts: &Scripts, node: u16, host: &mut dyn Host) -> Option<Value> {
        let e = scripts.expression(node)?;
        if e.kind == NodeKind::Global && e.value & 0xFFFF & ENGINE_GLOBAL != 0 {
            if let Some(v) = host.engine_global(scripts.text(e.text)) {
                return Some(v);
            }
        }
        leaf(&self.globals, scripts, node)
    }

    fn random(&mut self) -> u32 {
        xorshift(&mut self.rng)
    }

    /// Run a thread until it sleeps or ends; returns its value if it ended.
    fn run(&mut self, scripts: &Scripts, t: usize, host: &mut dyn Host) -> Option<Value> {
        let mut ret: Option<Value> = None;
        for _ in 0..BUDGET {
            let Some(frame) = self.threads[t].stack.last().cloned() else {
                return self.finish(scripts, t, ret);
            };
            let (step, frame) = self.step(scripts, frame, ret.take(), host);
            let th = &mut self.threads[t];
            *th.stack.last_mut().expect("a frame") = frame;
            match step {
                Step::Push(node) => {
                    let Some(e) = scripts.expression(node) else {
                        ret = Some(Value::Void);
                        continue;
                    };
                    if let Some(v) = self.leaf(scripts, node, host) {
                        ret = Some(v);
                        continue;
                    }
                    let th = &mut self.threads[t];
                    match e.kind {
                        NodeKind::Value | NodeKind::Global => {}
                        NodeKind::Call | NodeKind::ScriptCall => {
                            if th.stack.len() >= MAX_DEPTH {
                                if self.log {
                                    println!("script: calls too deep in script {}", th.script);
                                }
                                th.done = true;
                                th.stack.clear();
                                return None;
                            }
                            th.stack.push(Frame::new(node));
                        }
                    }
                }
                Step::Return(v) => {
                    th.stack.pop();
                    ret = Some(v);
                }
                Step::Sleep(ticks) => {
                    th.wake_at = if ticks == NEVER {
                        NEVER
                    } else {
                        self.now.saturating_add(ticks.max(1))
                    };
                    return None;
                }
            }
        }
        if self.log {
            println!("script: {} ran too long", self.name(scripts, t));
        }
        self.threads[t].wake_at = self.now + 1;
        None
    }

    /// A thread's stack ran out: a continuous script starts over next
    /// tick, the rest are done.
    fn finish(&mut self, scripts: &Scripts, t: usize, ret: Option<Value>) -> Option<Value> {
        let th = &mut self.threads[t];
        if th.kind == ScriptKind::Continuous {
            if let Some(root) = scripts.scripts.get(th.script as usize).and_then(|s| s.root) {
                th.stack.push(Frame::new(root));
                th.wake_at = self.now + 1;
                return ret;
            }
        }
        th.done = true;
        let actor = th.actor;
        if let Some(k) = actor.and_then(|a| self.queued.iter().position(|q| q.0 == a)) {
            let (a, script) = self.queued.remove(k);
            self.start_command(scripts, script as usize, a);
        }
        Some(ret.unwrap_or_default())
    }

    fn name<'a>(&self, scripts: &'a Scripts, t: usize) -> &'a str {
        scripts
            .scripts
            .get(self.threads[t].script as usize)
            .map_or("?", |s| s.name.as_str())
    }

    /// Set another script's wake time (`sleep`/`sleep_forever` naming a
    /// script).
    fn set_wake(&mut self, script: Option<u16>, at: u32) {
        if let Some(t) = script.and_then(|s| self.thread_of.get(s as usize).copied().flatten()) {
            self.threads[t].wake_at = at;
        }
    }

    /// Take one step in evaluating a call, given the value of what it last
    /// asked for.
    fn step(
        &mut self,
        scripts: &Scripts,
        mut f: Frame,
        ret: Option<Value>,
        host: &mut dyn Host,
    ) -> (Step, Frame) {
        let node = f.node as usize;
        let form = self.forms[node].unwrap_or(Form::Host);
        let args = &self.args[node];
        let returns = scripts.expressions[node].value_type;
        let step = match form {
            Form::Begin | Form::BeginRandom => {
                if form == Form::BeginRandom && f.pc == 0 && f.order.is_empty() {
                    let mut order = args.clone();
                    for k in (1..order.len()).rev() {
                        let j = xorshift(&mut self.rng) as usize % (k + 1);
                        order.swap(k, j);
                    }
                    f.order = order;
                }
                let list = if form == Form::BeginRandom {
                    &f.order
                } else {
                    args
                };
                let last = ret.unwrap_or_else(|| default_value(returns));
                if f.pc < list.len() {
                    f.pc += 1;
                    Step::Push(list[f.pc - 1])
                } else {
                    Step::Return(last)
                }
            }
            Form::If => match (f.pc, ret) {
                (0, _) => {
                    f.pc = 1;
                    match args.first() {
                        Some(&c) => Step::Push(c),
                        None => Step::Return(default_value(returns)),
                    }
                }
                (1, Some(c)) => {
                    f.pc = 2;
                    let branch = if c.truthy() { args.get(1) } else { args.get(2) };
                    match branch {
                        Some(&b) => Step::Push(b),
                        None => Step::Return(default_value(returns)),
                    }
                }
                (_, v) => Step::Return(v.unwrap_or_else(|| default_value(returns))),
            },
            Form::Set => match ret {
                None => match args.get(1) {
                    Some(&v) => Step::Push(v),
                    None => Step::Return(Value::Void),
                },
                Some(v) => {
                    if let Some(g) = args.first().and_then(|&g| scripts.expression(g)) {
                        let k = g.value & 0xFFFF;
                        if k & ENGINE_GLOBAL == 0 {
                            if let Some(slot) = self.globals.get_mut(k as usize) {
                                *slot = v.clone();
                            }
                        }
                    }
                    Step::Return(v)
                }
            },
            Form::And | Form::Or => {
                let and = form == Form::And;
                match ret {
                    Some(v) if v.truthy() != and => Step::Return(Value::Bool(!and)),
                    _ if f.pc < args.len() => {
                        f.pc += 1;
                        Step::Push(args[f.pc - 1])
                    }
                    _ => Step::Return(Value::Bool(and)),
                }
            }
            Form::Sleep => {
                if let Some(v) = ret {
                    f.values.push(v);
                }
                if f.pc < args.len() {
                    f.pc += 1;
                    Step::Push(args[f.pc - 1])
                } else if f.pc == args.len() && !args.is_empty() {
                    let ticks = f.values.first().map_or(0.0, Value::num);
                    let other = f.values.get(1).and_then(Value::index);
                    if other.is_some() {
                        let at = if ticks < 0.0 {
                            NEVER
                        } else {
                            self.now + ticks as u32
                        };
                        self.set_wake(other, at);
                        Step::Return(Value::Void)
                    } else {
                        f.pc += 1;
                        Step::Sleep(if ticks < 0.0 { NEVER } else { ticks as u32 })
                    }
                } else {
                    Step::Return(Value::Void)
                }
            }
            Form::SleepForever => {
                if let Some(v) = ret {
                    f.values.push(v);
                }
                if f.pc < args.len() {
                    f.pc += 1;
                    Step::Push(args[f.pc - 1])
                } else if f.pc == args.len() {
                    f.pc += 1;
                    match f.values.first().and_then(Value::index) {
                        Some(s) => {
                            self.set_wake(Some(s), NEVER);
                            Step::Return(Value::Void)
                        }
                        None => Step::Sleep(NEVER),
                    }
                } else {
                    Step::Return(Value::Void)
                }
            }
            Form::SleepUntil => {
                // Stages: evaluate the period and timeout (args 1 and 2),
                // then test the condition until it holds or time's up.
                let extra = args.len().saturating_sub(1).min(2);
                if f.pc < extra {
                    if let Some(v) = ret {
                        f.values.push(v);
                    }
                    f.pc += 1;
                    Step::Push(args[f.pc])
                } else {
                    if f.pc == extra {
                        if let Some(v) = ret.clone() {
                            f.values.push(v);
                        }
                        f.pc += 1;
                        let timeout = f.values.get(1).map_or(-1.0, Value::num);
                        if timeout >= 0.0 {
                            f.deadline = self.now + timeout as u32;
                        }
                        // Test the condition now.
                        return (
                            match args.first() {
                                Some(&c) => Step::Push(c),
                                None => Step::Return(Value::Void),
                            },
                            f,
                        );
                    }
                    let period = f
                        .values
                        .first()
                        .map_or(DEFAULT_PERIOD as f32, Value::num)
                        .max(1.0) as u32;
                    match ret {
                        // Back from the condition.
                        Some(c) if c.truthy() => Step::Return(Value::Bool(true)),
                        Some(_) if self.now >= f.deadline => Step::Return(Value::Bool(false)),
                        Some(_) => Step::Sleep(period),
                        // Back from sleeping: test again.
                        None => match args.first() {
                            Some(&c) => Step::Push(c),
                            None => Step::Return(Value::Void),
                        },
                    }
                }
            }
            Form::Wake => {
                if let Some(v) = ret {
                    if let Some(s) = v.index() {
                        self.wake(s as usize);
                    }
                    Step::Return(Value::Void)
                } else {
                    match args.first() {
                        Some(&a) => Step::Push(a),
                        None => Step::Return(Value::Void),
                    }
                }
            }
            Form::Print => {
                if let Some(e) = args.first().and_then(|&a| scripts.expression(a)) {
                    let text = scripts.text(e.text);
                    if self.log {
                        println!("script: {text}");
                    }
                    host.print(text);
                }
                Step::Return(Value::Void)
            }
            Form::Script(s) => match ret {
                Some(v) if f.pc > 0 => Step::Return(v),
                _ => {
                    f.pc = 1;
                    match scripts.scripts.get(s as usize).and_then(|s| s.root) {
                        Some(root) => Step::Push(root),
                        None => Step::Return(default_value(returns)),
                    }
                }
            },
            Form::Builtin(_) | Form::Host => {
                if let Some(v) = ret {
                    f.values.push(v);
                }
                if f.pc < args.len() {
                    f.pc += 1;
                    Step::Push(args[f.pc - 1])
                } else {
                    let v = match form {
                        Form::Builtin(b) => self.builtin(b, &f.values, returns),
                        _ => {
                            let name = scripts.function_name(f.node);
                            let v = match host.call(name, &f.values, returns) {
                                Some(v) => v,
                                None => {
                                    if self.log {
                                        log_unknown(name);
                                    }
                                    default_value(returns)
                                }
                            };
                            // Not done yet: ask again next tick.
                            if host.waiting() {
                                return (Step::Sleep(1), f);
                            }
                            v
                        }
                    };
                    Step::Return(v)
                }
            }
        };
        (step, f)
    }

    fn builtin(&mut self, b: Builtin, v: &[Value], returns: u16) -> Value {
        use Builtin::*;
        let n = |k: usize| v.get(k).map_or(0.0, Value::num);
        let fold = |f: fn(f32, f32) -> f32| v.iter().skip(1).fold(n(0), |acc, x| f(acc, x.num()));
        let real = |x: f32| {
            Value::Real(match returns {
                vt::SHORT | vt::LONG => x.trunc(),
                _ => x,
            })
        };
        match b {
            Add => real(fold(|a, b| a + b)),
            Sub => real(if v.len() == 1 {
                -n(0)
            } else {
                fold(|a, b| a - b)
            }),
            Mul => real(fold(|a, b| a * b)),
            Div => real(fold(|a, b| if b == 0.0 { 0.0 } else { a / b })),
            Min => real(fold(f32::min)),
            Max => real(fold(f32::max)),
            Eq => Value::Bool(v.len() >= 2 && v[0].same(&v[1])),
            Ne => Value::Bool(v.len() >= 2 && !v[0].same(&v[1])),
            Lt => Value::Bool(n(0) < n(1)),
            Gt => Value::Bool(n(0) > n(1)),
            Le => Value::Bool(n(0) <= n(1)),
            Ge => Value::Bool(n(0) >= n(1)),
            Not => Value::Bool(!v.first().is_some_and(Value::truthy)),
            Unit => Value::Objects(
                v.first()
                    .map_or(&[][..], Value::objects)
                    .iter()
                    .take(1)
                    .copied()
                    .collect(),
            ),
            ListGet => {
                let list = v.first().map_or(&[][..], Value::objects);
                let k = n(1);
                Value::Objects(
                    (k >= 0.0)
                        .then(|| list.get(k as usize).copied())
                        .flatten()
                        .into_iter()
                        .collect(),
                )
            }
            ListCount => Value::Real(v.first().map_or(0, |l| l.objects().len()) as f32),
            RandomRange => {
                let (lo, hi) = (n(0) as i32, n(1) as i32);
                let span = (hi - lo).max(1) as u32;
                Value::Real((lo + (self.random() % span) as i32) as f32)
            }
            RealRandomRange => {
                let t = (self.random() % 10_000) as f32 / 10_000.0;
                Value::Real(n(0) + (n(1) - n(0)) * t)
            }
            Pin => real(n(0).max(n(1)).min(n(2))),
        }
    }
}

/// The value of a node that's no call: a literal or a global.
fn leaf(globals: &[Value], scripts: &Scripts, node: u16) -> Option<Value> {
    let e = scripts.expression(node)?;
    Some(match e.kind {
        // Strings are known by where their text is.
        NodeKind::Value if e.value_type == vt::STRING => Value::Handle(e.text),
        NodeKind::Value => literal(e.value_type, e.value),
        NodeKind::Global => {
            let k = e.value & 0xFFFF;
            if k & ENGINE_GLOBAL != 0 {
                default_value(e.value_type)
            } else {
                globals
                    .get(k as usize)
                    .cloned()
                    .unwrap_or_else(|| default_value(e.value_type))
            }
        }
        NodeKind::Call | NodeKind::ScriptCall => return None,
    })
}

fn xorshift(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

/// A literal's value, read by its type.
fn literal(value_type: u16, value: u32) -> Value {
    match value_type {
        vt::BOOLEAN => Value::Bool(value & 0xFF != 0),
        vt::REAL => Value::Real(f32::from_bits(value)),
        vt::SHORT => Value::Real(value as u16 as i16 as f32),
        vt::LONG => Value::Real(value as i32 as f32),
        vt::OBJECT_LIST
        | vt::OBJECT
        | vt::UNIT
        | vt::VEHICLE
        | vt::WEAPON
        | vt::DEVICE
        | vt::SCENERY
        | vt::OBJECT_NAME..=vt::SCENERY_NAME => Value::Objects(
            Some(value as u16)
                .filter(|&i| i != u16::MAX)
                .map(Obj::Name)
                .into_iter()
                .collect(),
        ),
        _ => Value::Handle(value),
    }
}

/// Name each unknown engine function once.
fn log_unknown(name: &str) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.iter().any(|s| s == name) {
        println!("script: no {name} yet");
        seen.push(name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::script::{Expression, Global, Script};

    /// Builds expression trees for tests.
    #[derive(Default)]
    struct Builder {
        s: Scripts,
    }

    impl Builder {
        fn text(&mut self, t: &str) -> u32 {
            let at = self.s.strings.len() as u32;
            self.s.strings.extend_from_slice(t.as_bytes());
            self.s.strings.push(0);
            at
        }

        fn node(&mut self, e: Expression) -> u16 {
            self.s.expressions.push(e);
            (self.s.expressions.len() - 1) as u16
        }

        fn value(&mut self, value_type: u16, value: u32) -> u16 {
            self.node(Expression {
                opcode: value_type,
                value_type,
                kind: NodeKind::Value,
                next: None,
                text: 0,
                value,
            })
        }

        fn real(&mut self, x: f32) -> u16 {
            self.value(vt::REAL, x.to_bits())
        }

        fn short(&mut self, x: i16) -> u16 {
            self.value(vt::SHORT, x as u16 as u32)
        }

        fn global(&mut self, k: u32) -> u16 {
            self.node(Expression {
                opcode: 0,
                value_type: vt::REAL,
                kind: NodeKind::Global,
                next: None,
                text: 0,
                value: k,
            })
        }

        /// A call: its name node, then the arguments chained by `next`.
        fn call(&mut self, name: &str, args: &[u16]) -> u16 {
            let text = self.text(name);
            let f = self.node(Expression {
                opcode: 0,
                value_type: 2,
                kind: NodeKind::Value,
                next: args.first().copied(),
                text,
                value: 0,
            });
            for w in args.windows(2) {
                self.s.expressions[w[0] as usize].next = Some(w[1]);
            }
            self.node(Expression {
                opcode: 0,
                value_type: vt::REAL,
                kind: NodeKind::Call,
                next: None,
                text,
                value: f as u32,
            })
        }

        fn script(&mut self, name: &str, kind: ScriptKind, root: u16) {
            self.s.scripts.push(Script {
                name: name.into(),
                kind,
                return_type: vt::VOID,
                root: Some(root),
            });
        }
    }

    /// Answers `probe` with the next of its values, and notes calls.
    #[derive(Default)]
    struct Probe {
        calls: Vec<(String, Vec<Value>)>,
        answer: f32,
    }

    impl Host for Probe {
        fn call(&mut self, function: &str, args: &[Value], _: u16) -> Option<Value> {
            self.calls.push((function.to_string(), args.to_vec()));
            (function == "probe").then_some(Value::Real(self.answer))
        }
    }

    /// `walk` takes three calls to finish; `arrived` notes who arrived.
    #[derive(Default)]
    struct Walker {
        actor: Option<u32>,
        walks: u32,
        waiting: bool,
        arrived: Vec<(Option<u32>, f32)>,
    }

    impl Host for Walker {
        fn call(&mut self, function: &str, args: &[Value], _: u16) -> Option<Value> {
            match function {
                "walk" => {
                    self.walks += 1;
                    self.waiting = !self.walks.is_multiple_of(3);
                }
                "arrived" => self.arrived.push((self.actor, args[0].num())),
                _ => return None,
            }
            Some(Value::Void)
        }

        fn set_actor(&mut self, actor: Option<u32>) {
            self.actor = actor;
        }

        fn engine_global(&mut self, name: &str) -> Option<Value> {
            (name == "ai_current_actor").then(|| Value::Real(self.actor.map_or(-1.0, |a| a as f32)))
        }

        fn waiting(&mut self) -> bool {
            std::mem::take(&mut self.waiting)
        }
    }

    #[test]
    fn command_scripts_run_for_their_actor_and_wait_on_its_calls() {
        let mut b = Builder::default();
        // (begin (walk) (arrived ai_current_actor))
        let walk = b.call("walk", &[]);
        let text = b.text("ai_current_actor");
        let me = b.node(Expression {
            opcode: 0,
            value_type: vt::REAL,
            kind: NodeKind::Global,
            next: None,
            text,
            value: ENGINE_GLOBAL,
        });
        let arrived = b.call("arrived", &[me]);
        let body = b.call("begin", &[walk, arrived]);
        b.script("cs_walk", ScriptKind::CommandScript, body);
        let s = b.s;
        let mut host = Walker::default();
        let mut vm = Vm::new(&s, &mut host);
        vm.tick(&s, &mut host);
        assert_eq!(host.walks, 0, "command scripts wait to be run");
        vm.command(&s, 0, 7, false);
        assert!(vm.commanding(7));
        for _ in 0..5 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(host.walks, 3, "walking took three ticks");
        assert_eq!(host.arrived, vec![(Some(7), 7.0)]);
        assert!(!vm.commanding(7));
        // A queued script runs once the one before it ends.
        vm.command(&s, 0, 3, false);
        vm.command(&s, 0, 3, true);
        for _ in 0..10 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(host.arrived.len(), 3);
        // Stopped, it doesn't finish.
        vm.command(&s, 0, 4, false);
        vm.tick(&s, &mut host);
        vm.stop_command(4);
        for _ in 0..5 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(host.arrived.len(), 3);
    }

    #[test]
    fn startup_scripts_sleep_and_set_globals() {
        let mut b = Builder::default();
        b.s.globals.push(Global {
            name: "x".into(),
            value_type: vt::REAL,
            init: None,
        });
        let g = b.global(0);
        let one = b.real(1.0);
        let two = b.real(2.0);
        let sum = b.call("+", &[one, two]);
        let set = b.call("set", &[g, sum]);
        let ten = b.short(10);
        let sleep = b.call("sleep", &[ten]);
        let g2 = b.global(0);
        let five = b.real(5.0);
        let times = b.call("*", &[g2, five]);
        let g3 = b.global(0);
        let set2 = b.call("set", &[g3, times]);
        let body = b.call("begin", &[set, sleep, set2]);
        b.script("main", ScriptKind::Startup, body);
        let s = b.s;
        let mut host = Probe::default();
        let mut vm = Vm::new(&s, &mut host);
        vm.tick(&s, &mut host);
        assert_eq!(vm.global(&s, "x"), Some(&Value::Real(3.0)));
        for _ in 0..9 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(vm.global(&s, "x"), Some(&Value::Real(3.0)), "still asleep");
        vm.tick(&s, &mut host);
        assert_eq!(vm.global(&s, "x"), Some(&Value::Real(15.0)));
        assert!(!vm.running(0));
    }

    #[test]
    fn sleep_until_waits_for_its_condition_and_wakes_dormant_scripts() {
        let mut b = Builder::default();
        // Dormant: (begin (call_me))
        let mark = b.call("call_me", &[]);
        let dormant_body = b.call("begin", &[mark]);
        b.script("later", ScriptKind::Dormant, dormant_body);
        // Startup: (begin (sleep_until (> (probe) 0) 5) (wake later))
        let probe = b.call("probe", &[]);
        let zero = b.real(0.0);
        let test = b.call(">", &[probe, zero]);
        let five = b.short(5);
        let until = b.call("sleep_until", &[test, five]);
        let later = b.value(vt::SCRIPT, 0xffff_0000);
        let wake = b.call("wake", &[later]);
        let body = b.call("begin", &[until, wake]);
        b.script("main", ScriptKind::Startup, body);
        let s = b.s;
        let mut host = Probe::default();
        let mut vm = Vm::new(&s, &mut host);
        let probes = |h: &Probe| h.calls.iter().filter(|c| c.0 == "probe").count();
        for _ in 0..12 {
            vm.tick(&s, &mut host);
        }
        // Tested at ticks 0, 5 and 10.
        assert_eq!(probes(&host), 3);
        assert!(!host.calls.iter().any(|c| c.0 == "call_me"));
        host.answer = 1.0;
        for _ in 0..5 {
            vm.tick(&s, &mut host);
        }
        assert!(host.calls.iter().any(|c| c.0 == "call_me"), "woken");
        assert!(!vm.running(0) && !vm.running(1));
    }

    #[test]
    fn sleep_until_gives_up_after_its_timeout() {
        let mut b = Builder::default();
        let f = b.value(vt::BOOLEAN, 0);
        let one = b.short(1);
        let timeout = b.short(20);
        let until = b.call("sleep_until", &[f, one, timeout]);
        let done = b.call("done", &[]);
        let body = b.call("begin", &[until, done]);
        b.script("main", ScriptKind::Startup, body);
        let s = b.s;
        let mut host = Probe::default();
        let mut vm = Vm::new(&s, &mut host);
        for _ in 0..19 {
            vm.tick(&s, &mut host);
        }
        assert!(host.calls.is_empty());
        for _ in 0..3 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(host.calls.len(), 1);
    }

    #[test]
    fn static_scripts_are_called_and_continuous_ones_repeat() {
        let mut b = Builder::default();
        let two = b.real(2.0);
        let three = b.real(3.0);
        let product = b.call("*", &[two, three]);
        b.script("six", ScriptKind::Static, product);
        let six = b.node(Expression {
            opcode: 0,
            value_type: vt::REAL,
            kind: NodeKind::ScriptCall,
            next: None,
            text: 0,
            value: u32::MAX,
        });
        let say = b.call("say", &[six]);
        let body = b.call("begin", &[say]);
        b.script("loop", ScriptKind::Continuous, body);
        let s = b.s;
        let mut host = Probe::default();
        let mut vm = Vm::new(&s, &mut host);
        for _ in 0..4 {
            vm.tick(&s, &mut host);
        }
        assert_eq!(host.calls.len(), 4, "every tick");
        assert_eq!(host.calls[0].1, vec![Value::Real(6.0)]);
    }

    #[test]
    fn globals_start_with_their_literal_and_global_values() {
        let mut b = Builder::default();
        let thirty = b.short(30);
        let seconds = b.global(0);
        b.s.globals.push(Global {
            name: "seconds".into(),
            value_type: vt::SHORT,
            init: Some(thirty),
        });
        b.s.globals.push(Global {
            name: "copy".into(),
            value_type: vt::SHORT,
            init: Some(seconds),
        });
        let s = b.s;
        let mut host = Probe::default();
        let vm = Vm::new(&s, &mut host);
        assert_eq!(vm.global(&s, "seconds").map(Value::num), Some(30.0));
        assert_eq!(vm.global(&s, "copy").map(Value::num), Some(30.0));
        assert!(host.calls.is_empty(), "literals are not calls");
    }

    #[test]
    fn if_and_or_short_circuit() {
        let mut b = Builder::default();
        let t = b.value(vt::BOOLEAN, 1);
        let f = b.value(vt::BOOLEAN, 0);
        let never = b.call("never", &[]);
        let or = b.call("or", &[t, never]);
        let never2 = b.call("never", &[]);
        let and = b.call("and", &[f, never2]);
        let yes = b.call("yes", &[]);
        let no = b.call("no", &[]);
        let branch = b.call("if", &[and, no, yes]);
        let body = b.call("begin", &[or, branch]);
        b.script("main", ScriptKind::Startup, body);
        let s = b.s;
        let mut host = Probe::default();
        let mut vm = Vm::new(&s, &mut host);
        vm.tick(&s, &mut host);
        let names: Vec<&str> = host.calls.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, ["yes"]);
    }
}

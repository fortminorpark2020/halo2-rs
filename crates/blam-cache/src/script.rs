//! A scenario's compiled scripts: its scripts and globals, and the
//! expression tree they are made of (Halo's Lisp-like scripting language,
//! already parsed into nodes by the tools).

use crate::mapset::MapSet;
use crate::{i16_at, u32_at, Error, Result};

const SCNR_STRING_DATA: usize = 0x1B0;
const SCNR_SCRIPTS: usize = 0x1B8;
const SCNR_GLOBALS: usize = 0x1C0;
const SCNR_EXPRESSIONS: usize = 0x238;
const SCRIPT_SIZE: usize = 0x28;
const GLOBAL_SIZE: usize = 0x28;
const EXPRESSION_SIZE: usize = 0x14;

/// Expression flags.
const PRIMITIVE: u16 = 1 << 0;
const SCRIPT_CALL: u16 = 1 << 1;
const GLOBAL_REF: u16 = 1 << 2;

/// The types of value scripts work with (the ones that matter here).
pub mod value_type {
    pub const VOID: u16 = 0x04;
    pub const BOOLEAN: u16 = 0x05;
    pub const REAL: u16 = 0x06;
    pub const SHORT: u16 = 0x07;
    pub const LONG: u16 = 0x08;
    pub const STRING: u16 = 0x09;
    pub const SCRIPT: u16 = 0x0A;
    pub const TRIGGER_VOLUME: u16 = 0x0D;
    pub const DEVICE_GROUP: u16 = 0x12;
    pub const AI: u16 = 0x13;
    pub const STARTING_PROFILE: u16 = 0x18;
    pub const GAME_DIFFICULTY: u16 = 0x2C;
    pub const TEAM: u16 = 0x2D;
    pub const OBJECT: u16 = 0x32;
    pub const UNIT: u16 = 0x33;
    pub const VEHICLE: u16 = 0x34;
    pub const WEAPON: u16 = 0x35;
    pub const DEVICE: u16 = 0x36;
    pub const SCENERY: u16 = 0x37;
    pub const OBJECT_NAME: u16 = 0x38;
    pub const UNIT_NAME: u16 = 0x39;
    pub const VEHICLE_NAME: u16 = 0x3A;
    pub const WEAPON_NAME: u16 = 0x3B;
    pub const DEVICE_NAME: u16 = 0x3C;
    pub const SCENERY_NAME: u16 = 0x3D;
    pub const OBJECT_LIST: u16 = 0x1F;
}

/// When a script runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    /// Once, when the level starts.
    Startup,
    /// When another script wakes it.
    Dormant,
    /// Every tick.
    Continuous,
    /// Called like a function.
    Static,
    Stub,
    CommandScript,
    Other(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub name: String,
    pub kind: ScriptKind,
    pub return_type: u16,
    pub root: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Global {
    pub name: String,
    pub value_type: u16,
    pub init: Option<u16>,
}

/// What an expression node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A function call or special form: its first child names the
    /// function, the rest are arguments.
    Call,
    /// A literal (a number, a name of something in the scenario...).
    Value,
    /// A call of another script; the value is its index.
    ScriptCall,
    /// A global's value; the value is its index.
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Expression {
    /// For a function name, the function's number.
    pub opcode: u16,
    pub value_type: u16,
    pub kind: NodeKind,
    /// The next argument of the call this is in.
    pub next: Option<u16>,
    /// Its source text, as an offset into the string data.
    pub text: u32,
    /// The value (for a call: its first child's index).
    pub value: u32,
}

impl Expression {
    pub fn first_child(&self) -> Option<u16> {
        index_of(self.value)
    }

    pub fn as_f32(&self) -> f32 {
        f32::from_bits(self.value)
    }

    pub fn as_i16(&self) -> i16 {
        self.value as u16 as i16
    }

    pub fn as_bool(&self) -> bool {
        self.value & 0xFF != 0
    }
}

/// A datum handle's index (none for 0xFFFF).
fn index_of(datum: u32) -> Option<u16> {
    Some(datum as u16).filter(|&i| i != u16::MAX)
}

#[derive(Debug, Clone, Default)]
pub struct Scripts {
    pub scripts: Vec<Script>,
    pub globals: Vec<Global>,
    pub expressions: Vec<Expression>,
    /// NUL-separated source text.
    pub strings: Vec<u8>,
}

impl Scripts {
    /// Source text at an offset into the string data.
    pub fn text(&self, offset: u32) -> &str {
        let from = (offset as usize).min(self.strings.len());
        let rest = &self.strings[from..];
        let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
        std::str::from_utf8(&rest[..end]).unwrap_or("")
    }

    pub fn expression(&self, i: u16) -> Option<&Expression> {
        self.expressions.get(i as usize)
    }

    /// The arguments of a call (after the function name), in order.
    pub fn arguments(&self, call: u16) -> Vec<u16> {
        let mut out = Vec::new();
        let Some(first) = self.expression(call).and_then(|e| e.first_child()) else {
            return out;
        };
        let mut next = self.expression(first).and_then(|e| e.next);
        while let Some(i) = next {
            out.push(i);
            next = self.expression(i).and_then(|e| e.next);
            if out.len() > 4096 {
                break;
            }
        }
        out
    }

    /// The name of the function a call calls.
    pub fn function_name(&self, call: u16) -> &str {
        self.expression(call)
            .and_then(|e| e.first_child())
            .and_then(|f| self.expression(f))
            .map_or("", |f| self.text(f.text))
    }

    /// An expression as source text, for reading scripts.
    pub fn source(&self, i: u16, depth: usize) -> String {
        let Some(e) = self.expression(i) else {
            return "?".into();
        };
        match e.kind {
            NodeKind::Call if depth > 0 => {
                let args: Vec<String> = self
                    .arguments(i)
                    .iter()
                    .map(|&a| self.source(a, depth - 1))
                    .collect();
                format!("({} {})", self.function_name(i), args.join(" "))
            }
            NodeKind::Call => format!("({} ...)", self.function_name(i)),
            NodeKind::ScriptCall => {
                let args: Vec<String> = self
                    .arguments(i)
                    .iter()
                    .map(|&a| self.source(a, depth.saturating_sub(1)))
                    .collect();
                format!("({} {})", self.function_name(i), args.join(" ")).replace(" )", ")")
            }
            NodeKind::Value => match e.value_type {
                value_type::BOOLEAN => e.as_bool().to_string(),
                value_type::REAL => format!("{:?}", e.as_f32()),
                value_type::SHORT => e.as_i16().to_string(),
                value_type::LONG => (e.value as i32).to_string(),
                _ => format!("{}#{:x}", self.text(e.text), e.value),
            },
            NodeKind::Global => self.text(e.text).to_string(),
        }
    }
}

fn ascii(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// The scenario's scripts, globals and expressions.
pub fn scripts(set: &mut MapSet) -> Result<Scripts> {
    let map = &mut set.map;
    let scnr = map
        .tag(map.scenario)
        .cloned()
        .ok_or_else(|| Error::Corrupt("scenario tag missing".into()))?;
    let data = map.read_tag_data(&scnr)?;
    if data.len() < SCNR_EXPRESSIONS + 8 {
        return Err(Error::Corrupt("scenario tag too small".into()));
    }
    let meta = map.meta_region();
    let size = u32_at(&data, SCNR_STRING_DATA) as usize;
    let address = u32_at(&data, SCNR_STRING_DATA + 4);
    let strings = if size > 0 && size < 1 << 24 {
        map.read_in(meta, address, size)?
    } else {
        Vec::new()
    };
    let scripts = map.read_block(meta, &data, SCNR_SCRIPTS, SCRIPT_SIZE)?;
    let globals = map.read_block(meta, &data, SCNR_GLOBALS, GLOBAL_SIZE)?;
    let expressions = map.read_block(meta, &data, SCNR_EXPRESSIONS, EXPRESSION_SIZE)?;
    Ok(Scripts {
        scripts: scripts
            .as_chunks::<SCRIPT_SIZE>()
            .0
            .iter()
            .map(|s| Script {
                name: ascii(&s[..0x20]),
                kind: match i16_at(s, 0x20) as u16 {
                    0 => ScriptKind::Startup,
                    1 => ScriptKind::Dormant,
                    2 => ScriptKind::Continuous,
                    3 => ScriptKind::Static,
                    4 => ScriptKind::Stub,
                    5 => ScriptKind::CommandScript,
                    n => ScriptKind::Other(n),
                },
                return_type: i16_at(s, 0x22) as u16,
                root: index_of(u32_at(s, 0x24)),
            })
            .collect(),
        globals: globals
            .as_chunks::<GLOBAL_SIZE>()
            .0
            .iter()
            .map(|g| Global {
                name: ascii(&g[..0x20]),
                value_type: i16_at(g, 0x20) as u16,
                init: index_of(u32_at(g, 0x24)),
            })
            .collect(),
        expressions: expressions
            .as_chunks::<EXPRESSION_SIZE>()
            .0
            .iter()
            .map(|e| {
                let flags = i16_at(e, 6) as u16;
                Expression {
                    opcode: i16_at(e, 2) as u16,
                    value_type: i16_at(e, 4) as u16,
                    kind: if flags & SCRIPT_CALL != 0 {
                        NodeKind::ScriptCall
                    } else if flags & GLOBAL_REF != 0 {
                        NodeKind::Global
                    } else if flags & PRIMITIVE != 0 {
                        NodeKind::Value
                    } else {
                        NodeKind::Call
                    },
                    next: index_of(u32_at(e, 8)),
                    text: u32_at(e, 0xC),
                    value: u32_at(e, 0x10),
                }
            })
            .collect(),
        strings,
    })
}

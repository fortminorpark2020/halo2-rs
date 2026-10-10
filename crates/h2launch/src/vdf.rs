//! Steam's text KeyValues files, enough for `steamapps\libraryfolders.vdf`:
//! quoted keys and values, nested `{ }` blocks, `//` comments and the
//! backslash escapes. Unquoted tokens are accepted too.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Block(Vec<(String, Value)>),
}

impl Value {
    /// The first child with this key, ignoring case (Steam writes both
    /// "LibraryFolders" and "libraryfolders" over the years).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Block(kids) => kids
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Value::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Block(_) => None,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(&c) = it.peek() {
        match c {
            c if c.is_whitespace() => {
                it.next();
            }
            '{' => {
                it.next();
                out.push(Token::Open);
            }
            '}' => {
                it.next();
                out.push(Token::Close);
            }
            '/' => {
                it.next();
                if it.peek() == Some(&'/') {
                    for c in it.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    return Err("stray '/'".into());
                }
            }
            '"' => {
                it.next();
                let mut s = String::new();
                loop {
                    match it.next() {
                        None => return Err("unterminated string".into()),
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some(o) => {
                                s.push('\\');
                                s.push(o);
                            }
                            None => return Err("unterminated string".into()),
                        },
                        Some(o) => s.push(o),
                    }
                }
                out.push(Token::Str(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    it.next();
                }
                out.push(Token::Str(s));
            }
        }
    }
    Ok(out)
}

fn block(toks: &[Token], pos: &mut usize, top: bool) -> Result<Vec<(String, Value)>, String> {
    let mut kids = Vec::new();
    loop {
        match toks.get(*pos) {
            None if top => return Ok(kids),
            None => return Err("missing '}'".into()),
            Some(Token::Close) if !top => {
                *pos += 1;
                return Ok(kids);
            }
            Some(Token::Close) => return Err("unexpected '}'".into()),
            Some(Token::Open) => return Err("block without a key".into()),
            Some(Token::Str(k)) => {
                *pos += 1;
                let v = match toks.get(*pos) {
                    Some(Token::Str(v)) => {
                        *pos += 1;
                        Value::Str(v.clone())
                    }
                    Some(Token::Open) => {
                        *pos += 1;
                        Value::Block(block(toks, pos, false)?)
                    }
                    _ => return Err(format!("key {k:?} has no value")),
                };
                kids.push((k.clone(), v));
            }
        }
    }
}

/// The whole file as one block of its top-level keys.
pub fn parse(text: &str) -> Result<Value, String> {
    let toks = tokens(text.trim_start_matches('\u{feff}'))?;
    let mut pos = 0;
    Ok(Value::Block(block(&toks, &mut pos, true)?))
}

/// A Steam library folder and the app ids installed in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub path: String,
    pub apps: Vec<String>,
}

/// MCC's Steam app id.
pub const MCC_APP_ID: &str = "976730";

/// The libraries in `libraryfolders.vdf`, both the current form
/// (`"0" { "path" "..." "apps" { ... } }`) and the old one (`"1" "D:\\..."`).
pub fn libraries(text: &str) -> Result<Vec<Library>, String> {
    let root = parse(text)?;
    let top = root
        .get("libraryfolders")
        .ok_or("no \"libraryfolders\" block")?;
    let Value::Block(kids) = top else {
        return Err("\"libraryfolders\" is not a block".into());
    };
    let mut out = Vec::new();
    for (k, v) in kids {
        if !k.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match v {
            Value::Str(path) => out.push(Library {
                path: path.clone(),
                apps: Vec::new(),
            }),
            Value::Block(_) => {
                let Some(path) = v.get("path").and_then(Value::as_str) else {
                    continue;
                };
                let apps = match v.get("apps") {
                    Some(Value::Block(a)) => a.iter().map(|(id, _)| id.clone()).collect(),
                    _ => Vec::new(),
                };
                out.push(Library {
                    path: path.to_string(),
                    apps,
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"contentid"		"123"
		"totalsize"		"0"
		"apps"
		{
			"228980"		"406164286"
			"976730"		"9000000000"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"   // a comment
		"apps"
		{
			"440"		"1"
		}
	}
}
"#;

    const OLD: &str = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1234"
	"ContentStatsID"		"-5"
	"1"		"E:\\Games\\Steam Library"
}
"#;

    #[test]
    fn current_format() {
        let libs = libraries(NEW).unwrap();
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].path, r"C:\Program Files (x86)\Steam");
        assert_eq!(libs[0].apps, vec!["228980", MCC_APP_ID]);
        assert_eq!(libs[1].path, r"D:\SteamLibrary");
        assert_eq!(libs[1].apps, vec!["440"]);
    }

    #[test]
    fn old_format() {
        let libs = libraries(OLD).unwrap();
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].path, r"E:\Games\Steam Library");
        assert!(libs[0].apps.is_empty());
    }

    #[test]
    fn escapes_and_errors() {
        let v = parse(r#""a" "x\"y\\z" "b" { "c" plain }"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_str(), Some("x\"y\\z"));
        assert_eq!(
            v.get("b").unwrap().get("c").unwrap().as_str(),
            Some("plain")
        );
        assert!(parse(r#""a" { "b" "c""#).is_err());
        assert!(parse(r#""a" "unterminated"#).is_err());
        assert!(libraries(r#""other" { }"#).is_err());
    }
}

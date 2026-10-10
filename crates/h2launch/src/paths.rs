//! The paths host slots 46-49 hand back.
//!
//! - 47 `get_game_folder_path(kind)`: our folder for the engine's debug
//!   logs, config, temporary files or its root.
//! - 46 `get_folder_path(kind)`: the same plus `halo2\`.
//! - 48/49 `get_scenario_path_a/_w(builtin, buf, len)`: `buf` comes in
//!   holding a path relative to the MCC folder; it goes back with the MCC
//!   folder in front. HaloX does this in place with lengths that only work
//!   while the relative path is shorter than the MCC folder; we build the
//!   new string separately and fail if it does not fit.

/// `e_folder`.
pub fn folder_name(kind: i32) -> Option<&'static str> {
    match kind {
        0 => Some("debug_logs"),
        1 => Some("config"),
        2 => Some("temporary"),
        3 => Some(""),
        _ => None,
    }
}

/// `<base>\<kind>\`, or `<base>\` for the root; always ends in a
/// backslash. `base` should not end in one.
pub fn game_folder(base: &str, kind: i32) -> Option<String> {
    let name = folder_name(kind)?;
    let base = base.trim_end_matches('\\');
    Some(if name.is_empty() {
        format!("{base}\\")
    } else {
        format!("{base}\\{name}\\")
    })
}

/// `game_folder` plus `halo2\`.
pub fn game_folder_halo2(base: &str, kind: i32) -> Option<String> {
    Some(format!("{}halo2\\", game_folder(base, kind)?))
}

/// Whether a path is already absolute (`C:\...`, `C:/...` or `\\...`).
pub fn is_absolute<T: Copy + Into<u32>>(p: &[T]) -> bool {
    let c = |i: usize| p.get(i).map(|&x| x.into());
    let sep = |v: Option<u32>| v == Some('\\' as u32) || v == Some('/' as u32);
    let letter =
        |v: Option<u32>| v.is_some_and(|v| (v | 0x20) >= 'a' as u32 && (v | 0x20) <= 'z' as u32);
    (letter(c(0)) && c(1) == Some(':' as u32) && sep(c(2))) || (sep(c(0)) && sep(c(1)))
}

/// What to write back into a scenario-path buffer of `cap` units: the MCC
/// folder (`root`, ending in a backslash) then the relative path, without
/// a doubled backslash, plus room for the terminating zero. An absolute
/// path comes back unchanged. `None` when it does not fit.
pub fn prefix_root<T>(root: &[T], rel: &[T], cap: usize) -> Option<Vec<T>>
where
    T: Copy + Into<u32> + PartialEq,
{
    let out: Vec<T> = if is_absolute(rel) {
        rel.to_vec()
    } else {
        let is_sep = |x: &T| {
            let v: u32 = (*x).into();
            v == '\\' as u32 || v == '/' as u32
        };
        let skip = rel.iter().take_while(|x| is_sep(x)).count();
        let mut v = root.to_vec();
        v.extend_from_slice(&rel[skip..]);
        v
    };
    (out.len() < cap).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn folders() {
        let base = r"C:\Users\John\AppData\Local\h2launch\engine";
        assert_eq!(game_folder(base, 1).unwrap(), format!("{base}\\config\\"));
        assert_eq!(game_folder(base, 3).unwrap(), format!("{base}\\"));
        assert_eq!(
            game_folder_halo2(base, 0).unwrap(),
            format!("{base}\\debug_logs\\halo2\\")
        );
        assert_eq!(
            game_folder_halo2(&format!("{base}\\"), 3).unwrap(),
            format!("{base}\\halo2\\")
        );
        assert!(game_folder(base, 4).is_none());
        assert!(game_folder(base, -1).is_none());
    }

    #[test]
    fn scenario_paths() {
        let root = w(r"C:\MCC\");
        let rel = w(r"halo2\h2_maps_win64_dx11\lockout.map");
        let out = prefix_root(&root, &rel, 260).unwrap();
        assert_eq!(
            String::from_utf16(&out).unwrap(),
            r"C:\MCC\halo2\h2_maps_win64_dx11\lockout.map"
        );
        // A leading separator is not doubled.
        let out = prefix_root(&root, &w(r"\maps\x.map"), 260).unwrap();
        assert_eq!(String::from_utf16(&out).unwrap(), r"C:\MCC\maps\x.map");
        // Longer than the root: the case HaloX's arithmetic gets wrong.
        let long = w(&"a".repeat(100));
        assert_eq!(prefix_root(&root, &long, 260).unwrap().len(), 107);
        // Exactly full: needs one more unit for the zero.
        assert!(prefix_root(&root, &long, 107).is_none());
        assert!(prefix_root(&root, &long, 108).is_some());
        // Absolute paths pass through.
        let abs = w(r"D:\other\lockout.map");
        assert_eq!(prefix_root(&root, &abs, 260).unwrap(), abs);
        // Bytes work the same way.
        let out = prefix_root(b"C:\\MCC\\", b"halo2\\x.map", 64).unwrap();
        assert_eq!(out, b"C:\\MCC\\halo2\\x.map");
        assert!(is_absolute(b"\\\\server\\share"));
        assert!(!is_absolute(b"halo2"));
    }
}

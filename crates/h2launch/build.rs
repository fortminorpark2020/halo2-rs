//! Puts the git commit the launcher was built from into the binary, so its
//! log says exactly which build ran.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let commit = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some();
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=H2LAUNCH_GIT={commit}{} ({branch})",
        if dirty { "-dirty" } else { "" }
    );
    // Rebuild when HEAD or the branch it points at moves.
    for path in ["HEAD".to_string(), format!("refs/heads/{branch}")] {
        if let Some(p) = git(&["rev-parse", "--git-path", &path]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
}

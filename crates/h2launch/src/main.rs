//! h2launch: starts MCC's classic Halo 2 engine from the owner's install.
//! See `docs/notes/launcher/README.md`.

#[cfg(windows)]
fn main() {
    std::process::exit(h2launch::win::run());
}

#[cfg(not(windows))]
fn main() {
    println!("h2launch runs on Windows only");
}

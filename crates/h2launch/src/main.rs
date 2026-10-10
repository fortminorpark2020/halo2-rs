//! h2launch: starts MCC's classic Halo 2 engine from the owner's install.
//! See `docs/notes/launcher/README.md`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(h2launch::lobby::main(args));
}

//! `--check`: look over the install and print what a launch would use,
//! without loading `halo2.dll` or starting the engine. Everything here is
//! read-only.

use windows::core::Interface;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};

use super::{engine, gfx};
use crate::cli::Args;
use crate::expected;
use crate::mccroot;
use crate::pe::Pe;

pub fn run(args: &Args) -> i32 {
    println!("h2launch {} --check", crate::BUILD);
    let (root, candidates) = engine::find_root(args.mcc.as_deref());
    println!("\nMCC folder candidates (in order):");
    for (c, ok) in &candidates {
        println!(
            "  [{}] {}  ({})",
            if *ok { "found" } else { "  -  " },
            c.path,
            c.source
        );
    }
    let Some(root) = root else {
        println!("\nMCC was not found. Pass --mcc <folder> (the one holding halo2\\halo2.dll).");
        return 1;
    };
    println!("\nUsing MCC folder: {root}");

    let halo2_dir = mccroot::join(&root, "halo2");
    println!("\nhalo2 folder: {halo2_dir}");
    for name in [
        "halo2.dll",
        "MOPP.dll",
        "mss64.dll",
        "mss64dsp.flt",
        "game.cfg",
        "user.cfg",
    ] {
        let path = mccroot::join(&halo2_dir, name);
        match std::fs::metadata(&path) {
            Ok(m) => println!("  {name}: {} bytes", m.len()),
            Err(_) => println!("  {name}: MISSING"),
        }
    }

    let dll_path = mccroot::join(&halo2_dir, "halo2.dll");
    let mut problems = 0;
    match std::fs::read(&dll_path) {
        Ok(bytes) => problems += report_dll(&bytes),
        Err(e) => {
            println!("\nhalo2.dll could not be read: {e}");
            return 1;
        }
    }

    // groundhog.
    let gh = mccroot::join(&root, r"groundhog\groundhog.dll");
    println!(
        "\ngroundhog.dll: {} ({})",
        if std::path::Path::new(&gh).is_file() {
            "present"
        } else {
            "missing"
        },
        if args.groundhog {
            "would be loaded (--groundhog)"
        } else {
            "not loaded by default"
        }
    );

    // The map and variant a launch would use.
    let map = match &args.map {
        Some(m) => crate::maps::find(m).expect("checked by the parser"),
        None => crate::maps::by_id(crate::maps::LOCKOUT).expect("Lockout is in the table"),
    };
    let map_file = mccroot::join(&halo2_dir, &format!(r"h2_maps_win64_dx11\{}.map", map.file));
    println!("\nMap: {} (id {})", map.display, map.id);
    println!(
        "  {map_file}: {}",
        std::fs::metadata(&map_file)
            .map_or_else(|_| "MISSING".into(), |m| format!("{} bytes", m.len()))
    );
    for shared in ["shared.map", "mainmenu.map"] {
        let p = mccroot::join(&halo2_dir, &format!(r"h2_maps_win64_dx11\{shared}"));
        if !std::path::Path::new(&p).is_file() {
            println!("  {shared}: MISSING");
            problems += 1;
        }
    }
    let variant = crate::cli::variant_path(&root, args.variant.as_deref());
    println!("\nVariant: {variant}");
    println!(
        "  {}",
        std::fs::metadata(&variant)
            .map_or_else(|_| "MISSING".into(), |m| format!("{} bytes", m.len()))
    );

    // The DirectX helper DLL the engine delay-loads and that is not part
    // of Windows.
    check_system_dll("d3dx11_43.dll");
    check_system_dll("XAudio2_9.dll");

    println!("\nDisplay adapters:");
    list_adapters();

    println!(
        "\n{}",
        if problems == 0 {
            "Looks ready. Run h2launch with no arguments to play, or add --quit-after and --screenshot for an unattended test."
        } else {
            "Some files are missing or do not match (see above). The launch may still be attempted."
        }
    );
    0
}

/// Returns the number of problems found.
fn report_dll(bytes: &[u8]) -> u32 {
    let mut problems = 0;
    println!("\nhalo2.dll:");
    println!("  size: {} bytes", bytes.len());
    let sha = crate::pe::sha256_hex(bytes);
    let sha_ok = sha == expected::HALO2_SHA256;
    println!(
        "  SHA-256: {sha}{}",
        if sha_ok {
            " (matches the researched build)"
        } else {
            " (does NOT match the researched build 1.3528; numbers may differ)"
        }
    );
    let pe = match Pe::parse(bytes) {
        Ok(p) => p,
        Err(e) => {
            println!("  PE: could not parse ({e})");
            return problems + 1;
        }
    };
    match pe.versions() {
        Some((file, _)) => {
            let v = crate::pe::version_string(file);
            let ok = file == expected::HALO2_VERSION;
            println!(
                "  FileVersion: {v}{}",
                if ok { "" } else { " (expected 1.3528.0.0)" }
            );
            if !ok {
                problems += 1;
            }
        }
        None => println!("  FileVersion: not found"),
    }
    let ts_ok = pe.timestamp == expected::HALO2_TIMESTAMP;
    println!(
        "  TimeDateStamp: {:#010x}{}",
        pe.timestamp,
        if ts_ok { "" } else { " (expected 0x68A0F0F2)" }
    );
    let si_ok = pe.size_of_image == expected::HALO2_SIZE_OF_IMAGE;
    println!(
        "  SizeOfImage: {:#x}{}",
        pe.size_of_image,
        if si_ok { "" } else { " (expected 0x2A38000)" }
    );
    println!("  ImageBase: {:#x}", pe.image_base);
    println!("  sections:");
    for s in &pe.sections {
        println!(
            "    {:<8} rva {:#010x} vsize {:#010x} raw {:#010x} {}",
            s.name,
            s.rva,
            s.virtual_size,
            s.raw_size,
            s.access()
        );
    }
    match pe.exports() {
        Ok(exports) => {
            let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
            println!("  exports ({}): {}", names.len(), names.join(", "));
            for need in expected::HALO2_EXPORTS {
                if !names.contains(&need) {
                    println!("    missing export: {need}");
                    problems += 1;
                }
            }
        }
        Err(e) => {
            println!("  exports: {e}");
            problems += 1;
        }
    }
    match pe.imports() {
        Ok(i) => println!("  imports ({}): {}", i.len(), i.join(", ")),
        Err(e) => println!("  imports: {e}"),
    }
    match pe.delay_imports() {
        Ok(d) => {
            println!("  delay-load imports ({}):", d.len());
            for name in &d {
                let present = dll_resolves(name);
                println!(
                    "    {name}{}",
                    if present {
                        ""
                    } else {
                        "  (not found in the search path)"
                    }
                );
            }
        }
        Err(e) => println!("  delay-load imports: {e}"),
    }
    problems
}

/// Whether a bare DLL name resolves on the current search path, without
/// keeping it loaded. Uses LoadLibraryExW with DONT-RESOLVE semantics via
/// the datafile flag so no DLL code runs.
fn dll_resolves(name: &str) -> bool {
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_AS_DATAFILE};
    let w = crate::util::wide(name);
    // SAFETY: AS_DATAFILE maps the file without running its code.
    unsafe {
        match LoadLibraryExW(
            windows::core::PCWSTR(w.as_ptr()),
            None,
            LOAD_LIBRARY_AS_DATAFILE,
        ) {
            Ok(m) if !m.is_invalid() => {
                let _ = FreeLibrary(m);
                true
            }
            _ => false,
        }
    }
}

fn check_system_dll(name: &str) {
    println!(
        "\n{name}: {}",
        if dll_resolves(name) {
            "found on the search path"
        } else {
            "NOT found (the engine delay-loads it; install the DirectX June 2010 runtime if a launch fails)"
        }
    );
}

fn list_adapters() {
    // SAFETY: DXGI enumeration; this does not load halo2.dll.
    unsafe {
        let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
            Ok(f) => f,
            Err(e) => {
                println!("  (could not create a DXGI factory: {e})");
                return;
            }
        };
        let mut i = 0;
        loop {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(i) {
                Ok(a) => a,
                Err(_) => break,
            };
            if let Ok(a1) = adapter.cast::<IDXGIAdapter1>() {
                println!("  {}", gfx::adapter_line(&a1));
            }
            i += 1;
        }
        if i == 0 {
            println!("  (none)");
        }
    }
}

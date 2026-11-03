//! Builds the trust settings into the launcher and, on Windows, the icon.
//!
//! - Public keys: `UTSIM_CHAVE_PUBLICA` (one or more `ed25519:…`, separated by `;`) or the
//!   lines of `confianca/chave_publica.txt`. Without any, the launcher refuses every manifest
//!   (development build).
//! - Manifest address: `UTSIM_MANIFESTO_URL` or `confianca/manifesto_url.txt`.

use std::fmt::Write as _;
use std::path::Path;

fn read(env: &str, file: &str) -> String {
    println!("cargo:rerun-if-env-changed={env}");
    std::env::var(env)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::fs::read_to_string(file).ok())
        .unwrap_or_default()
}

fn main() {
    // The folder (always present) rather than the files, which may not exist.
    println!("cargo:rerun-if-changed=confianca");
    let keys: Vec<String> = read("UTSIM_CHAVE_PUBLICA", "confianca/chave_publica.txt")
        .split([';', '\n'])
        .map(str::trim)
        .filter(|l| l.starts_with("ed25519:"))
        .map(String::from)
        .collect();
    let url = read("UTSIM_MANIFESTO_URL", "confianca/manifesto_url.txt")
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("")
        .to_string();

    let mut code = String::from("pub const PUBLIC_KEYS: &[&str] = &[");
    for k in &keys {
        let _ = write!(code, "{k:?}, ");
    }
    let _ = writeln!(code, "];\npub const MANIFEST_URL: &str = {url:?};");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_default()).join("confianca.rs");
    std::fs::write(out, code).expect("OUT_DIR");

    println!("cargo:rerun-if-changed=../../assets/icone/familia/launcher.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icone/familia/launcher.ico");
        res.set("ProductName", "UTSim Launcher");
        res.set(
            "FileDescription",
            "UTSim Launcher: instala e atualiza os aplicativos da família UTSim",
        );
        res.set("LegalCopyright", "© 2026 Jandrei");
        if let Err(e) = res.compile() {
            println!("cargo:warning=não foi possível embutir o ícone: {e}");
        }
    }
}

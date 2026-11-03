//! What the launcher window does, without the window: fetch and check the manifest, install
//! a release, open a product. The window itself is [`janela`] (drawn with egui, run by eframe
//! in `main.rs`); [`catalogo`] turns the manifest into what the cards show and [`abertura`]
//! is the opening animation.

pub mod abertura;
pub mod catalogo;
pub mod janela;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::download::{self, Job, Progress};
use utsim_launcher_core::install::{self, Layout};
use utsim_launcher_core::manifest::{Manifest, Package, Product, Release};
use utsim_launcher_core::report::Log;
use utsim_launcher_core::shortcuts::{self, Shortcut};
use utsim_launcher_core::state::LauncherState;
use utsim_launcher_core::transport::{self, Transport};
use utsim_launcher_core::{Error, Result, VerifyingKey, drive, signature};

mod trust {
    include!(concat!(env!("OUT_DIR"), "/confianca.rs"));
}

/// Public keys built into this launcher.
pub fn built_in_keys() -> Vec<VerifyingKey> {
    trust::PUBLIC_KEYS
        .iter()
        .filter_map(|k| signature::public_from_text(k).ok())
        .collect()
}

/// Manifest address built into this launcher (may be empty in development builds).
pub fn built_in_manifest_url() -> &'static str {
    trust::MANIFEST_URL
}

/// How many versions besides the active one are kept after an update (for going back).
pub const KEEP_PREVIOUS: usize = 1;
const MANIFEST_LIMIT: u64 = 16 << 20;
const CACHE_FILE: &str = "manifesto_cache.json";

/// Everything the launcher works with.
pub struct Env {
    pub layout: Layout,
    pub trusted: Vec<VerifyingKey>,
    pub manifest_url: String,
    pub transport: Arc<dyn Transport>,
    pub log: Log,
    pub platform: String,
    /// Program the shortcuts point at (this launcher); `None` creates no shortcut.
    pub launcher_exe: Option<PathBuf>,
}

impl Env {
    /// The launcher as installed on this machine.
    pub fn standard() -> Result<Self> {
        let root = Layout::default_root()
            .ok_or_else(|| Error::Install("não encontrei a pasta do usuário".into()))?;
        let layout = Layout::new(root);
        let log = Log::new(&layout.logs(), "launcher.log");
        Ok(Self {
            trusted: built_in_keys(),
            manifest_url: built_in_manifest_url().to_string(),
            transport: Arc::new(transport::HttpTransport::new()),
            platform: utsim_launcher_core::manifest::current_platform().to_string(),
            launcher_exe: std::env::current_exe().ok(),
            layout,
            log,
        })
    }

    /// The manifest address in use: the tester's override, else the built-in one.
    pub fn manifest_url<'a>(&'a self, state: &'a LauncherState) -> &'a str {
        state
            .manifest_url
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or(&self.manifest_url)
    }
}

/// Downloads, checks and caches the manifest.
pub fn fetch_manifest(env: &Env, state: &mut LauncherState) -> Result<Manifest> {
    let url = env.manifest_url(state).trim().to_string();
    if url.is_empty() {
        return Err(Error::Install(
            "este launcher não sabe onde está a lista de versões (build sem manifesto_url)".into(),
        ));
    }
    let url = drive::direct_url(&url);
    let resp = env.transport.get(&url, None)?;
    if resp.is_html() {
        let page = resp.text(2 << 20)?;
        return Err(Error::Drive(drive::interpret(&page, &url).message().into()));
    }
    if resp.status != 200 {
        return Err(Error::Network(format!(
            "a lista de versões respondeu HTTP {}",
            resp.status
        )));
    }
    let text = resp.text(MANIFEST_LIMIT)?;
    let m = Manifest::from_signed(&text, &env.trusted)?;
    state.accept_sequence(m.sequence)?;
    let _ = install::write_atomic(&env.layout.root.join(CACHE_FILE), text.as_bytes());
    env.log.line(&format!(
        "manifesto aceito: sequência {}, {} produto(s)",
        m.sequence,
        m.products.len()
    ));
    Ok(m)
}

/// The last manifest accepted, for working offline (checked again).
pub fn cached_manifest(env: &Env) -> Option<Manifest> {
    let text = std::fs::read_to_string(env.layout.root.join(CACHE_FILE)).ok()?;
    Manifest::from_signed(&text, &env.trusted).ok()
}

/// Download address of a package. A package without a link yet is still reachable when the
/// manifest itself is a local file (the author testing the publication folder).
pub fn package_url(manifest_url: &str, pkg: &Package) -> Option<String> {
    if !pkg.url.trim().is_empty() {
        return Some(pkg.url.clone());
    }
    let manifest_path = transport::file_url_path(manifest_url)?;
    let dir = manifest_path.parent()?;
    (!pkg.file.is_empty()).then(|| transport::file_url(&dir.join("pacotes").join(&pkg.file)))
}

/// Whether `code` appears among a package's wraps (cheap check, no Argon2).
pub fn listed_for(pkg: &Package, code: &AccessCode) -> bool {
    pkg.wraps.iter().any(|w| w.id == code.id())
}

/// Whether this launcher can install `pkg`: it is open to everyone, or `code` is listed.
pub fn can_install(pkg: &Package, code: Option<&AccessCode>) -> bool {
    pkg.is_open() || code.is_some_and(|c| listed_for(pkg, c))
}

/// Downloads (resuming), checks, installs and activates a release; then removes old
/// versions and refreshes the shortcuts. Returns the version folder.
pub fn install_release(
    env: &Env,
    state: &LauncherState,
    product: &Product,
    release: &Release,
    code: Option<&AccessCode>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf> {
    let pkg = release.package(&env.platform).ok_or_else(|| {
        Error::Install(format!(
            "a versão {} não tem pacote para {}",
            release.version, env.platform
        ))
    })?;
    if !can_install(pkg, code) {
        return Err(Error::NoAccess);
    }
    let url = package_url(env.manifest_url(state), pkg).ok_or_else(|| {
        Error::Install("o autor ainda não publicou o link deste pacote; tente mais tarde".into())
    })?;
    let file = env.layout.package_file(&product.id, release, pkg);
    env.log.line(&format!(
        "baixando {} {} ({} bytes)",
        product.id, release.version, pkg.size
    ));
    let job = Job {
        url: &url,
        dest: &file,
        expected_size: Some(pkg.size),
        cancel,
    };
    download::download_with_retries(
        env.transport.as_ref(),
        &job,
        progress,
        3,
        Duration::from_secs(3),
    )?;
    let dir = match install::install(&env.layout, &product.id, release, pkg, code, &file) {
        Ok(d) => d,
        Err(e) => {
            if matches!(e, Error::Integrity(_)) {
                // A bad file must not be resumed next time.
                let _ = std::fs::remove_file(&file);
            }
            return Err(e);
        }
    };
    install::activate(&env.layout, &product.id, &release.version)?;
    let _ = std::fs::remove_file(&file);
    match install::remove_old(&env.layout, &product.id, KEEP_PREVIOUS) {
        Ok(removed) if !removed.is_empty() => env.log.line(&format!(
            "versões antigas removidas: {}",
            removed.join(", ")
        )),
        Err(e) => env.log.line(&format!("não removi versões antigas: {e}")),
        _ => {}
    }
    if let Err(e) = refresh_shortcuts(env, product, state.desktop_shortcuts) {
        env.log.line(&format!("atalho não criado: {e}"));
    }
    env.log.line(&format!(
        "instalado e ativo: {} {}",
        product.id, release.version
    ));
    Ok(dir)
}

/// Start-menu (and optionally desktop) shortcut pointing at `launcher --abrir <id>`.
pub fn refresh_shortcuts(env: &Env, product: &Product, desktop: bool) -> Result<()> {
    let Some(exe) = &env.launcher_exe else {
        return Ok(());
    };
    let sc = Shortcut {
        name: product.name.clone(),
        target: exe.clone(),
        args: vec!["--abrir".into(), product.id.clone()],
        description: format!("Abre {} (versão ativa)", product.name),
        icon: Some(exe.clone()),
    };
    if let Some(dir) = shortcuts::start_menu_dir() {
        shortcuts::create(&sc, &dir)?;
    }
    if let Some(dir) = shortcuts::desktop_dir() {
        if desktop {
            shortcuts::create(&sc, &dir)?;
        } else {
            shortcuts::remove(&dir, &sc.name)?;
        }
    }
    Ok(())
}

/// Removes a product entirely: versions and shortcuts.
pub fn uninstall_product(env: &Env, product_id: &str, name: &str) -> Result<()> {
    let dir = env.layout.product(product_id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| {
            Error::Install(format!(
                "não consegui remover {} ({e}); feche o programa e tente de novo",
                dir.display()
            ))
        })?;
    }
    for d in [shortcuts::start_menu_dir(), shortcuts::desktop_dir()]
        .into_iter()
        .flatten()
    {
        let _ = shortcuts::remove(&d, name);
    }
    env.log.line(&format!("desinstalado: {product_id}"));
    Ok(())
}

/// Error report: summary + launcher logs, saved in `dest` (normally Downloads).
pub fn error_report(env: &Env, state: &LauncherState, dest: &Path) -> Result<PathBuf> {
    let mut info = utsim_launcher_core::report::system_summary();
    info.push_str(&format!(
        "pasta: {}\nchaves embutidas: {}\nmanifesto: {}\núltima sequência: {}\n",
        env.layout.root.display(),
        env.trusted.len(),
        if env.manifest_url(state).is_empty() {
            "(nenhum)"
        } else {
            "configurado"
        },
        state.last_sequence
    ));
    if let Some(code) = state
        .code
        .as_deref()
        .and_then(|c| AccessCode::parse(c).ok())
    {
        // The public id only: never the secret.
        info.push_str(&format!("testador (id): {}\n", code.id()));
    }
    info.push_str("\nversões instaladas:\n");
    if let Ok(dir) = std::fs::read_dir(env.layout.products()) {
        for p in dir.filter_map(|e| e.ok()) {
            let id = p.file_name().to_string_lossy().to_string();
            let active = install::active_version(&env.layout, &id).unwrap_or_default();
            let all = install::installed_versions(&env.layout, &id);
            info.push_str(&format!(
                "  {id}: ativa {active}; instaladas {}\n",
                all.join(", ")
            ));
        }
    }
    utsim_launcher_core::report::create_report(dest, &info, &[env.layout.logs()])
}

/// The Downloads folder (where the report is saved), else the home folder.
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = home.join("Downloads");
    if d.is_dir() { d } else { home }
}

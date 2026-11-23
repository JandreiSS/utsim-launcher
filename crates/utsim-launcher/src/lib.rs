//! What the launcher window does, without the window: fetch and check the manifest, install
//! a release, open a product. The window itself is [`janela`] (drawn with egui, run by eframe
//! in `main.rs`); [`catalogo`] turns the manifest into what the cards show and [`abertura`]
//! is the opening animation.

pub mod abertura;
pub mod catalogo;
pub mod janela;
pub mod texto;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::download::{self, Job, Progress};
use utsim_launcher_core::install::{self, Layout};
use utsim_launcher_core::manifest::{self, Manifest, Package, Product, Release};
use utsim_launcher_core::report::Log;
use utsim_launcher_core::shortcuts::{self, Shortcut};
use utsim_launcher_core::state::LauncherState;
use utsim_launcher_core::transport::{self, Transport};
use utsim_launcher_core::{Error, Result, VerifyingKey, drive, signature};

use crate::texto::{fx, sx, tx, vx};

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
            .ok_or_else(|| Error::Install(tx("lib-pasta_do_usuario").into()))?;
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
        return Err(Error::Install(tx("lib-sem_endereco_da_lista").into()));
    }
    let url = drive::direct_url(&url);
    let resp = env.transport.get(&url, None)?;
    if resp.is_html() {
        let page = resp.text(2 << 20)?;
        return Err(Error::Drive(drive::interpret(&page, &url).message().into()));
    }
    if resp.status != 200 {
        if let Some(path) = missing_local_list(&url) {
            return Err(Error::Network(fx(
                "lib-lista_local_nao_encontrada",
                &[("path", sx(path.display().to_string()))],
            )));
        }
        return Err(Error::Network(fx(
            "lib-lista_respondeu_http",
            &[("status", sx(resp.status.to_string()))],
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

/// The file a `file://` list address points at, when that file does not exist (a desktop Drive
/// folder that is not mounted, say). `None` for network addresses and for files that exist.
pub fn missing_local_list(url: &str) -> Option<PathBuf> {
    transport::file_url_path(url.trim()).filter(|p| !p.is_file())
}

/// The address to keep in the state for what was typed: empty or the built-in one means
/// "use the built-in address" (so a later launcher with a new built-in address follows it).
pub fn manifest_override(typed: &str) -> Option<String> {
    let typed = typed.trim();
    (!typed.is_empty() && typed != built_in_manifest_url().trim()).then(|| typed.to_string())
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

/// Downloads (resuming) the encrypted package of `release` for this platform into the
/// downloads folder; returns it with the file. Nothing is checked yet besides the size.
fn download_release<'a>(
    env: &Env,
    state: &LauncherState,
    product: &Product,
    release: &'a Release,
    code: Option<&AccessCode>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(&'a Package, PathBuf)> {
    let pkg = release.package(&env.platform).ok_or_else(|| {
        Error::Install(fx(
            "lib-versao_sem_pacote",
            &[
                ("versao", vx(&release.version)),
                ("sistema", vx(&env.platform)),
            ],
        ))
    })?;
    if !can_install(pkg, code) {
        return Err(Error::NoAccess);
    }
    let url = package_url(env.manifest_url(state), pkg)
        .ok_or_else(|| Error::Install(tx("lib-pacote_sem_link").into()))?;
    let file = env.layout.package_file(&product.id, release, pkg);
    // Before spending the download: room for what is still to come plus the decrypted copy.
    // (The extraction is checked exactly later, from the zip's own index.)
    let have = [download::partial_path(&file), file.clone()]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .max()
        .unwrap_or(0)
        .min(pkg.size);
    install::ensure_space(
        &env.layout.downloads(),
        (pkg.size - have).saturating_add(pkg.size),
    )?;
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
    Ok((pkg, file))
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
    let (pkg, file) = download_release(env, state, product, release, code, cancel, progress)?;
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

/// Version of this launcher.
pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

/// Channel this launcher follows for its own updates: the one chosen for it, else the test
/// channel when the tester follows it in some app, else the stable one.
pub fn launcher_channel(state: &LauncherState) -> &str {
    if let Some(c) = state.channels.get(manifest::LAUNCHER) {
        return c;
    }
    if state.channels.values().any(|c| c == manifest::TESTING) {
        manifest::TESTING
    } else {
        manifest::STABLE
    }
}

/// The newer launcher this one can install, if any. Never for a development launcher (no
/// public key built in), and only when its package opens without a code or with `code`.
pub fn launcher_update<'a>(
    env: &Env,
    state: &LauncherState,
    m: &'a Manifest,
    running: &str,
    code: Option<&AccessCode>,
) -> Option<&'a Release> {
    if env.trusted.is_empty() {
        return None;
    }
    m.launcher_update(launcher_channel(state), &env.platform, running)
        .filter(|r| {
            r.package(&env.platform)
                .is_some_and(|p| can_install(p, code))
        })
}

/// Folder where the launcher's installer waits to be run (in the temporary folder).
pub fn launcher_update_dir() -> PathBuf {
    std::env::temp_dir().join("utsim-launcher-atualizacao")
}

/// Downloads the launcher's installer and checks it like any package (size and SHA-256 from
/// the signed manifest, then authenticated decryption); only then copies it into `dest`
/// (emptied first) and returns its path. A file that fails a check never reaches `dest`.
#[allow(clippy::too_many_arguments)]
pub fn prepare_launcher_update(
    env: &Env,
    state: &LauncherState,
    product: &Product,
    release: &Release,
    code: Option<&AccessCode>,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf> {
    if env.trusted.is_empty() {
        return Err(Error::Install(tx("lib-launcher_de_desenvolvimento").into()));
    }
    if product.id != manifest::LAUNCHER {
        return Err(Error::Install(fx(
            "lib-nao_e_o_launcher",
            &[("id", vx(&product.id))],
        )));
    }
    let name = Path::new(&release.executable)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return Err(Error::Install(fx(
            "lib-instalador_nao_exe",
            &[("arquivo", vx(&release.executable))],
        )));
    }
    let (pkg, file) = download_release(env, state, product, release, code, cancel, progress)?;
    let dir = match install::install(&env.layout, &product.id, release, pkg, code, &file) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    let installer = dest.join(&name);
    let copied = std::fs::copy(dir.join(&release.executable), &installer);
    // The extracted copy is not an app: it never stays in the products folder.
    let _ = std::fs::remove_dir_all(env.layout.product(&product.id));
    copied?;
    env.log.line(&format!(
        "instalador do launcher {} pronto em {}",
        release.version,
        installer.display()
    ));
    Ok(installer)
}

/// Starts the launcher's installer silently and detached (`/S /reabrir`): it waits for this
/// launcher to close, overwrites it and opens the new one. The caller closes the window next.
pub fn run_launcher_installer(installer: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new(installer);
    cmd.args(["/S", "/reabrir"]);
    if let Some(dir) = installer.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| {
        Error::Install(fx(
            "lib-nao_abri_o_instalador",
            &[
                ("arquivo", sx(installer.display().to_string())),
                ("e", sx(e.to_string())),
            ],
        ))
    })?;
    Ok(())
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
        description: fx("lib-atalho_descricao", &[("nome", vx(&product.name))]),
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
            Error::Install(fx(
                "lib-nao_removi",
                &[
                    ("pasta", sx(dir.display().to_string())),
                    ("e", sx(e.to_string())),
                ],
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

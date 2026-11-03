//! The author's side of the launcher. It **only writes local files**: the signing key and
//! the tester list stay in the author's profile folder (never in a repository), and the
//! encrypted packages and the signed manifest go to a folder the author chooses, normally
//! the Google Drive folder synced by Drive for desktop. Nothing is sent to the internet.
//!
//! ```text
//! <dados>/chaves/privada.txt          ed25519 private key (keep a backup offline)
//! <dados>/chaves/publica.txt          public key, built into the launcher
//! <dados>/publicar/testadores.json    testers, their codes and channels
//! <dados>/publicar/chaves_conteudo.json  content key of each package (for re-wrapping)
//! <dados>/publicar/historico/         copy of every manifest written
//! <saida>/manifesto.json              the signed manifest (the file testers' launchers read)
//! <saida>/pacotes/<p>-<v>-<plat>.utpk the encrypted packages
//! ```
//!
//! `<dados>` is `%APPDATA%\UTSim` on Windows (`$XDG_CONFIG_HOME/utsim` or `~/.config/utsim`
//! elsewhere), or `UTSIM_PUBLICAR_DADOS`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::{Argon2Params, ContentKey};
use utsim_launcher_core::manifest::{self, Access, Manifest, Product, Release};
use utsim_launcher_core::{Error, Result, SigningKey, VerifyingKey, drive, package, signature};

/// Name of the manifest file in the publication folder.
pub const MANIFEST_FILE: &str = "manifesto.json";
/// Folder of the packages inside the publication folder.
pub const PACKAGES_DIR: &str = "pacotes";

/// One tester.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tester {
    pub name: String,
    /// Public id (first group of the code).
    pub id: String,
    /// The full code (needed to wrap keys of future versions).
    pub code: String,
    /// Channels this tester may install.
    pub channels: Vec<String>,
    pub active: bool,
    pub created_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

impl Tester {
    pub fn access_code(&self) -> Result<AccessCode> {
        AccessCode::parse(&self.code)
    }
}

/// The author's private data folder.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

/// Default `<dados>` folder.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("UTSIM_PUBLICAR_DADOS") {
        return Some(PathBuf::from(d));
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("UTSim"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("utsim"))
    }
}

/// The git repository that contains `path` (or its nearest existing ancestor), if any.
pub fn enclosing_repo(path: &Path) -> Option<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    abs.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(t) => {
            serde_json::from_str(&t).map_err(|e| Error::Format(format!("{}: {e}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(|e| Error::Format(e.to_string()))?;
    utsim_launcher_core::install::write_atomic(path, &json)
}

impl Store {
    /// Opens `dir`, refusing a folder inside a git repository (where a private key could be
    /// committed by accident) unless `allow_repo`.
    pub fn open(dir: impl Into<PathBuf>, allow_repo: bool) -> Result<Self> {
        let dir = dir.into();
        if !allow_repo {
            if let Some(repo) = enclosing_repo(&dir) {
                return Err(Error::Install(format!(
                    "a pasta de dados {} fica dentro do repositório git {}; a chave privada nunca deve ficar num repositório",
                    dir.display(),
                    repo.display()
                )));
            }
        }
        Ok(Self { dir })
    }

    pub fn private_key_path(&self) -> PathBuf {
        self.dir.join("chaves").join("privada.txt")
    }

    pub fn public_key_path(&self) -> PathBuf {
        self.dir.join("chaves").join("publica.txt")
    }

    fn testers_path(&self) -> PathBuf {
        self.dir.join("publicar").join("testadores.json")
    }

    fn content_keys_path(&self) -> PathBuf {
        self.dir.join("publicar").join("chaves_conteudo.json")
    }

    fn history_dir(&self) -> PathBuf {
        self.dir.join("publicar").join("historico")
    }

    /// Creates the signing key pair; refuses to overwrite an existing one.
    pub fn generate_keys(&self) -> Result<VerifyingKey> {
        let private = self.private_key_path();
        if private.exists() {
            return Err(Error::Install(format!(
                "já existe uma chave privada em {}; apagar ou trocar a chave exige um launcher novo para todos os testadores",
                private.display()
            )));
        }
        let key = signature::generate()?;
        if let Some(parent) = private.parent() {
            fs::create_dir_all(parent)?;
        }
        utsim_launcher_core::install::write_atomic(
            &private,
            signature::private_to_text(&key).as_bytes(),
        )?;
        let public = key.verifying_key();
        fs::write(
            self.public_key_path(),
            format!("{}\n", signature::public_to_text(&public)),
        )?;
        Ok(public)
    }

    pub fn signing_key(&self) -> Result<SigningKey> {
        let path = self.private_key_path();
        let text = fs::read_to_string(&path).map_err(|e| {
            Error::Install(format!(
                "chave privada não encontrada em {} ({e}); rode 'utsim-publicar chaves gerar'",
                path.display()
            ))
        })?;
        signature::private_from_text(&text)
    }

    pub fn testers(&self) -> Result<Vec<Tester>> {
        read_json(&self.testers_path())
    }

    fn save_testers(&self, t: &[Tester]) -> Result<()> {
        write_json(&self.testers_path(), &t)
    }

    /// Registers a tester with a new code.
    pub fn add_tester(&self, name: &str, channels: &[String]) -> Result<Tester> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Format("o nome do testador está vazio".into()));
        }
        let mut list = self.testers()?;
        if list.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
            return Err(Error::Format(format!(
                "já existe um testador chamado '{name}'"
            )));
        }
        for c in channels {
            if !manifest::valid_id(c) {
                return Err(Error::Format(format!("canal inválido '{c}'")));
            }
        }
        let code = loop {
            let c = AccessCode::generate()?;
            if !list.iter().any(|t| t.id == c.id()) {
                break c;
            }
        };
        let tester = Tester {
            name: name.to_string(),
            id: code.id().to_string(),
            code: code.formatted(),
            channels: channels.to_vec(),
            active: true,
            created_at: utsim_launcher_core::state::now_rfc3339(),
            revoked_at: None,
        };
        list.push(tester.clone());
        self.save_testers(&list)?;
        Ok(tester)
    }

    /// Revokes (`active = false`) or reinstates a tester, found by name or id.
    pub fn set_active(&self, who: &str, active: bool) -> Result<Tester> {
        let mut list = self.testers()?;
        let t = list
            .iter_mut()
            .find(|t| {
                t.name.eq_ignore_ascii_case(who.trim()) || t.id.eq_ignore_ascii_case(who.trim())
            })
            .ok_or_else(|| Error::Format(format!("testador '{who}' não encontrado")))?;
        t.active = active;
        t.revoked_at = (!active).then(utsim_launcher_core::state::now_rfc3339);
        let out = t.clone();
        self.save_testers(&list)?;
        Ok(out)
    }

    fn content_keys(&self) -> Result<BTreeMap<String, String>> {
        read_json(&self.content_keys_path())
    }

    fn save_content_key(&self, ctx: &str, key: &ContentKey) -> Result<()> {
        let mut all = self.content_keys()?;
        all.insert(ctx.to_string(), key.to_base64());
        write_json(&self.content_keys_path(), &all)
    }

    fn content_key(&self, ctx: &str) -> Result<ContentKey> {
        let all = self.content_keys()?;
        let b64 = all.get(ctx).ok_or_else(|| {
            Error::Install(format!(
                "a chave de conteúdo de {ctx} não está em {}",
                self.content_keys_path().display()
            ))
        })?;
        ContentKey::from_base64(b64)
    }

    /// Replaces the channels a tester may install (found by name or id).
    pub fn set_channels(&self, who: &str, channels: &[String]) -> Result<Tester> {
        for c in channels {
            if !manifest::valid_id(c) {
                return Err(Error::Format(format!("canal inválido '{c}'")));
            }
        }
        let mut list = self.testers()?;
        let t = list
            .iter_mut()
            .find(|t| {
                t.name.eq_ignore_ascii_case(who.trim()) || t.id.eq_ignore_ascii_case(who.trim())
            })
            .ok_or_else(|| Error::Format(format!("testador '{who}' não encontrado")))?;
        t.channels = channels.to_vec();
        let out = t.clone();
        self.save_testers(&list)?;
        Ok(out)
    }

    fn admin_path(&self) -> PathBuf {
        self.dir.join("publicar").join("admin.json")
    }

    /// The admin app's preferences (missing file: defaults).
    pub fn admin_settings(&self) -> AdminSettings {
        read_json(&self.admin_path()).unwrap_or_default()
    }

    pub fn save_admin_settings(&self, s: &AdminSettings) -> Result<()> {
        write_json(&self.admin_path(), s)
    }

    /// Active testers allowed on `channel`.
    pub fn codes_for(&self, channel: &str) -> Result<Vec<AccessCode>> {
        self.testers()?
            .iter()
            .filter(|t| t.active && t.channels.iter().any(|c| c == channel))
            .map(Tester::access_code)
            .collect()
    }
}

/// Publication folder proposed the first time the admin app opens.
pub const DEFAULT_OUTPUT: &str = r"G:\Meu Drive\UTSim - testes";

/// Preferences of the admin app, in `<dados>/publicar/admin.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminSettings {
    /// Last publication folder used.
    #[serde(default)]
    pub output: Option<PathBuf>,
}

impl AdminSettings {
    /// The folder to use: the last one, else [`DEFAULT_OUTPUT`].
    pub fn output_dir(&self) -> PathBuf {
        self.output
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT))
    }
}

/// The publication folder (normally the synced Google Drive folder).
#[derive(Debug, Clone)]
pub struct Output {
    pub dir: PathBuf,
}

impl Output {
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    /// Reads the current manifest, checking it was signed by our own key; a missing file
    /// gives an empty manifest.
    pub fn load(&self, key: &SigningKey) -> Result<Manifest> {
        match fs::read_to_string(self.manifest_path()) {
            Ok(text) => Manifest::from_signed(&text, &[key.verifying_key()]),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Signs and writes the manifest, keeping a copy in the author's history.
    ///
    /// The file is rewritten **in place** (not replaced by a rename) so Drive for desktop
    /// keeps the same file id, and therefore the same link built into the launcher.
    pub fn save(&self, store: &Store, key: &SigningKey, m: &mut Manifest) -> Result<()> {
        m.sequence += 1;
        m.published_at = utsim_launcher_core::state::now_rfc3339();
        let text = m.to_signed(key)?;
        fs::create_dir_all(store.history_dir())?;
        fs::write(
            store
                .history_dir()
                .join(format!("manifesto-{:05}.json", m.sequence)),
            &text,
        )?;
        fs::create_dir_all(&self.dir)?;
        fs::write(self.manifest_path(), text)?;
        Ok(())
    }
}

/// Where the program comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A folder, zipped by the tool.
    Folder(PathBuf),
    /// A ready .zip.
    Zip(PathBuf),
}

/// Everything `publish` needs.
#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub product: String,
    /// Name shown to testers (only needed for a new product).
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: String,
    pub channel: String,
    pub platform: String,
    pub source: Source,
    pub executable: String,
    pub args: Vec<String>,
    pub notes: String,
    /// Replace a package already published for this version and platform.
    pub replace: bool,
    /// How the product looks in the launcher (fields left `None` keep what is there).
    pub appearance: Appearance,
}

/// How a product looks in the launcher; every field is optional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Appearance {
    /// Accent colour, `#RRGGBB`.
    pub color: Option<String>,
    /// One line under the name.
    pub subtitle: Option<String>,
    /// Who makes it.
    pub author: Option<String>,
    /// A square PNG (see [`read_icon`]).
    pub icon: Option<PathBuf>,
}

/// Smallest and largest side accepted for an icon, pixels.
pub const ICON_SIDES: (u32, u32) = (32, 512);

/// Reads a product icon: a square PNG from 32 to 512 pixels a side (256 x 256 recommended),
/// at most [`manifest::ICON_MAX_BYTES`]. Returns it in base64, as the manifest keeps it.
pub fn read_icon(path: &Path) -> Result<String> {
    use base64::Engine as _;
    let bytes =
        fs::read(path).map_err(|e| Error::Install(format!("ícone {}: {e}", path.display())))?;
    let (w, h) = png_size(&bytes)
        .ok_or_else(|| Error::Format(format!("o ícone {} não é um PNG", path.display())))?;
    if w != h || w < ICON_SIDES.0 || w > ICON_SIDES.1 {
        return Err(Error::Format(format!(
            "o ícone deve ser quadrado, de {} a {} pixels de lado (recomendado 256 x 256); este tem {w} x {h}",
            ICON_SIDES.0, ICON_SIDES.1
        )));
    }
    if bytes.len() > manifest::ICON_MAX_BYTES {
        return Err(Error::Format(format!(
            "o ícone tem {} kB; o limite é {} kB (o manifesto é baixado a cada abertura)",
            bytes.len() / 1024,
            manifest::ICON_MAX_BYTES / 1024
        )));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Width and height from a PNG header.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Some((be(16), be(20)))
}

/// Applies `look` to a product (fields left `None` are kept).
pub fn apply_appearance(p: &mut Product, look: &Appearance) -> Result<()> {
    if let Some(c) = &look.color {
        let c = c.trim();
        if c.is_empty() {
            p.color = None;
        } else {
            let [r, g, b] = manifest::parse_color(c)
                .ok_or_else(|| Error::Format(format!("cor inválida '{c}' (use #RRGGBB)")))?;
            p.color = Some(format!("#{r:02X}{g:02X}{b:02X}"));
        }
    }
    let text = |v: &Option<String>| {
        v.as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    if look.subtitle.is_some() {
        p.subtitle = text(&look.subtitle);
    }
    if look.author.is_some() {
        p.author = text(&look.author);
    }
    if let Some(icon) = &look.icon {
        p.icon = Some(read_icon(icon)?);
    }
    Ok(())
}

/// Result of `publish`.
#[derive(Debug, Clone)]
pub struct Published {
    pub package_file: PathBuf,
    pub size: u64,
    pub wraps: usize,
    pub sequence: u64,
}

/// Encrypts and signs a new package and writes it with the updated manifest.
pub fn publish(
    store: &Store,
    out: &Output,
    req: &PublishRequest,
    params: Argon2Params,
) -> Result<Published> {
    if !manifest::valid_id(&req.product) {
        return Err(Error::Format(format!(
            "id de produto inválido '{}'",
            req.product
        )));
    }
    if !manifest::valid_version(&req.version) {
        return Err(Error::Format(format!("versão inválida '{}'", req.version)));
    }
    if !manifest::valid_id(&req.channel) {
        return Err(Error::Format(format!("canal inválido '{}'", req.channel)));
    }
    if !manifest::relative_path_ok(&req.executable) {
        return Err(Error::Format(format!(
            "o executável deve ser um caminho relativo dentro do pacote: '{}'",
            req.executable
        )));
    }
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    if let Some(p) = m.product(&req.product) {
        if let Some(r) = p.release(&req.version) {
            if r.package(&req.platform).is_some() && !req.replace {
                return Err(Error::Install(format!(
                    "{} {} ({}) já foi publicado; use --substituir para trocar o pacote",
                    req.product, req.version, req.platform
                )));
            }
            if r.channel != req.channel {
                return Err(Error::Install(format!(
                    "{} {} já está no canal '{}'",
                    req.product, req.version, r.channel
                )));
            }
        }
    } else if req.name.is_none() {
        return Err(Error::Format(format!(
            "produto novo '{}': informe --nome (o nome que os testadores veem)",
            req.product
        )));
    }

    let open = m
        .product(&req.product)
        .is_some_and(|p| p.access(&req.channel) == Access::Open);
    let codes = store.codes_for(&req.channel)?;
    if codes.is_empty() && !open {
        return Err(Error::Install(format!(
            "nenhum testador ativo tem acesso ao canal '{}'; cadastre um com 'utsim-publicar testador adicionar'",
            req.channel
        )));
    }

    let scratch = scratch_dir()?;
    let result = (|| {
        let zip = match &req.source {
            Source::Folder(dir) => {
                if !dir.join(&req.executable).is_file() {
                    return Err(Error::Install(format!(
                        "{} não existe em {}",
                        req.executable,
                        dir.display()
                    )));
                }
                let z = scratch.join("conteudo.zip");
                package::zip_dir(dir, &z)?;
                z
            }
            Source::Zip(z) => z.clone(),
        };
        let file_name = format!("{}-{}-{}.utpk", req.product, req.version, req.platform);
        let dest = out.dir.join(PACKAGES_DIR).join(&file_name);
        let sealed = package::seal_file(&zip, &dest)?;
        let ctx =
            utsim_launcher_core::crypto::package_context(&req.product, &req.version, &req.platform);
        store.save_content_key(&ctx, &sealed.key)?;
        let wraps = package::wrap_for_all(
            &sealed.key,
            codes.iter(),
            &req.product,
            &req.version,
            &req.platform,
            params,
        )?;
        let n = wraps.len();
        let mut entry = package::package_entry(&req.platform, "", &sealed, wraps);
        if open {
            entry.open_key = Some(sealed.key.to_base64());
        }
        Ok((entry, n, dest, sealed.size))
    })();
    let _ = fs::remove_dir_all(&scratch);
    let (entry, wraps, dest, size) = result?;

    if m.product(&req.product).is_none() {
        m.products.push(Product {
            id: req.product.clone(),
            name: req.name.clone().unwrap_or_default(),
            description: req.description.clone().unwrap_or_default(),
            ..Product::default()
        });
    }
    let p = m
        .product_mut(&req.product)
        .ok_or_else(|| Error::Install("produto sumiu".into()))?;
    if let Some(n) = &req.name {
        p.name = n.clone();
    }
    if let Some(d) = &req.description {
        p.description = d.clone();
    }
    apply_appearance(p, &req.appearance)?;
    let now = utsim_launcher_core::state::now_rfc3339();
    match p.releases.iter_mut().find(|r| r.version == req.version) {
        Some(r) => {
            r.packages.retain(|x| x.platform != req.platform);
            r.packages.push(entry);
            r.notes = req.notes.clone();
            r.executable = req.executable.clone();
            r.args = req.args.clone();
        }
        None => p.releases.push(Release {
            version: req.version.clone(),
            channel: req.channel.clone(),
            published_at: now,
            notes: req.notes.clone(),
            executable: req.executable.clone(),
            args: req.args.clone(),
            packages: vec![entry],
        }),
    }
    out.save(store, &key, &mut m)?;
    Ok(Published {
        package_file: dest,
        size,
        wraps,
        sequence: m.sequence,
    })
}

/// A scratch folder in the system temp dir (the plaintext zip never goes to the output).
fn scratch_dir() -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("utsim-publicar-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Sets the download address of a package from the Drive sharing link (or any URL).
pub fn set_link(
    store: &Store,
    out: &Output,
    product: &str,
    version: &str,
    platform: &str,
    link: &str,
) -> Result<String> {
    let url = if drive::is_drive_url(link) || !link.contains("://") {
        let id = drive::file_id(link).ok_or_else(|| {
            Error::Format(format!("não encontrei o id do arquivo no link '{link}'"))
        })?;
        drive::download_url(&id)
    } else {
        link.trim().to_string()
    };
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    let pkg = m
        .product_mut(product)
        .and_then(|p| p.releases.iter_mut().find(|r| r.version == version))
        .and_then(|r| r.packages.iter_mut().find(|p| p.platform == platform))
        .ok_or_else(|| {
            Error::Format(format!(
                "{product} {version} ({platform}) não está no manifesto"
            ))
        })?;
    pkg.url = url.clone();
    out.save(store, &key, &mut m)?;
    Ok(url)
}

/// Re-wraps the content keys of published packages for the testers active now (adds new
/// testers, drops revoked ones). `only` limits it to one product and optionally one version.
pub fn rewrap(
    store: &Store,
    out: &Output,
    only: Option<(&str, Option<&str>)>,
    params: Argon2Params,
) -> Result<usize> {
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    let mut count = 0;
    for p in &mut m.products {
        if only.is_some_and(|(id, _)| id != p.id) {
            continue;
        }
        for r in &mut p.releases {
            if only.is_some_and(|(_, v)| v.is_some_and(|v| v != r.version)) {
                continue;
            }
            let codes = store.codes_for(&r.channel)?;
            let open = p.access.get(&r.channel).copied().unwrap_or_default() == Access::Open;
            for pkg in &mut r.packages {
                let ctx =
                    utsim_launcher_core::crypto::package_context(&p.id, &r.version, &pkg.platform);
                let content = store.content_key(&ctx)?;
                pkg.wraps = package::wrap_for_all(
                    &content,
                    codes.iter(),
                    &p.id,
                    &r.version,
                    &pkg.platform,
                    params,
                )?;
                // A channel closed again stops publishing the key from now on (whoever read
                // the manifest while it was open keeps it: see `set_access`).
                pkg.open_key = open.then(|| content.to_base64());
                count += 1;
            }
        }
    }
    out.save(store, &key, &mut m)?;
    Ok(count)
}

/// Sets who may install `product` in `channel` and re-signs the manifest in place; returns how
/// many published packages became open.
///
/// - [`Access::Open`]: the content key of every package already published in that channel
///   goes into the manifest, so any launcher installs it without a code, now; the next
///   publications in the channel are open too.
/// - [`Access::Code`]: the next publications (and `rewrap`) need a code again. The packages
///   published while open **stay openable** by anyone who read the manifest then (the key was
///   public); only a new version is closed for them. The keys already in the manifest stay
///   until the next `rewrap`.
pub fn set_access(
    store: &Store,
    out: &Output,
    product: &str,
    channel: &str,
    access: Access,
) -> Result<usize> {
    if !manifest::valid_id(channel) {
        return Err(Error::Format(format!("canal inválido '{channel}'")));
    }
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    let p = m
        .product_mut(product)
        .ok_or_else(|| Error::Format(format!("produto '{product}' não está no manifesto")))?;
    match access {
        Access::Code => {
            p.access.remove(channel);
        }
        Access::Open => {
            p.access.insert(channel.to_string(), Access::Open);
        }
    }
    let mut opened = 0;
    if access == Access::Open {
        let id = p.id.clone();
        for r in p.releases.iter_mut().filter(|r| r.channel == channel) {
            for pkg in &mut r.packages {
                let ctx =
                    utsim_launcher_core::crypto::package_context(&id, &r.version, &pkg.platform);
                pkg.open_key = Some(store.content_key(&ctx)?.to_base64());
                opened += 1;
            }
        }
    }
    out.save(store, &key, &mut m)?;
    Ok(opened)
}

/// Removes a release from the manifest (the package file is left for the author to delete).
pub fn remove_release(store: &Store, out: &Output, product: &str, version: &str) -> Result<()> {
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    let p = m
        .product_mut(product)
        .ok_or_else(|| Error::Format(format!("produto '{product}' não está no manifesto")))?;
    let before = p.releases.len();
    p.releases.retain(|r| r.version != version);
    if p.releases.len() == before {
        return Err(Error::Format(format!(
            "{product} {version} não está no manifesto"
        )));
    }
    out.save(store, &key, &mut m)
}

/// A readable check of the publication folder: signature, links, local files.
pub fn check(store: &Store, out: &Output) -> Result<(String, usize)> {
    let key = store.signing_key()?;
    let m = out.load(&key)?;
    let testers = store.testers()?;
    let name_of = |id: &str| {
        testers
            .iter()
            .find(|t| t.id == id)
            .map_or_else(|| format!("{id} (desconhecido)"), |t| t.name.clone())
    };
    let mut s = format!(
        "Manifesto {} (sequência {}, assinatura válida)\n",
        out.manifest_path().display(),
        m.sequence
    );
    let mut problems = 0;
    for p in &m.products {
        s.push_str(&format!("\n{} ({})\n", p.name, p.id));
        for (channel, access) in &p.access {
            if *access == Access::Open {
                s.push_str(&format!(
                    "  canal {channel}: ABERTO PARA TODOS (sem código)\n"
                ));
            }
        }
        for r in &p.releases {
            s.push_str(&format!("  {} [{}]\n", r.version, r.channel));
            for pkg in &r.packages {
                let local = out.dir.join(PACKAGES_DIR).join(&pkg.file);
                let state = if !local.is_file() {
                    "arquivo local ausente".to_string()
                } else if utsim_launcher_core::sha256_file(&local)? != pkg.sha256 {
                    problems += 1;
                    "ARQUIVO LOCAL DIFERENTE DO MANIFESTO".to_string()
                } else {
                    "arquivo local confere".to_string()
                };
                let link = if pkg.url.is_empty() {
                    problems += 1;
                    "SEM LINK (rode 'utsim-publicar link')".to_string()
                } else {
                    pkg.url.clone()
                };
                let mut who: Vec<String> = pkg.wraps.iter().map(|w| name_of(&w.id)).collect();
                if pkg.is_open() {
                    who.insert(0, "todos (aberto, sem código)".into());
                }
                s.push_str(&format!(
                    "    {}: {} bytes, {state}\n      link: {link}\n      acesso: {}\n",
                    pkg.platform,
                    pkg.size,
                    if who.is_empty() {
                        "ninguém".to_string()
                    } else {
                        who.join(", ")
                    }
                ));
            }
        }
    }
    Ok((s, problems))
}

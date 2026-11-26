//! Distribution through GitHub releases, run from the author's machine with the `gh` command
//! line (no CI).
//!
//! - Each product has a public repository used only for downloads ([`repo_for`]). A version
//!   is the release `v<version>` of that repository and its encrypted package is an asset of
//!   that release.
//! - The signed manifest is the asset [`MANIFEST_ASSET`] of the fixed release [`MANIFEST_TAG`]
//!   in the launcher's repository ([`manifest_url`], built into the launcher).
//! - Each product keeps the newest [`KEEP`] versions; older ones leave the manifest and their
//!   GitHub release is deleted.
//!
//! Every call to GitHub goes through the [`Gh`] trait, so the flow is tested without network.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use utsim_launcher_core::manifest::{self, TESTING};
use utsim_launcher_core::{Error, Result};

use crate::{MANIFEST_FILE, Output, PACKAGES_DIR, Store};

/// Owner of the download repositories.
pub const OWNER: &str = "JandreiSS";
/// Repository holding the launcher's releases and the manifest.
pub const LAUNCHER_REPO: &str = "utsim-launcher";
/// Fixed release (a pre-release, never "latest") whose asset is the manifest.
pub const MANIFEST_TAG: &str = "manifesto";
/// Name of the manifest asset.
pub const MANIFEST_ASSET: &str = MANIFEST_FILE;
/// Temporary name while a new manifest replaces the old one.
const MANIFEST_NEW: &str = "manifesto-novo.json";
/// Versions kept per product.
pub const KEEP: usize = 3;

/// Download repository of each product (without the owner).
pub fn repo_for(product: &str) -> Option<&'static str> {
    Some(match product {
        "launcher" => LAUNCHER_REPO,
        "utsim" => "utsim-simulador",
        "lean-seis-sigma" => "utsim-lean-seis-sigma",
        "qualidade" => "utsim-qualidade",
        "seguranca" => "utsim-seguranca",
        "estatistica" => "utsim-estatistica",
        "ensino" => "utsim-ensino",
        "produto" => "utsim-produto",
        "ambiental" => "utsim-ambiental",
        _ => return None,
    })
}

/// `owner/name` of a product's repository, or the one given.
pub fn full_repo(product: &str, given: Option<&str>) -> Result<String> {
    match given {
        Some(r) if r.contains('/') => Ok(r.to_string()),
        Some(r) => Ok(format!("{OWNER}/{r}")),
        None => repo_for(product)
            .map(|r| format!("{OWNER}/{r}"))
            .ok_or_else(|| {
                Error::Format(format!(
                    "não sei o repositório de '{product}'; informe --repo"
                ))
            }),
    }
}

/// Tag of a version's release.
pub fn tag(version: &str) -> String {
    format!("v{version}")
}

/// Download address of a release asset.
pub fn asset_url(repo: &str, tag: &str, file: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{file}")
}

/// Address of the manifest, built into the launcher.
pub fn manifest_url() -> String {
    asset_url(
        &format!("{OWNER}/{LAUNCHER_REPO}"),
        MANIFEST_TAG,
        MANIFEST_ASSET,
    )
}

/// Is `url` an asset of a release of `repo`? Returns the tag.
pub fn tag_of_asset<'a>(url: &'a str, repo: &str) -> Option<&'a str> {
    let rest = url.strip_prefix(&format!("https://github.com/{repo}/releases/download/"))?;
    rest.split_once('/').map(|(tag, _)| tag)
}

/// The `gh` command line.
pub trait Gh {
    /// Runs `gh <args>`; the standard output on success, an error with the standard error
    /// otherwise.
    fn run(&self, args: &[&str]) -> Result<String>;
}

/// The real `gh`, already signed in (`gh auth login`).
pub struct GhCli;

impl Gh for GhCli {
    fn run(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("gh").args(args).output().map_err(|e| {
            Error::Network(format!(
                "não consegui rodar o gh ({e}); instale o GitHub CLI e rode 'gh auth login'"
            ))
        })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(Error::Network(format!(
                "gh {}: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

/// What [`upload`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Uploaded {
    pub url: String,
    /// Versions taken out of the manifest to keep [`KEEP`].
    pub pruned: Vec<String>,
    /// Pruned versions whose package was not on GitHub (an old Drive link): delete by hand.
    pub elsewhere: Vec<(String, String)>,
    pub sequence: u64,
}

fn release_exists(gh: &dyn Gh, repo: &str, tag: &str) -> bool {
    gh.run(&["release", "view", tag, "--repo", repo, "--json", "tagName"])
        .is_ok()
}

fn temp_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "utsim-publicar-gh-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Uploads the package of `product` `version` as an asset of the release `v<version>` of
/// `repo` (creating the release), writes its address in the manifest, keeps the newest
/// `keep` versions of the product (deleting the GitHub releases of the others) and sends
/// the manifest ([`send_manifest`]).
#[allow(clippy::too_many_arguments)]
pub fn upload(
    store: &Store,
    out: &Output,
    gh: &dyn Gh,
    product: &str,
    version: &str,
    platform: &str,
    repo: &str,
    keep: usize,
) -> Result<Uploaded> {
    let key = store.signing_key()?;
    let mut m = out.load(&key)?;
    let p = m
        .product(product)
        .ok_or_else(|| Error::Format(format!("produto '{product}' não está no manifesto")))?;
    let name = p.name.clone();
    let r = p
        .release(version)
        .ok_or_else(|| Error::Format(format!("{product} {version} não está no manifesto")))?;
    let pkg = r.package(platform).ok_or_else(|| {
        Error::Format(format!(
            "{product} {version} não tem pacote para {platform}"
        ))
    })?;
    let local = out.dir.join(PACKAGES_DIR).join(&pkg.file);
    if !local.is_file() {
        return Err(Error::Format(format!(
            "pacote {} não encontrado",
            local.display()
        )));
    }
    if utsim_launcher_core::sha256_file(&local)? != pkg.sha256 {
        return Err(Error::Integrity(format!(
            "{} é diferente do que o manifesto anuncia",
            local.display()
        )));
    }
    let file = pkg.file.clone();
    let prerelease = r.channel == TESTING;
    let notes = r.notes.clone();
    let t = tag(version);

    if !release_exists(gh, repo, &t) {
        let tmp = temp_dir()?;
        let notes_file = tmp.join("notas.md");
        fs::write(&notes_file, notes_or_default(&notes))?;
        let title = format!("{name} {version}");
        let nf = notes_file.to_string_lossy().into_owned();
        let mut args = vec![
            "release",
            "create",
            &t,
            "--repo",
            repo,
            "--title",
            &title,
            "--notes-file",
            &nf,
        ];
        if prerelease {
            args.push("--prerelease");
        }
        let res = gh.run(&args);
        let _ = fs::remove_dir_all(&tmp);
        res?;
    }
    let lp = local.to_string_lossy().into_owned();
    gh.run(&["release", "upload", &t, &lp, "--repo", repo, "--clobber"])?;

    let url = asset_url(repo, &t, &file);
    let pkg = m
        .product_mut(product)
        .and_then(|p| p.releases.iter_mut().find(|r| r.version == version))
        .and_then(|r| r.packages.iter_mut().find(|p| p.platform == platform))
        .expect("checked above");
    pkg.url = url.clone();

    // Keep the newest `keep` versions.
    let mut done = Uploaded {
        url,
        ..Default::default()
    };
    let p = m.product_mut(product).expect("checked above");
    let mut versions: Vec<String> = p.releases.iter().map(|r| r.version.clone()).collect();
    versions.sort_by(|a, b| manifest::compare_versions(b, a));
    let old: Vec<String> = versions.into_iter().skip(keep.max(1)).collect();
    let mut delete = Vec::new();
    for v in &old {
        let r = p.release(v).expect("listed above");
        let on_github = r
            .packages
            .iter()
            .any(|k| tag_of_asset(&k.url, repo).is_some());
        if on_github {
            delete.push(tag(v));
        } else {
            let where_ = r
                .packages
                .iter()
                .map(|k| k.url.clone())
                .collect::<Vec<_>>()
                .join(" ");
            done.elsewhere.push((v.clone(), where_));
        }
    }
    p.releases.retain(|r| !old.contains(&r.version));
    done.pruned = old;
    out.save(store, &key, &mut m)?;
    done.sequence = m.sequence;

    send_manifest(gh, out, None)?;
    // Only after the manifest stops listing them.
    for t in delete {
        gh.run(&[
            "release",
            "delete",
            &t,
            "--repo",
            repo,
            "--yes",
            "--cleanup-tag",
        ])?;
    }
    Ok(done)
}

fn notes_or_default(notes: &str) -> String {
    let n = notes.trim();
    if n.is_empty() {
        "Pacote cifrado; instale pelo launcher do UTSim.".into()
    } else {
        format!("{n}\n\n---\nPacote cifrado; instale pelo launcher do UTSim.\n")
    }
}

/// Sends the signed manifest of `out` as the asset [`MANIFEST_ASSET`] of the release
/// [`MANIFEST_TAG`] of `repo` (default: the launcher's). The new file goes up under a
/// temporary name and is renamed after the old one is deleted, so a launcher almost never
/// finds the address empty.
pub fn send_manifest(gh: &dyn Gh, out: &Output, repo: Option<&str>) -> Result<String> {
    let repo = repo
        .map(str::to_string)
        .unwrap_or_else(|| format!("{OWNER}/{LAUNCHER_REPO}"));
    if !release_exists(gh, &repo, MANIFEST_TAG) {
        gh.run(&[
            "release",
            "create",
            MANIFEST_TAG,
            "--repo",
            &repo,
            "--title",
            "Manifesto",
            "--notes",
            "Lista assinada das versões que o launcher do UTSim instala. Não é uma versão do launcher.",
            "--prerelease",
            "--latest=false",
        ])?;
    }
    let tmp = temp_dir()?;
    let staged = tmp.join(MANIFEST_NEW);
    fs::copy(out.manifest_path(), &staged)?;
    let sp = staged.to_string_lossy().into_owned();
    let res = replace_asset(gh, &repo, &sp);
    let _ = fs::remove_dir_all(&tmp);
    res?;
    Ok(asset_url(&repo, MANIFEST_TAG, MANIFEST_ASSET))
}

fn replace_asset(gh: &dyn Gh, repo: &str, staged: &str) -> Result<()> {
    gh.run(&[
        "release",
        "upload",
        MANIFEST_TAG,
        staged,
        "--repo",
        repo,
        "--clobber",
    ])?;
    let ids = gh.run(&[
        "api",
        &format!("repos/{repo}/releases/tags/{MANIFEST_TAG}"),
        "--jq",
        r#".assets[] | "\(.id) \(.name)""#,
    ])?;
    let id_of = |name: &str| {
        ids.lines()
            .filter_map(|l| l.split_once(' '))
            .find(|(_, n)| n.trim() == name)
            .map(|(id, _)| id.trim().to_string())
    };
    let new = id_of(MANIFEST_NEW).ok_or_else(|| {
        Error::Network("o manifesto novo não apareceu na release depois do envio".into())
    })?;
    if let Some(old) = id_of(MANIFEST_ASSET) {
        gh.run(&[
            "api",
            "-X",
            "DELETE",
            &format!("repos/{repo}/releases/assets/{old}"),
        ])?;
    }
    gh.run(&[
        "api",
        "-X",
        "PATCH",
        &format!("repos/{repo}/releases/assets/{new}"),
        "-f",
        &format!("name={MANIFEST_ASSET}"),
    ])?;
    Ok(())
}

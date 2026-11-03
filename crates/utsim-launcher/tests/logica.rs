//! The launcher's logic (no window) over an in-memory "Drive" and over a local folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use utsim_launcher::Env;
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::install::{self, Layout};
use utsim_launcher_core::manifest::{Manifest, Product, Release, STABLE};
use utsim_launcher_core::report::Log;
use utsim_launcher_core::state::LauncherState;
use utsim_launcher_core::transport::{self, Canned, HttpTransport, MemoryTransport, Transport};
use utsim_launcher_core::{Error, SigningKey, package, signature};

const PLATFORM: &str = "windows-x64";
const MANIFEST_URL: &str = "https://drive.usercontent.google.com/download?id=1ManifestoManifestoManifesto0&export=download";

fn env(home: &Path, key: &SigningKey, t: Arc<dyn Transport>, url: &str) -> Env {
    let layout = Layout::new(home);
    Env {
        log: Log::new(&layout.logs(), "launcher.log"),
        layout,
        trusted: vec![key.verifying_key()],
        manifest_url: url.into(),
        transport: t,
        platform: PLATFORM.into(),
        launcher_exe: None,
    }
}

/// A release of `version` for `code`, its package written to `dir/pacotes`.
fn release(dir: &Path, version: &str, code: &AccessCode, url: &str) -> (Release, Vec<u8>) {
    let src = dir.join(format!("src-{version}"));
    fs::create_dir_all(src.join("bin")).unwrap();
    fs::write(src.join("bin/app.exe"), version).unwrap();
    let zip = dir.join(format!("{version}.zip"));
    package::zip_dir(&src, &zip).unwrap();
    let file = dir
        .join("pacotes")
        .join(format!("utsim-{version}-{PLATFORM}.utpk"));
    let sealed = package::seal_file(&zip, &file).unwrap();
    let wraps = package::wrap_for_all(
        &sealed.key,
        [code],
        "utsim",
        version,
        PLATFORM,
        Argon2Params::TEST,
    )
    .unwrap();
    let r = Release {
        version: version.into(),
        channel: STABLE.into(),
        executable: "bin/app.exe".into(),
        packages: vec![package::package_entry(PLATFORM, url, &sealed, wraps)],
        ..Release::default()
    };
    (r, fs::read(file).unwrap())
}

fn manifest(seq: u64, releases: Vec<Release>) -> Manifest {
    let mut m = Manifest::new();
    m.sequence = seq;
    m.products.push(Product {
        id: "utsim".into(),
        name: "UTSim".into(),
        releases,
        ..Product::default()
    });
    m
}

#[test]
fn fetch_install_and_refuse_rollback() {
    let work = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let key = signature::generate().unwrap();
    let code = AccessCode::generate().unwrap();
    let t = Arc::new(MemoryTransport::new());
    let (r1, bytes) = release(work.path(), "0.1.0", &code, "http://drive.local/p1");
    t.add("http://drive.local/p1", Canned::file(bytes));
    let m2 = manifest(2, vec![r1.clone()]);
    t.add(
        MANIFEST_URL,
        Canned::file(m2.to_signed(&key).unwrap().into_bytes()),
    );
    let env = env(home.path(), &key, t.clone(), MANIFEST_URL);
    let mut state = LauncherState::default();

    let m = utsim_launcher::fetch_manifest(&env, &mut state).unwrap();
    assert_eq!(state.last_sequence, 2);
    assert!(utsim_launcher::cached_manifest(&env).is_some());

    let p = m.product("utsim").unwrap();
    let stop = AtomicBool::new(false);
    utsim_launcher::install_release(&env, &state, p, &r1, &code, &stop, &mut |_| {}).unwrap();
    assert_eq!(
        install::active_version(&env.layout, "utsim").as_deref(),
        Some("0.1.0")
    );
    // The downloaded package is removed after installing.
    assert!(
        !env.layout
            .package_file("utsim", &r1, &r1.packages[0])
            .exists()
    );

    // Wrong code: refused before downloading.
    let other = AccessCode::generate().unwrap();
    let r = utsim_launcher::install_release(&env, &state, p, &r1, &other, &stop, &mut |_| {});
    assert!(matches!(r, Err(Error::NoAccess)));

    // An older (validly signed) manifest is a rollback.
    t.add(
        MANIFEST_URL,
        Canned::file(manifest(1, vec![]).to_signed(&key).unwrap().into_bytes()),
    );
    let err = utsim_launcher::fetch_manifest(&env, &mut state).unwrap_err();
    assert!(err.to_string().contains("mais antigo"), "{err}");

    // Drive answering with an HTML page instead of the manifest.
    t.add(
        MANIFEST_URL,
        Canned::html("<html>Too many users have viewed or downloaded this file recently</html>"),
    );
    let err = utsim_launcher::fetch_manifest(&env, &mut state).unwrap_err();
    assert!(matches!(err, Error::Drive(_)), "{err}");

    let report = utsim_launcher::error_report(&env, &state, &work.path().join("relato")).unwrap();
    let mut z = zip::ZipArchive::new(fs::File::open(report).unwrap()).unwrap();
    let mut summary = String::new();
    std::io::Read::read_to_string(&mut z.by_name("resumo.txt").unwrap(), &mut summary).unwrap();
    assert!(summary.contains("utsim: ativa 0.1.0"), "{summary}");
}

#[test]
fn local_publication_folder_without_links() {
    // The author points the launcher at the local folder before uploading anything: packages
    // without a link are found next to the manifest.
    let drive = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let key = signature::generate().unwrap();
    let code = AccessCode::generate().unwrap();
    let (r1, _) = release(drive.path(), "0.1.0", &code, "");
    let manifest_path = drive.path().join("manifesto.json");
    fs::write(
        &manifest_path,
        manifest(1, vec![r1.clone()]).to_signed(&key).unwrap(),
    )
    .unwrap();
    let url = transport::file_url(&manifest_path);
    let env = env(home.path(), &key, Arc::new(HttpTransport::new()), &url);
    let mut state = LauncherState::default();
    let m = utsim_launcher::fetch_manifest(&env, &mut state).unwrap();
    let stop = AtomicBool::new(false);
    utsim_launcher::install_release(
        &env,
        &state,
        m.product("utsim").unwrap(),
        &r1,
        &code,
        &stop,
        &mut |_| {},
    )
    .unwrap();
    let (exe, _, _) = install::launch_command(&env.layout, "utsim").unwrap();
    assert_eq!(fs::read_to_string(exe).unwrap(), "0.1.0");
}

#[test]
fn development_build_has_no_trust() {
    // Built without confianca/chave_publica.txt: no key, every manifest refused.
    if utsim_launcher::built_in_keys().is_empty() {
        let home = tempfile::tempdir().unwrap();
        let key = signature::generate().unwrap();
        let t = Arc::new(MemoryTransport::new());
        t.add(
            MANIFEST_URL,
            Canned::file(manifest(1, vec![]).to_signed(&key).unwrap().into_bytes()),
        );
        let mut e = env(home.path(), &key, t, MANIFEST_URL);
        e.trusted = utsim_launcher::built_in_keys();
        let err = utsim_launcher::fetch_manifest(&e, &mut LauncherState::default()).unwrap_err();
        assert!(err.to_string().contains("chave pública embutida"), "{err}");
    }
}

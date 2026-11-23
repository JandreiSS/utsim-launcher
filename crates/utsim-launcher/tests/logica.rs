//! The launcher's logic (no window) over an in-memory "Drive" and over a local folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use utsim_launcher::Env;
use utsim_launcher::catalogo;
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::install::{self, Layout};
use utsim_launcher_core::manifest::{LAUNCHER, Manifest, Product, Release, STABLE, TESTING};
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
    utsim_launcher::install_release(&env, &state, p, &r1, Some(&code), &stop, &mut |_| {}).unwrap();
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
    let r = utsim_launcher::install_release(&env, &state, p, &r1, Some(&other), &stop, &mut |_| {});
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
        Some(&code),
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

/// The launcher's own release: an installer `.exe` in an open package (no code needed).
fn launcher_release(dir: &Path, version: &str, url: &str) -> (Release, Vec<u8>) {
    let src = dir.join(format!("launcher-{version}"));
    fs::create_dir_all(&src).unwrap();
    let exe = format!("utsim-launcher-v{version}-windows-x64-instalador.exe");
    fs::write(src.join(&exe), format!("instalador {version}")).unwrap();
    let zip = dir.join(format!("launcher-{version}.zip"));
    package::zip_dir(&src, &zip).unwrap();
    let file = dir
        .join("pacotes")
        .join(format!("launcher-{version}-{PLATFORM}.utpk"));
    let sealed = package::seal_file(&zip, &file).unwrap();
    let mut pkg = package::package_entry(PLATFORM, url, &sealed, Vec::new());
    pkg.open_key = Some(sealed.key.to_base64());
    let r = Release {
        version: version.into(),
        channel: STABLE.into(),
        executable: exe,
        packages: vec![pkg],
        ..Release::default()
    };
    (r, fs::read(file).unwrap())
}

#[test]
fn launcher_updates_itself_from_the_signed_manifest() {
    let work = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let key = signature::generate().unwrap();
    let code = AccessCode::generate().unwrap();
    let t = Arc::new(MemoryTransport::new());
    let (app, _) = release(work.path(), "0.1.0", &code, "http://drive.local/app");
    let (lr, bytes) = launcher_release(work.path(), "0.2.0", "http://drive.local/launcher");
    t.add("http://drive.local/launcher", Canned::file(bytes.clone()));
    let mut m = manifest(1, vec![app]);
    m.products.push(Product {
        id: LAUNCHER.into(),
        name: "UTSim Launcher".into(),
        releases: vec![lr.clone()],
        ..Product::default()
    });
    let env = env(home.path(), &key, t.clone(), MANIFEST_URL);
    let state = LauncherState::default();

    // Never an app card.
    let items = catalogo::items(&m, &env.layout, &state, PLATFORM, None, "");
    assert_eq!(
        items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["utsim"]
    );
    assert!(catalogo::items(&m, &env.layout, &state, PLATFORM, None, "launcher").is_empty());

    // Offered only when newer than the running launcher, with or without a code.
    let up = |running: &str| {
        utsim_launcher::launcher_update(&env, &state, &m, running, None).map(|r| r.version.clone())
    };
    assert_eq!(up("0.1.2").as_deref(), Some("0.2.0"));
    assert_eq!(up("0.2.0-dev").as_deref(), Some("0.2.0"));
    assert_eq!(up("0.2.0"), None);
    assert_eq!(up("0.3.0-rc1"), None);

    // Downloaded, checked and copied to the temporary folder; nothing left among the apps.
    let dest = work.path().join("atualizacao");
    let stop = AtomicBool::new(false);
    let p = m.product(LAUNCHER).unwrap();
    let installer = utsim_launcher::prepare_launcher_update(
        &env,
        &state,
        p,
        &lr,
        None,
        &dest,
        &stop,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(installer, dest.join(&lr.executable));
    assert_eq!(fs::read_to_string(&installer).unwrap(), "instalador 0.2.0");
    assert!(!env.layout.product(LAUNCHER).exists());
    assert!(
        !env.layout
            .package_file(LAUNCHER, &lr, &lr.packages[0])
            .exists()
    );

    // A package altered on the way (same size) never reaches the folder it is run from.
    let mut bad = bytes;
    let mid = bad.len() / 2;
    bad[mid] ^= 0xff;
    t.add("http://drive.local/launcher", Canned::file(bad));
    let dest2 = work.path().join("atualizacao2");
    let err = utsim_launcher::prepare_launcher_update(
        &env,
        &state,
        p,
        &lr,
        None,
        &dest2,
        &stop,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, Error::Integrity(_)), "{err}");
    assert!(!dest2.exists());
    assert!(
        !env.layout
            .package_file(LAUNCHER, &lr, &lr.packages[0])
            .exists()
    );

    // A development launcher (no public key) never updates itself.
    let mut dev = env;
    dev.trusted.clear();
    assert!(utsim_launcher::launcher_update(&dev, &state, &m, "0.1.0", None).is_none());
    let dest3 = work.path().join("atualizacao3");
    assert!(
        utsim_launcher::prepare_launcher_update(
            &dev,
            &state,
            p,
            &lr,
            None,
            &dest3,
            &stop,
            &mut |_| {}
        )
        .is_err()
    );
    assert!(!dest3.exists());
}

#[test]
fn launcher_channel_follows_the_tester() {
    let mut s = LauncherState::default();
    assert_eq!(utsim_launcher::launcher_channel(&s), STABLE);
    s.channels.insert("utsim".into(), TESTING.into());
    assert_eq!(utsim_launcher::launcher_channel(&s), TESTING);
    s.channels.insert(LAUNCHER.into(), STABLE.into());
    assert_eq!(utsim_launcher::launcher_channel(&s), STABLE);
}

#[test]
fn missing_local_list_says_where_it_looked() {
    // The list address points at the desktop Drive folder and the drive is not mounted: the
    // message names the file instead of "HTTP 404".
    let drive = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let key = signature::generate().unwrap();
    let path = drive.path().join("Meu Drive").join("manifesto.json");
    let url = transport::file_url(&path);
    assert_eq!(utsim_launcher::missing_local_list(&url), Some(path.clone()));
    assert_eq!(utsim_launcher::missing_local_list(MANIFEST_URL), None);
    let env = env(home.path(), &key, Arc::new(HttpTransport::new()), &url);
    let mut state = LauncherState::default();
    let e = utsim_launcher::fetch_manifest(&env, &mut state)
        .unwrap_err()
        .to_string();
    assert!(e.contains("manifesto.json"), "{e}");
    assert!(e.contains("Drive para computador"), "{e}");
    assert!(!e.contains("404"), "{e}");
    // Once the file is there, nothing is reported missing.
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{}").unwrap();
    assert_eq!(utsim_launcher::missing_local_list(&url), None);
}

#[test]
fn typed_address_equal_to_the_built_in_is_no_override() {
    assert_eq!(utsim_launcher::manifest_override("  "), None);
    assert_eq!(
        utsim_launcher::manifest_override(utsim_launcher::built_in_manifest_url()),
        None
    );
    assert_eq!(
        utsim_launcher::manifest_override(" file:///G:/x.json "),
        Some("file:///G:/x.json".into())
    );
}

//! Access open to everyone (no code) and the product's look in the launcher.

use std::fs;
use std::path::Path;

use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::manifest::{Access, Manifest, TESTING};
use utsim_launcher_core::{Error, install};
use utsim_publicar::{Appearance, Output, PublishRequest, Source, Store};

fn program(dir: &Path) {
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin/app.exe"), b"exe").unwrap();
}

fn request(version: &str, src: &Path) -> PublishRequest {
    PublishRequest {
        product: "utsim".into(),
        name: Some("UTSim".into()),
        description: None,
        version: version.into(),
        channel: TESTING.into(),
        platform: "windows-x64".into(),
        source: Source::Folder(src.to_path_buf()),
        executable: "bin/app.exe".into(),
        args: vec![],
        notes: String::new(),
        replace: false,
        restricted: true,
        appearance: Appearance::default(),
    }
}

fn manifest(store: &Store, out: &Output) -> Manifest {
    let key = store.signing_key().unwrap();
    let text = fs::read_to_string(out.manifest_path()).unwrap();
    Manifest::from_signed(&text, &[key.verifying_key()]).unwrap()
}

/// Installs `version` on a fresh machine with `code` (or none).
fn install_with(store: &Store, out: &Output, version: &str, code: Option<&AccessCode>) -> bool {
    let home = tempfile::tempdir().unwrap();
    let layout = install::Layout::new(home.path());
    let m = manifest(store, out);
    let r = m.product("utsim").unwrap().release(version).unwrap();
    let pkg = r.package("windows-x64").unwrap();
    let file = out.dir.join("pacotes").join(&pkg.file);
    match install::install(&layout, "utsim", r, pkg, code, &file) {
        Ok(_) => true,
        Err(Error::NoAccess) => false,
        Err(e) => panic!("{e}"),
    }
}

fn icon_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icone/familia/utsim_256.png")
}

#[test]
fn open_channel_installs_without_a_code_and_closing_only_affects_new_versions() {
    let data = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    program(src.path());
    let store = Store::open(data.path(), false).unwrap();
    let out = Output {
        dir: drive.path().join("saida"),
    };
    store.generate_keys().unwrap();
    let ana = store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let ana = AccessCode::parse(&ana.code).unwrap();

    // 0.1.0 published with code: no code, no install.
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.1.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert!(!install_with(&store, &out, "0.1.0", None));
    assert!(install_with(&store, &out, "0.1.0", Some(&ana)));

    // Opening the channel opens what is already published, now.
    let opened = utsim_publicar::set_access(&store, &out, "utsim", TESTING, Access::Open).unwrap();
    assert_eq!(opened, 1);
    let m = manifest(&store, &out);
    assert_eq!(m.product("utsim").unwrap().access(TESTING), Access::Open);
    assert!(install_with(&store, &out, "0.1.0", None));
    assert!(
        install_with(&store, &out, "0.1.0", Some(&ana)),
        "o código continua valendo"
    );

    // 0.2.0 published while open: open too.
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.2.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert!(install_with(&store, &out, "0.2.0", None));

    // Closing: the versions published while open stay open (the key was public)...
    utsim_publicar::set_access(&store, &out, "utsim", TESTING, Access::Code).unwrap();
    assert_eq!(
        manifest(&store, &out)
            .product("utsim")
            .unwrap()
            .access(TESTING),
        Access::Code
    );
    assert!(install_with(&store, &out, "0.2.0", None));
    // ...only a new version is closed.
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.3.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert!(!install_with(&store, &out, "0.3.0", None));
    assert!(install_with(&store, &out, "0.3.0", Some(&ana)));

    // Re-wrapping a closed channel takes the old keys out of the manifest.
    utsim_publicar::rewrap(&store, &out, None, Argon2Params::TEST).unwrap();
    let m = manifest(&store, &out);
    assert!(
        m.product("utsim")
            .unwrap()
            .releases
            .iter()
            .all(|r| r.packages.iter().all(|p| !p.is_open()))
    );
    assert!(install_with(&store, &out, "0.2.0", Some(&ana)));

    let (report, problems) = utsim_publicar::check(&store, &out).unwrap();
    assert!(problems > 0, "sem link ainda: {report}");
}

#[test]
fn an_open_channel_needs_no_tester() {
    let data = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    program(src.path());
    let store = Store::open(data.path(), false).unwrap();
    let out = Output {
        dir: drive.path().to_path_buf(),
    };
    store.generate_keys().unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.1.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    utsim_publicar::set_access(&store, &out, "utsim", TESTING, Access::Open).unwrap();
    store.set_active("Ana", false).unwrap();
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.2.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert!(install_with(&store, &out, "0.2.0", None));
    let (report, _) = utsim_publicar::check(&store, &out).unwrap();
    assert!(report.contains("ABERTO PARA TODOS"), "{report}");
}

#[test]
fn appearance_goes_into_the_manifest() {
    let data = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    program(src.path());
    let store = Store::open(data.path(), false).unwrap();
    let out = Output {
        dir: drive.path().to_path_buf(),
    };
    store.generate_keys().unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let mut req = request("0.1.0", src.path());
    req.appearance = Appearance {
        color: Some("22b5ff".into()),
        subtitle: Some("Simulação de processos".into()),
        author: Some("UTSim".into()),
        icon: Some(icon_path()),
    };
    utsim_publicar::publish(&store, &out, &req, Argon2Params::TEST).unwrap();
    let m = manifest(&store, &out);
    let p = m.product("utsim").unwrap();
    assert_eq!(p.color.as_deref(), Some("#22B5FF"));
    assert_eq!(p.subtitle.as_deref(), Some("Simulação de processos"));
    assert_eq!(p.author.as_deref(), Some("UTSim"));
    assert_eq!(p.icon_png().unwrap(), fs::read(icon_path()).unwrap());

    // A later publication without appearance keeps it.
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.2.0", src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    let m = manifest(&store, &out);
    assert_eq!(
        m.product("utsim").unwrap().color.as_deref(),
        Some("#22B5FF")
    );
    assert!(m.product("utsim").unwrap().icon.is_some());

    // Bad input is refused and the manifest is left as it was.
    let mut bad = request("0.3.0", src.path());
    bad.appearance.color = Some("azul".into());
    assert!(utsim_publicar::publish(&store, &out, &bad, Argon2Params::TEST).is_err());
    let not_png = src.path().join("x.png");
    fs::write(&not_png, b"nada").unwrap();
    assert!(utsim_publicar::read_icon(&not_png).is_err());
    assert_eq!(
        utsim_publicar::png_size(&fs::read(icon_path()).unwrap()),
        Some((256, 256))
    );
}

#[test]
fn channels_of_a_tester_can_be_edited_and_admin_settings_are_kept() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path(), false).unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let t = store
        .set_channels("ana", &["estavel".into(), "vr-interno".into()])
        .unwrap();
    assert_eq!(t.channels, vec!["estavel", "vr-interno"]);
    assert!(store.set_channels("Ana", &["Inválido".into()]).is_err());
    assert_eq!(store.codes_for("vr-interno").unwrap().len(), 1);

    let mut s = store.admin_settings();
    assert_eq!(s.output_dir(), Path::new(utsim_publicar::DEFAULT_OUTPUT));
    s.output = Some(data.path().join("drive"));
    store.save_admin_settings(&s).unwrap();
    assert_eq!(
        store.admin_settings().output_dir(),
        data.path().join("drive")
    );
}

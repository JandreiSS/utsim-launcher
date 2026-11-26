//! The author's flow in temporary folders, checked from the tester's side with the core.

use std::fs;
use std::path::Path;

use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::{self, Argon2Params};
use utsim_launcher_core::manifest::{Manifest, STABLE, TESTING};
use utsim_launcher_core::{Error, install};
use utsim_publicar::{Output, PublishRequest, Source, Store};

fn program(dir: &Path) {
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin/utsim-view.exe"), b"exe").unwrap();
}

fn request(version: &str, channel: &str, src: &Path) -> PublishRequest {
    PublishRequest {
        product: "utsim".into(),
        name: Some("UTSim".into()),
        description: None,
        version: version.into(),
        channel: channel.into(),
        platform: "windows-x64".into(),
        source: Source::Folder(src.to_path_buf()),
        executable: "bin/utsim-view.exe".into(),
        args: vec!["--novo".into()],
        notes: format!("Novidades da {version}"),
        replace: false,
        restricted: true,
        appearance: Default::default(),
    }
}

fn manifest(store: &Store, out: &Output) -> Manifest {
    let key = store.signing_key().unwrap();
    let text = fs::read_to_string(out.manifest_path()).unwrap();
    Manifest::from_signed(&text, &[key.verifying_key()]).unwrap()
}

fn opens(m: &Manifest, version: &str, code: &str) -> bool {
    let code = AccessCode::parse(code).unwrap();
    let r = m.product("utsim").unwrap().release(version).unwrap();
    let pkg = r.package("windows-x64").unwrap();
    let ctx = crypto::package_context("utsim", version, "windows-x64");
    match crypto::unwrap_for(&pkg.wraps, &code, &ctx) {
        Ok(_) => true,
        Err(Error::NoAccess) => false,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn keys_testers_publish_link_revoke_rewrap() {
    let data = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    program(src.path());
    let store = Store::open(data.path(), false).unwrap();
    let out = Output {
        dir: drive.path().join("UTSim - testes"),
    };

    store.generate_keys().unwrap();
    assert!(
        store.generate_keys().is_err(),
        "não pode sobrescrever a chave"
    );
    let private = fs::read_to_string(store.private_key_path()).unwrap();
    assert!(private.starts_with("utsim-chave-privada-ed25519"));

    let ana = store
        .add_tester("Ana", &[STABLE.into(), TESTING.into()])
        .unwrap();
    let bia = store.add_tester("Bia", &[STABLE.into()]).unwrap();
    assert!(store.add_tester("ana", &[STABLE.into()]).is_err());

    // 0.1.0 stable: both; 0.2.0 testing: only Ana (Bia is stable-only).
    let p = utsim_publicar::publish(
        &store,
        &out,
        &request("0.1.0", STABLE, src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert_eq!(p.wraps, 2);
    assert_eq!(p.sequence, 1);
    let p = utsim_publicar::publish(
        &store,
        &out,
        &request("0.2.0", TESTING, src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    assert_eq!(p.wraps, 1);
    assert!(
        utsim_publicar::publish(
            &store,
            &out,
            &request("0.2.0", TESTING, src.path()),
            Argon2Params::TEST
        )
        .is_err(),
        "republicar exige --substituir"
    );

    let m = manifest(&store, &out);
    assert!(opens(&m, "0.1.0", &ana.code) && opens(&m, "0.1.0", &bia.code));
    assert!(opens(&m, "0.2.0", &ana.code) && !opens(&m, "0.2.0", &bia.code));
    // The plaintext never reaches the output folder.
    let names: Vec<String> = fs::read_dir(out.dir.join("pacotes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| n.ends_with(".utpk")), "{names:?}");

    // Link from Drive.
    let id = "1AbCdEfGhIjKlMnOpQrStUvWxYz012345";
    let url = utsim_publicar::set_link(
        &store,
        &out,
        "utsim",
        "0.1.0",
        "windows-x64",
        &format!("https://drive.google.com/file/d/{id}/view?usp=sharing"),
    )
    .unwrap();
    assert_eq!(url, utsim_launcher_core::drive::download_url(id));
    let (report, problems) = utsim_publicar::check(&store, &out).unwrap();
    assert_eq!(problems, 1, "0.2.0 ainda sem link:\n{report}");
    assert!(report.contains("Ana, Bia"));

    // Revoke Ana, publish 0.3.0: Ana loses it, keeps 0.1.0 and 0.2.0.
    store.set_active(&ana.id, false).unwrap();
    let carlos = store.add_tester("Carlos", &[TESTING.into()]).unwrap();
    utsim_publicar::publish(
        &store,
        &out,
        &request("0.3.0", TESTING, src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    let m = manifest(&store, &out);
    assert!(!opens(&m, "0.3.0", &ana.code) && opens(&m, "0.3.0", &carlos.code));
    assert!(opens(&m, "0.2.0", &ana.code) && !opens(&m, "0.2.0", &carlos.code));

    // Re-wrap: Carlos gets 0.2.0, Ana loses what she has not downloaded yet.
    let n = utsim_publicar::rewrap(&store, &out, None, Argon2Params::TEST).unwrap();
    assert_eq!(n, 3);
    let m = manifest(&store, &out);
    assert!(opens(&m, "0.2.0", &carlos.code) && !opens(&m, "0.2.0", &ana.code));
    assert!(opens(&m, "0.1.0", &bia.code));
    assert_eq!(m.sequence, 5);
    // Links survive re-wrapping.
    assert_eq!(
        m.product("utsim")
            .unwrap()
            .release("0.1.0")
            .unwrap()
            .packages[0]
            .url,
        url
    );

    // History keeps every manifest.
    let history = fs::read_dir(data.path().join("publicar/historico"))
        .unwrap()
        .count();
    assert_eq!(history, 5);

    utsim_publicar::remove_release(&store, &out, "utsim", "0.2.0").unwrap();
    assert!(
        manifest(&store, &out)
            .product("utsim")
            .unwrap()
            .release("0.2.0")
            .is_none()
    );

    // The tester installs 0.3.0 from the published file.
    let home = tempfile::tempdir().unwrap();
    let layout = install::Layout::new(home.path());
    let m = manifest(&store, &out);
    let r = m.product("utsim").unwrap().release("0.3.0").unwrap();
    let pkg = r.package("windows-x64").unwrap();
    let file = out.dir.join("pacotes").join(&pkg.file);
    let code = AccessCode::parse(&carlos.code).unwrap();
    install::install(&layout, "utsim", r, pkg, Some(&code), &file).unwrap();
    install::activate(&layout, "utsim", "0.3.0").unwrap();
    let (_, args, _) = install::launch_command(&layout, "utsim").unwrap();
    assert_eq!(args, vec!["--novo"]);
}

#[test]
fn data_folder_inside_a_repository_is_refused() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    let err = Store::open(repo.path().join("dados"), false).unwrap_err();
    assert!(err.to_string().contains("repositório"), "{err}");
}

#[test]
fn manifest_signed_by_another_key_is_not_overwritten() {
    let data = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    program(src.path());
    let out = Output {
        dir: drive.path().to_path_buf(),
    };
    let a = Store::open(data.path(), false).unwrap();
    a.generate_keys().unwrap();
    a.add_tester("Ana", &[TESTING.into()]).unwrap();
    utsim_publicar::publish(
        &a,
        &out,
        &request("0.1.0", TESTING, src.path()),
        Argon2Params::TEST,
    )
    .unwrap();
    let b = Store::open(other.path(), false).unwrap();
    b.generate_keys().unwrap();
    b.add_tester("Bia", &[TESTING.into()]).unwrap();
    let r = utsim_publicar::publish(
        &b,
        &out,
        &request("0.2.0", TESTING, src.path()),
        Argon2Params::TEST,
    );
    assert!(matches!(r, Err(Error::Signature(_))), "{r:?}");
}

#[test]
fn long_release_notes_are_trimmed_at_a_line() {
    let short = "- uma linha\n";
    assert_eq!(utsim_publicar::trim_notes(short), short);
    let long = "- Launcher 0.3.3: espaço em disco conferido, ação é feita\n".repeat(2000);
    let cut = utsim_publicar::trim_notes(&long);
    assert!(
        cut.len() <= utsim_publicar::NOTES_LIMIT + 200,
        "{}",
        cut.len()
    );
    assert!(cut.contains("notas resumidas"));
    // Whole lines only.
    let body = cut.split("\n\n(notas").next().unwrap();
    assert!(body.lines().all(|l| l.ends_with("feita")));
}

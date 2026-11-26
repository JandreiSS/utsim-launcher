//! The admin window drawn headless, and its actions through `utsim-publicar`. With
//! `UTSIM_CAPTURAS=<pasta>` each screen is also painted into a PNG there.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{Context, vec2};
use utsim_admin::{AccessChange, Admin, Tela};
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::manifest::{Access, TESTING};
use utsim_publicar::{Appearance, Output, PublishRequest, Source, Store};
use utsim_ui_tema::captura::Captura;
use utsim_ui_tema::familia;

fn draw(admin: &mut Admin, name: &str) {
    let ctx = Context::default();
    familia::install(&ctx);
    let mut cap = Captura::new(ctx, vec2(1200.0, 760.0));
    match std::env::var_os("UTSIM_CAPTURAS").map(PathBuf::from) {
        Some(dir) => cap
            .imagem(12, |ui| admin.draw(ui))
            .salvar_png(&dir.join("admin").join(format!("{name}.png")))
            .unwrap(),
        None => {
            for _ in 0..3 {
                let _ = cap.quadro(Vec::new(), |ui| admin.draw(ui));
            }
        }
    }
}

fn publish(store: &Store, out: &Output, src: &Path, version: &str) {
    fs::create_dir_all(src.join("bin")).unwrap();
    fs::write(src.join("bin/app.exe"), b"exe").unwrap();
    let req = PublishRequest {
        product: "lean-seis-sigma".into(),
        name: Some("UTSim LSS".into()),
        description: Some("Projetos Lean Seis Sigma".into()),
        version: version.into(),
        channel: TESTING.into(),
        platform: "windows-x64".into(),
        source: Source::Folder(src.to_path_buf()),
        executable: "bin/app.exe".into(),
        args: vec![],
        notes: String::new(),
        replace: false,
        restricted: true,
        appearance: Appearance {
            color: Some("#4FD06A".into()),
            ..Appearance::default()
        },
    };
    utsim_publicar::publish(store, out, &req, Argon2Params::TEST).unwrap();
}

#[test]
fn without_the_private_key_it_only_explains() {
    let data = tempfile::tempdir().unwrap();
    let mut admin = Admin::open(data.path().to_path_buf());
    let why = admin.store.as_ref().err().cloned().unwrap();
    assert!(why.contains("utsim-publicar chaves gerar"), "{why}");
    draw(&mut admin, "sem_chave");
}

#[test]
fn every_screen_draws_and_the_actions_work() {
    let data = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let store = Store::open(data.path(), false).unwrap();
    store.generate_keys().unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let out = Output {
        dir: drive.path().join("UTSim - testes"),
    };
    publish(&store, &out, src.path(), "0.1.0");

    let mut admin = Admin::open_with(data.path().to_path_buf(), Argon2Params::TEST);
    assert!(admin.store.is_ok());
    // First time: the default Drive folder; then the one chosen, remembered.
    assert_eq!(admin.output, utsim_publicar::DEFAULT_OUTPUT);
    admin.output = out.dir.display().to_string();
    admin.use_output();
    assert!(admin.manifest.is_some(), "{:?}", admin.manifest_error);
    let again = Admin::open_with(data.path().to_path_buf(), Argon2Params::TEST);
    assert_eq!(again.output, out.dir.display().to_string());

    // Testers: add (code shown once), channels, revoke.
    admin.new_name = "Bruno".into();
    admin.add_tester();
    let bruno = admin.shown_code.clone().expect("código mostrado");
    assert_eq!(bruno.name, "Bruno");
    assert_eq!(admin.testers.len(), 2);
    draw(&mut admin, "testadores_codigo");
    admin.shown_code = None;
    admin.save_channels(&bruno.id, "estavel, testes, vr-interno");
    assert_eq!(
        admin.testers[1].channels,
        vec!["estavel", "testes", "vr-interno"]
    );
    admin.set_active("Ana", false);
    assert!(!admin.testers[0].active);
    admin.editing = Some((bruno.id.clone(), "estavel".into()));
    draw(&mut admin, "testadores");
    admin.editing = None;

    // Publications: check and re-wrap (Bruno gets the published version, Ana loses it).
    admin.tela = Tela::Publicacoes;
    admin.run_check();
    assert!(admin.check.as_ref().unwrap().1 > 0, "sem link ainda");
    let ctx = Context::default();
    admin.start_rewrap(&ctx);
    admin.wait();
    assert!(!admin.status_is_error, "{}", admin.status);
    let wraps: Vec<String> = admin.manifest.as_ref().unwrap().products[0].releases[0].packages[0]
        .wraps
        .iter()
        .map(|w| w.id.clone())
        .collect();
    assert_eq!(wraps, vec![bruno.id.clone()]);
    draw(&mut admin, "publicacoes");

    // Access: open to everyone, confirmed.
    admin.tela = Tela::Acesso;
    let change = AccessChange {
        product: "lean-seis-sigma".into(),
        channel: TESTING.into(),
        access: Access::Open,
    };
    admin.confirm = Some(change.clone());
    draw(&mut admin, "acesso_confirmar");
    admin.confirm = None;
    admin.apply_access(&change);
    let m = admin.manifest.clone().unwrap();
    assert_eq!(m.products[0].access(TESTING), Access::Open);
    assert!(m.products[0].releases[0].packages[0].is_open());
    draw(&mut admin, "acesso");
    admin.confirm = Some(AccessChange {
        access: Access::Code,
        ..change
    });
    draw(&mut admin, "acesso_fechar");

    // Keys: the public key only.
    admin.confirm = None;
    admin.tela = Tela::Chaves;
    draw(&mut admin, "chaves");
}

#[test]
fn a_folder_without_manifest_is_explained() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path(), false).unwrap();
    store.generate_keys().unwrap();
    let mut admin = Admin::open_with(data.path().to_path_buf(), Argon2Params::TEST);
    admin.output = data.path().join("vazia").display().to_string();
    admin.use_output();
    assert!(
        admin
            .manifest_error
            .as_deref()
            .unwrap()
            .contains("Nenhum manifesto")
    );
    for t in Tela::ALL {
        admin.tela = t;
        draw(&mut admin, &format!("vazia_{}", t.label()));
    }
}

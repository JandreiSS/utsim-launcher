//! The launcher in the five languages: every screen draws in each one, no key is missing,
//! dates and plurals follow the language, and the saved language comes back on opening.

use std::path::Path;
use std::sync::Arc;

use eframe::egui::{self, Context};
use utsim_idiomas::Idioma;
use utsim_launcher::Env;
use utsim_launcher::catalogo;
use utsim_launcher::janela::{self, App};
use utsim_launcher::texto::{TEXTOS, fx, tx, vx};
use utsim_launcher_core::install::Layout;
use utsim_launcher_core::manifest::{Access, Manifest, Package, Product, Release, STABLE};
use utsim_launcher_core::report::Log;
use utsim_launcher_core::state::{LauncherState, ViewMode};
use utsim_launcher_core::transport::MemoryTransport;
use utsim_ui_tema::captura::Captura;
use utsim_ui_tema::familia;

const PLATFORM: &str = "windows-x64";

fn env(home: &Path) -> Arc<Env> {
    let layout = Layout::new(home);
    Arc::new(Env {
        log: Log::new(&layout.logs(), "launcher.log"),
        layout,
        trusted: vec![],
        manifest_url: "https://drive.usercontent.google.com/download?id=x&export=download".into(),
        transport: Arc::new(MemoryTransport::new()),
        platform: PLATFORM.into(),
        launcher_exe: None,
    })
}

fn manifest() -> Manifest {
    let mut m = Manifest::new();
    m.sequence = 1;
    for (id, name, date) in [
        ("lean-seis-sigma", "UTSim Seis Sigma", "2026-11-01"),
        ("ensino", "UTSim Ensino", "2026-10-30"),
    ] {
        let mut p = Product {
            id: id.into(),
            name: name.into(),
            releases: vec![Release {
                version: "0.1.0".into(),
                channel: STABLE.into(),
                published_at: format!("{date}T12:00:00Z"),
                executable: "bin/app.exe".into(),
                packages: vec![Package {
                    platform: PLATFORM.into(),
                    size: 1,
                    open_key: Some("AAAA".into()),
                    ..Package::default()
                }],
                ..Release::default()
            }],
            ..Product::default()
        };
        p.access.insert(STABLE.into(), Access::Open);
        m.products.push(p);
    }
    m
}

fn draw(app: &mut App) {
    let ctx = Context::default();
    familia::install(&ctx);
    let mut cap = Captura::new(ctx, egui::vec2(1180.0, 760.0));
    for _ in 0..3 {
        let _ = cap.quadro(Vec::new(), |ui| app.draw(ui));
    }
}

#[test]
fn the_launcher_in_five_languages() {
    assert!(TEXTOS.faltas().is_empty(), "{:?}", TEXTOS.faltas());
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let mut seen = Vec::new();
    for i in Idioma::TODOS {
        utsim_idiomas::definir(i);
        seen.push(tx("janela-configuracoes"));
        for view in [ViewMode::Grid, ViewMode::List] {
            let state = LauncherState {
                view,
                ..LauncherState::default()
            };
            let mut app = App::sem_rede(Arc::clone(&env), state, Some(manifest()));
            draw(&mut app);
            app.settings_open = true;
            draw(&mut app);
            app.settings_open = false;
            app.notes = Some("ensino".into());
            draw(&mut app);
            app.notes = None;
            app.confirm_uninstall = Some("ensino".into());
            draw(&mut app);
            app.confirm_uninstall = None;
            app.editing_code = true;
            draw(&mut app);
        }
        assert!(catalogo::date_text("2026-11-02T12:00:00Z").is_some());
    }
    seen.dedup();
    assert_eq!(seen.len(), Idioma::TODOS.len(), "{seen:?}");

    utsim_idiomas::definir(Idioma::En);
    assert_eq!(
        catalogo::date_text("2026-11-02T12:00:00Z").as_deref(),
        Some("Nov 2, 2026")
    );
    assert_eq!(
        fx("janela-instalados", &[("n", vx(&1))]),
        "1 installed".to_string()
    );
    utsim_idiomas::definir(Idioma::PtBr);
    assert_eq!(
        catalogo::date_text("2026-11-02T12:00:00Z").as_deref(),
        Some("2 de nov. de 2026")
    );
    assert_eq!(fx("janela-instalados", &[("n", vx(&2))]), "2 instalados");

    // The saved language comes back when the window opens.
    let state = LauncherState {
        language: Some("it".into()),
        ..LauncherState::default()
    };
    janela::apply_language(&state);
    assert_eq!(utsim_idiomas::atual(), Idioma::It);
    utsim_idiomas::definir(Idioma::PtBr);
}

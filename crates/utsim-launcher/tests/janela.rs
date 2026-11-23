//! The launcher window drawn headless (no screen): both views with 0, 1 and many products
//! in every state, the code screen, a download in progress, the notes and the opening
//! animation. With `UTSIM_CAPTURAS=<pasta>` each case is also painted into a PNG there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Context, vec2};
use utsim_launcher::Env;
use utsim_launcher::abertura::{self, Intro};
use utsim_launcher::catalogo::{self, Status};
use utsim_launcher::janela::App;
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::crypto::{Argon2Params, WrappedKey};
use utsim_launcher_core::download::Progress;
use utsim_launcher_core::install::Layout;
use utsim_launcher_core::manifest::{Access, Manifest, Package, Product, Release, STABLE, TESTING};
use utsim_launcher_core::report::Log;
use utsim_launcher_core::state::{LauncherState, SortOrder, Theme, ViewMode};
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

fn code() -> AccessCode {
    AccessCode::generate().unwrap()
}

fn package(code: Option<&AccessCode>, open: bool) -> Package {
    Package {
        platform: PLATFORM.into(),
        size: 1,
        sha256: "00".into(),
        cipher: "xchacha20poly1305-stream-v1".into(),
        wraps: code
            .map(|c| {
                vec![WrappedKey {
                    id: c.id().into(),
                    salt: String::new(),
                    nonce: String::new(),
                    key: String::new(),
                    argon2: Argon2Params::TEST,
                }]
            })
            .unwrap_or_default(),
        open_key: open.then(|| "AAAA".into()),
        ..Package::default()
    }
}

fn release(version: &str, date: &str, pkg: Option<Package>) -> Release {
    Release {
        version: version.into(),
        channel: STABLE.into(),
        published_at: format!("{date}T12:00:00Z"),
        notes: format!("Novidades da {version}: correções e melhorias."),
        executable: "bin/app.exe".into(),
        packages: pkg.into_iter().collect(),
        ..Release::default()
    }
}

fn product(id: &str, name: &str, description: &str, releases: Vec<Release>) -> Product {
    Product {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        releases,
        ..Product::default()
    }
}

/// Marks `version` of `id` installed and active (no real program needed to draw).
fn installed(layout: &Layout, id: &str, version: &str) {
    let dir = layout.version(id, version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".utsim-versao.json"),
        format!(
            r#"{{"product": "{id}", "version": "{version}", "channel": "{STABLE}", "executable": "bin/app.exe"}}"#
        ),
    )
    .unwrap();
    utsim_launcher_core::install::activate(layout, id, version).unwrap();
}

/// A manifest with every state: update, installed, not installed (open to everyone), no
/// access, no package; one product without any look in the manifest (monogram icon).
fn many(layout: &Layout, code: &AccessCode) -> Manifest {
    let mut m = Manifest::new();
    m.sequence = 1;
    m.products.push(product(
        "utsim",
        "UTSim",
        "Editor de modelos de simulação a eventos discretos.",
        vec![
            release("0.1.0", "2026-10-20", Some(package(Some(code), false))),
            release(
                "0.2.0-dev.1",
                "2026-11-02",
                Some(package(Some(code), false)),
            ),
        ],
    ));
    installed(layout, "utsim", "0.1.0");
    m.products.push(product(
        "lean-seis-sigma",
        "UTSim Seis Sigma",
        "Gerenciador de projetos Lean Seis Sigma da família UTSim.",
        vec![release(
            "0.1.0",
            "2026-11-01",
            Some(package(Some(code), false)),
        )],
    ));
    installed(layout, "lean-seis-sigma", "0.1.0");
    let mut ensino = product(
        "ensino",
        "UTSim Ensino",
        "Aulas de engenharia de produção com modelos prontos para simular.",
        vec![release("0.1.0", "2026-10-30", Some(package(None, true)))],
    );
    ensino.access.insert(STABLE.into(), Access::Open);
    m.products.push(ensino);
    m.products.push(product(
        "utsim-3d",
        "UTSim 3D",
        "Prévia 3D do modelo, com câmera livre.",
        vec![release("0.1.0", "2026-10-28", Some(package(None, false)))],
    ));
    let mut other = product(
        "ferramenta-nova",
        "Ferramenta Nova",
        "Um produto publicado depois deste launcher, com a cor e o subtítulo do manifesto.",
        vec![release("0.0.1", "2026-10-01", None)],
    );
    other.color = Some("#C084FC".into());
    other.subtitle = Some("Sem versão para este sistema".into());
    other.author = Some("Convidado".into());
    m.products.push(other);
    m
}

fn one() -> Manifest {
    let mut m = Manifest::new();
    m.sequence = 1;
    m.products.push(product(
        "lean-seis-sigma",
        "UTSim Seis Sigma",
        "Gerenciador de projetos Lean Seis Sigma da família UTSim.",
        vec![release("0.1.0", "2026-11-01", Some(package(None, true)))],
    ));
    m
}

fn context() -> Context {
    let ctx = Context::default();
    familia::install(&ctx);
    ctx
}

fn out_dir() -> Option<PathBuf> {
    std::env::var_os("UTSIM_CAPTURAS").map(PathBuf::from)
}

/// Draws `app` for a few frames; saves a PNG when `UTSIM_CAPTURAS` is set.
fn draw(app: &mut App, size: egui::Vec2, name: &str) {
    let mut cap = Captura::new(context(), size);
    match out_dir() {
        Some(dir) => {
            let img = cap.imagem(12, |ui| app.draw(ui));
            img.salvar_png(&dir.join(format!("{name}.png"))).unwrap();
        }
        None => {
            for _ in 0..3 {
                let _ = cap.quadro(Vec::new(), |ui| app.draw(ui));
            }
        }
    }
}

fn state(view: ViewMode, code: Option<&AccessCode>) -> LauncherState {
    LauncherState {
        code: code.map(AccessCode::formatted),
        view,
        checked_at: utsim_launcher_core::state::now_secs() - 600,
        ..LauncherState::default()
    }
}

#[test]
fn both_views_draw_with_zero_one_and_many_products() {
    let c = code();
    for view in [ViewMode::Grid, ViewMode::List] {
        let tag = if view == ViewMode::Grid {
            "grade"
        } else {
            "lista"
        };
        for (n, wide) in [(0usize, true), (1, true), (5, true), (5, false)] {
            let home = tempfile::tempdir().unwrap();
            let env = env(home.path());
            let m = match n {
                0 => {
                    let mut m = Manifest::new();
                    m.sequence = 1;
                    m
                }
                1 => one(),
                _ => many(&env.layout, &c),
            };
            let mut app = App::sem_rede(env, state(view, Some(&c)), Some(m));
            let size = if wide {
                vec2(1280.0, 820.0)
            } else {
                vec2(700.0, 560.0)
            };
            let w = if wide { "larga" } else { "estreita" };
            draw(&mut app, size, &format!("{tag}_{n}_{w}"));
        }
    }
}

#[test]
fn every_state_is_computed() {
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let c = code();
    let m = many(&env.layout, &c);
    let st = state(ViewMode::Grid, Some(&c));
    let items = catalogo::items(&m, &env.layout, &st, PLATFORM, Some(&c), "");
    let status = |id: &str| items.iter().find(|i| i.id == id).unwrap().status.clone();
    assert_eq!(
        status("utsim"),
        Status::Update {
            from: "0.1.0".into(),
            to: "0.2.0-dev.1".into()
        }
    );
    assert_eq!(
        status("lean-seis-sigma"),
        Status::Installed {
            version: "0.1.0".into()
        }
    );
    assert_eq!(status("ensino"), Status::NotInstalled);
    assert_eq!(status("utsim-3d"), Status::NoAccess);
    assert_eq!(status("ferramenta-nova"), Status::NoPackage);
    let ensino = items.iter().find(|i| i.id == "ensino").unwrap();
    assert!(ensino.open);
    assert_eq!(ensino.name_parts, ("UTSim".into(), "Ensino".into()));
    assert!(ensino.icon.is_some(), "ícone embutido");
    let other = items.iter().find(|i| i.id == "ferramenta-nova").unwrap();
    assert_eq!(other.color, [0xC0, 0x84, 0xFC]);
    assert_eq!(other.author, "Convidado");
    assert!(other.icon.is_none(), "sem ícone: monograma");
    assert!(catalogo::needs_code(&items));

    // Without a code only what is open can be installed.
    let items = catalogo::items(
        &m,
        &env.layout,
        &state(ViewMode::Grid, None),
        PLATFORM,
        None,
        "",
    );
    let status = |id: &str| items.iter().find(|i| i.id == id).unwrap().status.clone();
    assert_eq!(status("ensino"), Status::NotInstalled);
    assert_eq!(
        status("utsim"),
        Status::Installed {
            version: "0.1.0".into()
        }
    );

    // Search and order.
    let found = catalogo::items(&m, &env.layout, &st, PLATFORM, Some(&c), "sigma");
    assert_eq!(found.len(), 1);
    let mut st2 = st.clone();
    st2.sort = SortOrder::Updates;
    assert_eq!(
        catalogo::items(&m, &env.layout, &st2, PLATFORM, Some(&c), "")[0].id,
        "utsim"
    );
    st2.sort = SortOrder::Recent;
    assert_eq!(
        catalogo::items(&m, &env.layout, &st2, PLATFORM, Some(&c), "")[0].id,
        "utsim"
    );
    st2.sort = SortOrder::Name;
    assert_eq!(
        catalogo::items(&m, &env.layout, &st2, PLATFORM, Some(&c), "")[0].id,
        "ferramenta-nova"
    );
}

#[test]
fn code_screen_download_notes_and_no_manifest_draw() {
    let c = code();
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let m = many(&env.layout, &c);

    let mut app = App::sem_rede(Arc::clone(&env), state(ViewMode::Grid, None), None);
    app.editing_code = true;
    app.code_error = Some("o código tem 20 caracteres; faltam 3".into());
    draw(&mut app, vec2(1100.0, 720.0), "codigo");

    let mut app = App::sem_rede(Arc::clone(&env), state(ViewMode::Grid, None), None);
    draw(&mut app, vec2(1100.0, 720.0), "sem_lista");

    for view in [ViewMode::Grid, ViewMode::List] {
        let mut app = App::sem_rede(Arc::clone(&env), state(view, Some(&c)), Some(m.clone()));
        app.show_task(
            "Instalando UTSim 0.2.0-dev.1",
            Some("utsim"),
            Some(Progress {
                done: 45 << 20,
                total: Some(120 << 20),
            }),
        );
        let tag = if view == ViewMode::Grid {
            "grade"
        } else {
            "lista"
        };
        draw(&mut app, vec2(1280.0, 820.0), &format!("baixando_{tag}"));
        assert!(
            app.check_line().contains("VIA DRIVE"),
            "{}",
            app.check_line()
        );
    }

    let mut app = App::sem_rede(
        Arc::clone(&env),
        state(ViewMode::Grid, Some(&c)),
        Some(m.clone()),
    );
    app.notes = Some("utsim".into());
    draw(&mut app, vec2(1280.0, 820.0), "notas");

    let mut app = App::sem_rede(
        Arc::clone(&env),
        state(ViewMode::Grid, Some(&c)),
        Some(m.clone()),
    );
    app.confirm_uninstall = Some("lean-seis-sigma".into());
    draw(&mut app, vec2(1280.0, 820.0), "desinstalar");

    let mut app = App::sem_rede(
        Arc::clone(&env),
        state(ViewMode::Grid, Some(&c)),
        Some(m.clone()),
    );
    app.settings_open = true;
    draw(&mut app, vec2(1280.0, 820.0), "configuracoes");
    assert!(app.settings_open);

    let mut app = App::sem_rede(env, state(ViewMode::Grid, Some(&c)), Some(m));
    app.set_status(
        "a lista de versões respondeu HTTP 404. Mostrando a última lista baixada.",
        true,
    );
    app.offline = true;
    assert!(app.check_line().starts_with("LISTA GUARDADA"));
    draw(&mut app, vec2(1280.0, 820.0), "erro");
}

#[test]
fn opening_animation_draws_and_ends_in_the_window() {
    let c = code();
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let m = many(&env.layout, &c);
    for (k, t) in [0.3f32, 0.8, 1.2, 1.6, 2.1, 2.5, 2.8, 3.1, 3.4]
        .iter()
        .enumerate()
    {
        let mut app = App::sem_rede(
            Arc::clone(&env),
            state(ViewMode::Grid, Some(&c)),
            Some(m.clone()),
        );
        let mut cap = Captura::new(context(), vec2(1280.0, 820.0));
        // The animation started `t` seconds before the frames drawn (the capture's clock
        // starts at 0 and moves 1/60 s per frame).
        app.intro = Some(Intro::new(-(*t as f64)));
        match out_dir() {
            Some(dir) => {
                let img = cap.imagem(2, |ui| app.draw(ui));
                img.salvar_png(
                    &dir.join("abertura")
                        .join(format!("abertura_{k}_{t:.1}s.png")),
                )
                .unwrap();
            }
            None => {
                let _ = cap.quadro(Vec::new(), |ui| app.draw(ui));
            }
        }
        let over = *t + 2.0 / 60.0 >= abertura::DURACAO;
        assert_eq!(app.intro.is_none(), over, "em {t}s");
    }

    // A key press skips it.
    let mut app = App::sem_rede(env, state(ViewMode::Grid, Some(&c)), Some(m));
    app.intro = Some(Intro::new(0.0));
    let mut cap = Captura::new(context(), vec2(900.0, 600.0));
    let _ = cap.quadro(Vec::new(), |ui| app.draw(ui));
    assert!(app.intro.is_some());
    let _ = cap.quadro(
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        |ui| app.draw(ui),
    );
    assert!(app.intro.is_none());
}

#[test]
fn test_manifests_keep_the_channel_of_testing() {
    // The many-products fixture follows the stable channel; a tester on "testes" sees the
    // same products (the stable versions are always visible).
    let c = code();
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let m = many(&env.layout, &c);
    let mut st = state(ViewMode::Grid, Some(&c));
    st.channels.insert("utsim".into(), TESTING.into());
    let items = catalogo::items(&m, &env.layout, &st, PLATFORM, Some(&c), "");
    assert_eq!(items.len(), 5);
}

/// A product with 22 test versions, 4 of them installed: the "…" menu lists the 5 newest,
/// puts the rest under "Mais versões" and keeps "Remover versões antigas" near the top.
#[test]
fn the_product_menu_stays_short_with_many_versions() {
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let mut m = Manifest::new();
    m.sequence = 1;
    let releases = (1..=22)
        .map(|k| {
            let mut r = release(
                &format!("0.2.0-dev.{k}"),
                &format!("2026-10-{:02}", k + 1),
                Some(package(None, true)),
            );
            r.channel = TESTING.into();
            r
        })
        .collect();
    m.products.push(product(
        "utsim",
        "UTSim",
        "Editor de modelos de simulação a eventos discretos.",
        releases,
    ));
    for k in [19, 20, 21, 22] {
        installed(&env.layout, "utsim", &format!("0.2.0-dev.{k}"));
    }
    for (theme, tag) in [(Theme::Dark, "escuro"), (Theme::Light, "claro")] {
        let mut st = state(ViewMode::Grid, None);
        st.channels.insert("utsim".into(), TESTING.into());
        st.theme = theme;
        let mut app = App::sem_rede(Arc::clone(&env), st, Some(m.clone()));
        let mut cap = Captura::new(context(), vec2(340.0, 640.0));
        let mut texts = String::new();
        let mut paint = |ui: &mut egui::Ui| {
            app.draw(ui);
            egui::Area::new(egui::Id::new("menu"))
                .fixed_pos(egui::pos2(10.0, 10.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        app.product_menu_of(ui, "utsim");
                    });
                });
        };
        match out_dir() {
            Some(dir) => {
                let img = cap.imagem(12, &mut paint);
                img.salvar_png(&dir.join(format!("menu_versoes_{tag}.png")))
                    .unwrap();
            }
            None => {
                for _ in 0..3 {
                    let _ = cap.quadro(Vec::new(), &mut paint);
                }
            }
        }
        // The texts of the menu, from the shapes of a plain context.
        let plain = context();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(340.0, 900.0),
            )),
            ..Default::default()
        };
        let mut menu = |ui: &mut egui::Ui| {
            egui::Area::new(egui::Id::new("menu"))
                .fixed_pos(egui::pos2(10.0, 10.0))
                .show(ui.ctx(), |ui| app.product_menu_of(ui, "utsim"));
        };
        let mut out = plain.run_ui(input(), &mut menu);
        for _ in 0..3 {
            out.textures_delta.clear();
            out = plain.run_ui(input(), &mut menu);
        }
        out.textures_delta.clear();
        for shape in &out.shapes {
            if let egui::epaint::Shape::Text(t) = &shape.shape {
                texts.push_str(t.galley.text());
                texts.push('\n');
            }
        }
        assert!(texts.contains("Remover versões antigas (3)"), "{texts}");
        assert!(texts.contains("Mais versões (17)"), "{texts}");
        assert!(texts.contains("0.2.0-dev.22"), "{texts}");
        assert!(!texts.contains("0.2.0-dev.17"), "{texts}");
        assert_eq!(familia::tema_claro(), theme == Theme::Light);
    }
}

/// The light theme draws the grid, the list, the settings and the code screen.
#[test]
fn the_light_theme_draws() {
    let c = code();
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let m = many(&env.layout, &c);
    for view in [ViewMode::Grid, ViewMode::List] {
        let mut st = state(view, Some(&c));
        st.theme = Theme::Light;
        let mut app = App::sem_rede(Arc::clone(&env), st, Some(m.clone()));
        let tag = if view == ViewMode::Grid {
            "grade"
        } else {
            "lista"
        };
        draw(&mut app, vec2(1280.0, 820.0), &format!("claro_{tag}"));
        assert!(familia::tema_claro());
    }
    let mut st = state(ViewMode::Grid, Some(&c));
    st.theme = Theme::Light;
    let mut app = App::sem_rede(Arc::clone(&env), st.clone(), Some(m.clone()));
    app.settings_open = true;
    draw(&mut app, vec2(1280.0, 860.0), "claro_configuracoes");
    let mut app = App::sem_rede(Arc::clone(&env), st, Some(m));
    app.editing_code = true;
    draw(&mut app, vec2(1100.0, 720.0), "claro_codigo");
    // Back to dark when the setting says so.
    let mut app = App::sem_rede(env, state(ViewMode::Grid, Some(&c)), None);
    draw(&mut app, vec2(700.0, 500.0), "escuro_de_volta");
    assert!(!familia::tema_claro());
}

/// Product names (the product app and a much longer one) stay inside the cards and
/// the list rows, in the five languages, wide and narrow.
#[test]
fn long_names_fit_the_cards_in_every_language() {
    let c = code();
    let home = tempfile::tempdir().unwrap();
    let env = env(home.path());
    let mut m = many(&env.layout, &c);
    m.products.push(product(
        "produto",
        "UTSim Produto",
        "Requisitos, fases e portões, estrutura do produto e casa da qualidade.",
        vec![release(
            "0.1.0",
            "2026-11-13",
            Some(package(Some(&c), false)),
        )],
    ));
    installed(&env.layout, "produto", "0.1.0");
    m.products.push(product(
        "longo",
        "UTSim Gerenciamento Integrado de Manutenção Produtiva Total e Confiabilidade",
        "Um nome longo demais, para ver as reticências e a dica com o nome inteiro.",
        vec![release(
            "0.1.0",
            "2026-11-13",
            Some(package(Some(&c), false)),
        )],
    ));
    for codigo in ["pt-BR", "en", "es", "fr", "it"] {
        utsim_idiomas::definir_nesta_thread(utsim_idiomas::Idioma::de_codigo(codigo));
        for view in [ViewMode::Grid, ViewMode::List] {
            let tag = if view == ViewMode::Grid {
                "grade"
            } else {
                "lista"
            };
            for (size, w) in [
                (vec2(1280.0, 1100.0), "larga"),
                (vec2(720.0, 1700.0), "estreita"),
            ] {
                let mut app =
                    App::sem_rede(Arc::clone(&env), state(view, Some(&c)), Some(m.clone()));
                draw(&mut app, size, &format!("nomes_{codigo}_{tag}_{w}"));
            }
        }
    }
    utsim_idiomas::definir_nesta_thread(None);
}

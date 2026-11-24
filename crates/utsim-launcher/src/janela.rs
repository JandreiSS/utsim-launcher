//! The launcher window: a top bar with the wordmark, a toolbar (refresh, update all, order,
//! search, grid or list), the products as a grid of cards or as a table, and a footer with
//! the last check and the progress of a download. Flat and near-black (see
//! [`utsim_ui_tema::familia`]). Long work (manifest, download, report) runs on other threads.

use crate::texto::{dx, fx, sx, tx, vx};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Color32, Context, CornerRadius, Frame, Id, LayerId, Margin, Order, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureId, Ui, UiBuilder, Vec2, pos2, vec2,
};
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::download::Progress;
use utsim_launcher_core::install;
use utsim_launcher_core::manifest::{self, Manifest, Product, Release};
use utsim_launcher_core::state::{self, LauncherState, SortOrder, ViewMode};
use utsim_ui_tema::familia::{self, Botao, Icone, paleta};

use crate::Env;
use crate::abertura::{self, Intro, IntroState, Voo};
use crate::catalogo::{self, Item, Status};

/// Height of a card in the grid.
pub const CARD_H: f32 = 272.0;
/// Narrowest card; the grid has as many columns as fit.
pub const CARD_MIN_W: f32 = 330.0;
/// Height of a row in the list.
pub const ROW_H: f32 = 68.0;
/// Versions listed directly in a product's "…" menu; the rest go under "Mais versões".
const MENU_VERSIONS: usize = 5;
/// Height of the "Mais versões" list before it scrolls.
const MENU_MORE_H: f32 = 320.0;

/// What every version button of the "…" menu needs.
struct MenuVersions<'a> {
    item: &'a Item,
    installed: &'a [String],
    platform: &'a str,
    code: Option<&'a AccessCode>,
    busy: bool,
}
const TOP_H: f32 = 56.0;
/// Side of the logo in the top bar.
const LOGO: f32 = 32.0;
/// Space between cards.
const GAP: f32 = 16.0;
const FOOT_H: f32 = 30.0;
const PAD: f32 = 20.0;

/// What a background task hands back.
enum Outcome {
    Manifest(Result<(Manifest, LauncherState), String>),
    Installed(Result<String, String>),
    Message(Result<String, String>),
    /// The launcher's installer, checked and waiting in the temporary folder.
    LauncherReady(Result<std::path::PathBuf, String>),
}

/// A task running on another thread.
pub struct Task {
    pub label: String,
    /// The product being installed, if any (its card shows the progress).
    pub product: Option<String>,
    pub progress: Mutex<Option<Progress>>,
    pub cancel: AtomicBool,
    /// Fetching the manifest (the footer says "checking").
    manifest: bool,
    outcome: Mutex<Option<Outcome>>,
}

impl Task {
    fn new(label: impl Into<String>, product: Option<String>) -> Arc<Self> {
        Arc::new(Self::with(label.into(), product, false))
    }

    /// The task that fetches the manifest.
    fn new_manifest() -> Arc<Self> {
        let label = tx("janela-buscando_a_lista_de_versoes").to_string();
        Arc::new(Self::with(label, None, true))
    }

    fn with(label: String, product: Option<String>, manifest: bool) -> Self {
        Self {
            label,
            product,
            progress: Mutex::new(None),
            cancel: AtomicBool::new(false),
            manifest,
            outcome: Mutex::new(None),
        }
    }

    fn finish(&self, o: Outcome) {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = Some(o);
        }
    }

    /// Done fraction of the download, if its size is known.
    pub fn fraction(&self) -> Option<f32> {
        let p = self.progress.lock().ok().and_then(|p| *p)?;
        p.total.map(|t| p.done as f32 / t.max(1) as f32)
    }

    fn is_manifest(&self) -> bool {
        self.manifest
    }
}

/// The launcher's window state.
pub struct App {
    pub env: Arc<Env>,
    pub state: LauncherState,
    pub manifest: Option<Manifest>,
    /// The manifest shown came from the local copy (no network yet).
    pub offline: bool,
    pub code_input: String,
    /// Manifest address being edited (code screen, advanced; settings): starts as the one in use.
    pub url_input: String,
    pub code_error: Option<String>,
    pub editing_code: bool,
    pub search: String,
    pub task: Option<Arc<Task>>,
    /// Updates waiting for the current task ("Atualizar todos").
    queue: Vec<(String, String)>,
    pub status: String,
    pub status_is_error: bool,
    /// Product whose release notes are open.
    pub notes: Option<String>,
    /// Product waiting for the uninstall confirmation.
    pub confirm_uninstall: Option<String>,
    icons: HashMap<String, (usize, Option<TextureHandle>)>,
    /// The opening animation, while it plays.
    pub intro: Option<Intro>,
    less_motion: Arc<AtomicBool>,
    /// Icon places of this frame (where the opening animation lands them).
    slots: Vec<Voo>,
    /// The code screen was already offered in this session.
    code_offered: bool,
    /// The official logo of the top bar.
    logo: Option<TextureHandle>,
    /// The settings window is open.
    pub settings_open: bool,
}

impl App {
    /// The window as it opens: reads the state and the cached manifest and starts fetching
    /// the manifest at once (the animation, if any, never delays it).
    pub fn new(ctx: &Context, env: Arc<Env>, message: Option<String>, intro: bool) -> Self {
        install::clean_leftovers(&env.layout);
        let state_file = env.layout.state_file();
        let state = LauncherState::load(&state_file);
        // A code left in plain text by an older launcher is protected now (DPAPI on Windows).
        if LauncherState::code_unprotected_on_disk(&state_file) {
            match state.save(&state_file) {
                Ok(()) => env.log.line("código de acesso protegido no estado.json"),
                Err(e) => env
                    .log
                    .line(&format!("não consegui proteger o código: {e}")),
            }
        }
        apply_language(&state);
        let manifest = crate::cached_manifest(&env);
        let mut app = Self::sem_rede(env, state, manifest);
        app.offline = app.manifest.is_some();
        if let Some(m) = message {
            app.set_status(m, true);
        }
        app.editing_code = app.state.code.is_none()
            && app.env.manifest_url(&app.state).trim().is_empty()
            && app.manifest.is_none();
        if intro && !app.state.skip_intro {
            app.intro = Some(Intro::new(ctx.input(|i| i.time)));
            let flag = Arc::clone(&app.less_motion);
            let c = ctx.clone();
            std::thread::spawn(move || {
                if abertura::system_prefers_less_motion() {
                    flag.store(true, Ordering::Relaxed);
                    c.request_repaint();
                }
            });
        }
        if !app.editing_code {
            app.refresh(ctx);
        }
        app
    }

    /// A window over `manifest` that starts nothing (no network, no animation): for tests.
    pub fn sem_rede(env: Arc<Env>, state: LauncherState, manifest: Option<Manifest>) -> Self {
        let url_input = env.manifest_url(&state).to_string();
        Self {
            offline: false,
            manifest,
            code_input: String::new(),
            url_input,
            code_error: None,
            editing_code: false,
            search: String::new(),
            task: None,
            queue: Vec::new(),
            status: String::new(),
            status_is_error: false,
            notes: None,
            confirm_uninstall: None,
            icons: HashMap::new(),
            intro: None,
            less_motion: Arc::new(AtomicBool::new(false)),
            slots: Vec::new(),
            code_offered: false,
            logo: None,
            settings_open: false,
            state,
            env,
        }
    }

    pub fn code(&self) -> Option<AccessCode> {
        self.state
            .code
            .as_deref()
            .and_then(|c| AccessCode::parse(c).ok())
    }

    fn save_state(&mut self) {
        if let Err(e) = self.state.save(&self.env.layout.state_file()) {
            self.set_status(
                fx(
                    "janela-nao_consegui_gravar_as_preferencias",
                    &[("e", vx(&e))],
                ),
                true,
            );
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = msg.into();
        self.status_is_error = error;
        if error {
            self.env.log.line(&format!("erro: {}", self.status));
        }
    }

    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// A task shown as running (for tests and the footer).
    pub fn show_task(&mut self, label: &str, product: Option<&str>, progress: Option<Progress>) {
        let t = Task::new(label, product.map(String::from));
        if let Ok(mut p) = t.progress.lock() {
            *p = progress;
        }
        self.task = Some(t);
    }

    /// The products as shown now (filtered and sorted).
    pub fn items(&self) -> Vec<Item> {
        let Some(m) = &self.manifest else {
            return Vec::new();
        };
        let code = self.code();
        catalogo::items(
            m,
            &self.env.layout,
            &self.state,
            &self.env.platform,
            code.as_ref(),
            &self.search,
        )
    }

    /// Fetches the manifest in the background.
    pub fn refresh(&mut self, ctx: &Context) {
        if self.busy() {
            return;
        }
        let task = Task::new_manifest();
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let mut state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = crate::fetch_manifest(&env, &mut state)
                .map(|m| (m, state))
                .map_err(|e| e.to_string());
            task.finish(Outcome::Manifest(r));
            ctx.request_repaint();
        });
    }

    fn find(&self, product: &str, version: &str) -> Option<(Product, Release)> {
        let p = self.manifest.as_ref()?.product(product)?;
        Some((p.clone(), p.release(version)?.clone()))
    }

    fn start_install(&mut self, ctx: &Context, product: &str, version: &str) {
        let Some((product, release)) = self.find(product, version) else {
            return;
        };
        let code = self.code();
        let can = release
            .package(&self.env.platform)
            .is_some_and(|p| crate::can_install(p, code.as_ref()));
        if !can {
            self.editing_code = true;
            return;
        }
        let task = Task::new(
            fx(
                "janela-instalando",
                &[
                    ("name", vx(&product.name)),
                    ("version", vx(&release.version)),
                ],
            ),
            Some(product.id.clone()),
        );
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let t = Arc::clone(&task);
            let c = ctx.clone();
            let mut progress = move |p: Progress| {
                if let Ok(mut slot) = t.progress.lock() {
                    *slot = Some(p);
                }
                c.request_repaint();
            };
            let r = crate::install_release(
                &env,
                &state,
                &product,
                &release,
                code.as_ref(),
                &task.cancel,
                &mut progress,
            )
            .map(|_| {
                fx(
                    "janela-instalado",
                    &[
                        ("name", vx(&product.name)),
                        ("version", vx(&release.version)),
                    ],
                )
            })
            .map_err(|e| e.to_string());
            task.finish(Outcome::Installed(r));
            ctx.request_repaint();
        });
    }

    /// The newer launcher offered at the top of the window, if any.
    pub fn launcher_update(&self) -> Option<(Product, Release)> {
        let m = self.manifest.as_ref()?;
        let code = self.code();
        let r = crate::launcher_update(&self.env, &self.state, m, crate::VERSAO, code.as_ref())?;
        Some((m.product(manifest::LAUNCHER)?.clone(), r.clone()))
    }

    /// Downloads and checks the new launcher; [`Outcome::LauncherReady`] runs it.
    fn start_launcher_update(&mut self, ctx: &Context) {
        let Some((product, release)) = self.launcher_update() else {
            return;
        };
        let code = self.code();
        let task = Task::new(
            fx(
                "janela-baixando_o_launcher",
                &[("version", vx(&release.version))],
            ),
            Some(product.id.clone()),
        );
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let t = Arc::clone(&task);
            let c = ctx.clone();
            let mut progress = move |p: Progress| {
                if let Ok(mut slot) = t.progress.lock() {
                    *slot = Some(p);
                }
                c.request_repaint();
            };
            let r = crate::prepare_launcher_update(
                &env,
                &state,
                &product,
                &release,
                code.as_ref(),
                &crate::launcher_update_dir(),
                &task.cancel,
                &mut progress,
            )
            .map_err(|e| e.to_string());
            task.finish(Outcome::LauncherReady(r));
            ctx.request_repaint();
        });
    }

    fn update_all(&mut self, ctx: &Context) {
        let mut queue: Vec<(String, String)> = self
            .items()
            .into_iter()
            .filter_map(|i| match i.status {
                Status::Update { to, .. } => Some((i.id, to)),
                _ => None,
            })
            .collect();
        queue.reverse();
        if let Some((p, v)) = queue.pop() {
            self.queue = queue;
            self.start_install(ctx, &p, &v);
        }
    }

    fn start_report(&mut self, ctx: &Context) {
        let task = Task::new(tx("janela-gerando_o_relato_de_erro"), None);
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = crate::error_report(&env, &state, &crate::downloads_dir())
                .map(|p| {
                    fx(
                        "janela-relato_salvo_em_envie_este",
                        &[("p", vx(&p.display()))],
                    )
                })
                .map_err(|e| e.to_string());
            task.finish(Outcome::Message(r));
            ctx.request_repaint();
        });
    }

    fn poll_task(&mut self, ctx: &Context) {
        let Some(task) = &self.task else {
            return;
        };
        let outcome = task.outcome.lock().ok().and_then(|mut o| o.take());
        let Some(outcome) = outcome else {
            return;
        };
        let cancelled = task.cancel.load(Ordering::Relaxed);
        self.task = None;
        match outcome {
            Outcome::Manifest(Ok((m, state))) => {
                self.state = state;
                self.state.checked_at = state::now_secs();
                self.save_state();
                self.offline = false;
                self.manifest = Some(m);
                // No code yet and something needs one: offer the code screen once.
                if self.state.code.is_none()
                    && !self.code_offered
                    && catalogo::needs_code(&self.items())
                {
                    self.code_offered = true;
                    self.editing_code = true;
                }
                if !self.status_is_error {
                    self.set_status(tx("janela-lista_de_versoes_atualizada"), false);
                }
            }
            Outcome::Manifest(Err(e)) => {
                let extra = if self.manifest.is_some() {
                    tx("janela-mostrando_a_ultima_lista_baixada")
                } else {
                    ""
                };
                self.set_status(format!("{e}.{extra}"), true);
            }
            Outcome::Installed(Ok(msg)) => {
                self.set_status(msg, false);
                if let Some((p, v)) = self.queue.pop() {
                    self.start_install(ctx, &p, &v);
                }
            }
            Outcome::Message(Ok(msg)) => self.set_status(msg, false),
            Outcome::LauncherReady(Ok(installer)) => {
                match crate::run_launcher_installer(&installer) {
                    Ok(()) => {
                        self.env.log.line(
                            "instalador do launcher aberto; o launcher fecha para ser trocado",
                        );
                        self.save_state();
                        self.set_status(tx("janela-atualizando_o_launcher_ele_reabre"), false);
                        // Closing frees the program's file for the installer.
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => self.set_status(e.to_string(), true),
                }
            }
            Outcome::Installed(Err(e))
            | Outcome::Message(Err(e))
            | Outcome::LauncherReady(Err(e)) => {
                self.queue.clear();
                if cancelled {
                    self.set_status(tx("janela-download_cancelado_ele_continua_de"), false);
                } else {
                    self.set_status(e, true);
                }
            }
        }
    }

    fn texture(&mut self, ctx: &Context, item: &Item) -> Option<TextureId> {
        let len = item.icon.as_ref().map_or(0, Vec::len);
        if let Some((l, t)) = self.icons.get(&item.id)
            && *l == len
        {
            return t.as_ref().map(TextureHandle::id);
        }
        let tex = item
            .icon
            .as_deref()
            .and_then(familia::imagem_png)
            .map(|img| {
                ctx.load_texture(
                    format!("icone-{}", item.id),
                    img,
                    egui::TextureOptions::LINEAR,
                )
            });
        let id = tex.as_ref().map(TextureHandle::id);
        self.icons.insert(item.id.clone(), (len, tex));
        id
    }

    /// Draws the whole window into `ui` (one frame).
    pub fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.poll_task(&ctx);
        self.slots.clear();

        // The opening animation: any key or click skips it; the system's "less motion" too.
        let now = ui.input(|i| i.time);
        let mut frame: Option<IntroState> = None;
        if let Some(intro) = &mut self.intro {
            let skip = self.less_motion.load(Ordering::Relaxed)
                || ui.input(|i| {
                    i.pointer.any_pressed()
                        || i.events
                            .iter()
                            .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
                });
            if skip {
                intro.skip();
            }
            let f = intro.frame(now);
            if f.done {
                self.intro = None;
            } else {
                frame = Some(f);
                ctx.request_repaint();
            }
        }
        if let Some(f) = &frame {
            ui.multiply_opacity(f.ui_alpha.max(0.001));
        }
        let icon_alpha = frame.map_or(1.0, |f| 1.0 - f.icons_alpha);

        if self.editing_code {
            egui::CentralPanel::default()
                .frame(Frame::NONE.fill(paleta().fundo))
                .show(ui, |ui| self.code_screen(ui));
        } else {
            egui::Panel::top("topo")
                .exact_size(TOP_H)
                .frame(Frame::NONE.fill(paleta().fundo))
                .show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("rodape")
                .exact_size(FOOT_H)
                .frame(Frame::NONE.fill(paleta().fundo))
                .show(ui, |ui| self.footer(ui));
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(paleta().fundo)
                        .inner_margin(Margin::symmetric(PAD as i8, 0)),
                )
                .show(ui, |ui| self.main_area(ui, icon_alpha));
            self.modals(&ctx);
        }

        if let Some(f) = frame {
            let screen = ctx.content_rect();
            egui::Area::new(Id::new("abertura"))
                .order(Order::Foreground)
                .fixed_pos(screen.min)
                .show(&ctx, |ui| {
                    ui.set_opacity(1.0);
                    let resp = ui.allocate_rect(screen, Sense::click());
                    let _ = resp;
                    let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("abertura")));
                    abertura::pintar(&p, screen, &f, &self.slots);
                });
        }
    }

    fn code_screen(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let w = 460.0_f32.min(full.width() - 2.0 * PAD);
        let col = Rect::from_min_size(
            pos2(full.center().x - w / 2.0, full.top() + 70.0),
            vec2(w, full.height() - 70.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(col), |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.label(familia::espacado(
                    "UTSIM",
                    familia::negrito(22.0),
                    paleta().branco,
                    4.0,
                ));
                ui.add_space(12.0);
                ui.label(familia::espacado(
                    tx("janela-familia_de_apps"),
                    familia::mono(11.0),
                    paleta().texto_fraco,
                    2.5,
                ));
            });
            ui.add_space(18.0);
            familia::rotulo(
                ui,
                tx("janela-codigo_de_acesso"),
                11.0,
                paleta().texto_apagado,
            );
            ui.label(
                egui::RichText::new(tx("janela-digite_o_codigo_que_voce"))
                    .color(paleta().texto_fraco),
            );
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.code_input)
                    .hint_text("XXXX-XXXX-XXXX-XXXX-XXXX")
                    .font(familia::mono(17.0))
                    .desired_width(w)
                    .min_size(vec2(w, 42.0))
                    .vertical_align(egui::Align::Center)
                    .margin(Margin::symmetric(12, 8)),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(4.0);
            let can_skip = self.state.code.is_some()
                || self.manifest.is_some()
                || !self.env.manifest_url(&self.state).trim().is_empty();
            let mut ok = false;
            ui.horizontal(|ui| {
                ok = familia::botao(
                    ui,
                    tx("janela-continuar"),
                    Botao::Primario,
                    vec2(180.0, 36.0),
                    true,
                )
                .clicked()
                    || enter;
                if can_skip {
                    let label = if self.state.code.is_some() {
                        tx("janela-cancelar")
                    } else {
                        tx("janela-agora_nao")
                    };
                    if familia::botao(ui, label, Botao::Contorno, vec2(140.0, 36.0), true).clicked()
                    {
                        self.editing_code = false;
                        self.code_error = None;
                    }
                }
            });
            if ok {
                self.accept_code(ui.ctx());
            }
            if let Some(e) = &self.code_error {
                ui.colored_label(paleta().erro, e);
            }
            ui.add_space(12.0);
            let no_builtin = crate::built_in_manifest_url().is_empty();
            egui::CollapsingHeader::new(familia::espacado(
                tx("janela-avancado"),
                familia::mono(11.0),
                paleta().texto_fraco,
                2.0,
            ))
            .default_open(no_builtin)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(tx("janela-endereco_da_lista_de_versoes"))
                        .color(paleta().texto_fraco),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.url_input)
                        .hint_text(tx("janela-link_do_manifesto_json_no"))
                        .desired_width(w - 20.0)
                        .margin(Margin::symmetric(10, 7)),
                );
            });
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(tx("janela-o_codigo_fica_guardado_so"))
                    .size(12.0)
                    .color(paleta().texto_apagado),
            );
        });
    }

    fn accept_code(&mut self, ctx: &Context) {
        let url = self.url_input.trim().to_string();
        let typed = self.code_input.trim().to_string();
        if typed.is_empty() {
            if url.is_empty() && self.env.manifest_url(&self.state).trim().is_empty() {
                self.code_error = Some(tx("janela-digite_o_codigo_ou_em").into());
                return;
            }
        } else {
            match AccessCode::parse(&typed) {
                Ok(code) => self.state.code = Some(code.formatted()),
                Err(e) => {
                    self.code_error = Some(e.to_string());
                    return;
                }
            }
        }
        self.state.manifest_url = crate::manifest_override(&url);
        self.code_input.clear();
        self.code_error = None;
        self.editing_code = false;
        self.save_state();
        self.refresh(ctx);
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let r = ui.max_rect();
        let p = ui.painter().clone();
        p.line_segment(
            [
                r.left_bottom() + vec2(0.0, -0.5),
                r.right_bottom() + vec2(0.0, -0.5),
            ],
            Stroke::new(1.0, paleta().linha),
        );
        // The official logo (the three gears), then the wordmark.
        let logo = Rect::from_min_size(
            pos2(r.left() + PAD, r.center().y - LOGO / 2.0),
            Vec2::splat(LOGO),
        );
        if let Some(id) = self.logo(ui.ctx()) {
            p.image(
                id,
                logo,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let mark = familia::pintar_texto(
            &p,
            pos2(logo.right() + 12.0, r.center().y),
            familia::espacado("UTSIM", familia::negrito(17.0), paleta().branco, 3.5),
        );
        let x = mark.right() + 22.0;
        p.line_segment(
            [pos2(x, r.top()), pos2(x, r.bottom() - 1.0)],
            Stroke::new(1.0, paleta().linha),
        );
        familia::pintar_texto(
            &p,
            pos2(x + 16.0, r.center().y),
            familia::espacado(
                tx("janela-familia_de_apps"),
                familia::mono(11.0),
                paleta().texto_fraco,
                2.6,
            ),
        );

        let gear = Rect::from_min_size(pos2(r.right() - TOP_H, r.top()), vec2(TOP_H, TOP_H - 1.0));
        p.line_segment(
            [gear.left_top(), gear.left_bottom()],
            Stroke::new(1.0, paleta().linha),
        );
        let resp = ui
            .allocate_rect(gear, Sense::click())
            .on_hover_text(tx("janela-configuracoes"));
        if resp.hovered() {
            p.rect_filled(gear.shrink2(vec2(0.5, 0.0)), 0.0, paleta().sobre);
        }
        familia::pintar_icone(
            &p,
            Rect::from_center_size(gear.center(), Vec2::splat(18.0)),
            Icone::Engrenagem,
            paleta().texto,
        );
        if resp.clicked() {
            self.settings_open = true;
        }

        // Who is testing: a chip that opens the code screen.
        let code = self.code();
        let text = match &code {
            Some(c) => fx("janela-testador", &[("codigo", vx(&c.id().to_uppercase()))]),
            None => tx("janela-sem_codigo_de_testador").to_string(),
        };
        let g = p.layout_job(familia::espacado(
            &text,
            familia::mono(10.5),
            paleta().texto_fraco,
            1.6,
        ));
        let chip = Rect::from_min_size(
            pos2(gear.left() - 16.0 - g.size().x - 24.0, r.center().y - 14.0),
            vec2(g.size().x + 24.0, 28.0),
        );
        if chip.left() > x + 200.0 {
            let busy = self.busy();
            let resp = ui
                .allocate_rect(chip, if busy { Sense::hover() } else { Sense::click() })
                .on_hover_text(if code.is_some() {
                    tx("janela-trocar_o_codigo_de_testador")
                } else {
                    tx("janela-inserir_um_codigo_de_testador")
                });
            let fill = if resp.hovered() && !busy {
                paleta().sobre
            } else {
                paleta().fundo
            };
            p.rect(
                chip,
                CornerRadius::same(14),
                fill,
                Stroke::new(1.0, paleta().linha),
                StrokeKind::Inside,
            );
            let dot = pos2(chip.left() + 12.0, chip.center().y);
            let on = if code.is_some() {
                paleta().verde
            } else {
                paleta().texto_apagado
            };
            p.circle_filled(dot, 3.0, on);
            p.galley(
                pos2(chip.left() + 20.0, chip.center().y - g.size().y / 2.0),
                g,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.editing_code = true;
            }
        }
    }

    /// The texture of the official logo, loaded once.
    fn logo(&mut self, ctx: &Context) -> Option<TextureId> {
        if self.logo.is_none() {
            self.logo = familia::imagem_png(catalogo::LOGO_UTSIM)
                .map(|img| ctx.load_texture("logo-utsim", img, egui::TextureOptions::LINEAR));
        }
        self.logo.as_ref().map(TextureHandle::id)
    }

    /// The settings window: tester code, installation, list of versions, opening, help.
    fn settings(&mut self, ctx: &Context) {
        if !self.settings_open {
            return;
        }
        let modal = egui::Modal::new(Id::new("configuracoes"))
            .frame(
                Frame::NONE
                    .fill(paleta().superficie)
                    .stroke(Stroke::new(1.0, paleta().linha_forte))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(24)),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0_f32.min(ctx.content_rect().width() - 80.0));
                ui.label(
                    egui::RichText::new(tx("janela-configuracoes"))
                        .font(familia::negrito(20.0))
                        .color(paleta().branco),
                );
                ui.add_space(4.0);
                self.settings_body(ui);
                ui.add_space(16.0);
                familia::botao(
                    ui,
                    tx("janela-fechar"),
                    Botao::Contorno,
                    vec2(120.0, 32.0),
                    true,
                )
                .clicked()
            });
        if modal.inner || modal.should_close() {
            self.settings_open = false;
        }
    }

    fn settings_body(&mut self, ui: &mut Ui) {
        let busy = self.busy();
        let section = |ui: &mut Ui, title: &str| {
            ui.add_space(14.0);
            familia::rotulo(ui, title, 10.5, paleta().texto_apagado);
            ui.add_space(2.0);
        };
        let weak = |t: String| egui::RichText::new(t).color(paleta().texto_fraco);

        section(ui, tx("janela-testador-2"));
        let code = self.code();
        ui.label(match &code {
            Some(c) => fx("janela-codigo_de", &[("id", vx(&c.id()))]),
            None => tx("janela-sem_codigo_so_os_aplicativos").into(),
        });
        let label = if code.is_some() {
            tx("janela-trocar_codigo")
        } else {
            tx("janela-inserir_codigo")
        };
        if familia::botao(ui, label, Botao::Contorno, vec2(170.0, 30.0), !busy).clicked() {
            self.editing_code = true;
            self.settings_open = false;
        }

        section(ui, tx("janela-instalacao"));
        let mut desktop = self.state.desktop_shortcuts;
        if ui
            .checkbox(&mut desktop, tx("janela-criar_atalho_na_area_de"))
            .changed()
        {
            self.state.desktop_shortcuts = desktop;
            self.save_state();
        }
        let root = self.env.layout.root.clone();
        ui.horizontal(|ui| {
            ui.label(weak(fx(
                "janela-aplicativos_em",
                &[("root", vx(&root.display()))],
            )));
            if ui.small_button(tx("janela-abrir_pasta")).clicked() {
                open_folder(&root);
            }
        });

        section(ui, tx("janela-lista_de_versoes"));
        ui.label(weak(self.check_line().to_lowercase()));
        ui.add_space(4.0);
        ui.label(weak(tx("janela-endereco_em_uso").into()));
        ui.add(
            egui::TextEdit::singleline(&mut self.url_input)
                .hint_text(tx("janela-link_do_manifesto_json_no"))
                .desired_width(ui.available_width())
                .margin(Margin::symmetric(10, 7)),
        );
        let typed = crate::manifest_override(&self.url_input);
        let changed = typed != self.state.manifest_url;
        let has_builtin = !crate::built_in_manifest_url().trim().is_empty();
        let mut fetch = false;
        ui.horizontal(|ui| {
            if changed {
                if familia::botao(
                    ui,
                    tx("janela-salvar_e_buscar"),
                    Botao::Primario,
                    vec2(170.0, 30.0),
                    !busy,
                )
                .clicked()
                {
                    self.state.manifest_url = typed.clone();
                    fetch = true;
                }
            } else if familia::botao(
                ui,
                tx("janela-buscar_agora"),
                Botao::Contorno,
                vec2(170.0, 30.0),
                !busy,
            )
            .clicked()
            {
                fetch = true;
            }
            if has_builtin
                && (self.state.manifest_url.is_some() || changed)
                && familia::botao(
                    ui,
                    tx("janela-usar_o_padrao"),
                    Botao::Contorno,
                    vec2(150.0, 30.0),
                    !busy,
                )
                .clicked()
            {
                self.use_built_in_url();
                fetch = true;
            }
        });
        if fetch {
            self.save_state();
            self.refresh(ui.ctx());
        }

        section(ui, tx("janela-aparencia"));
        ui.horizontal(|ui| {
            ui.label(tx("janela-idioma"));
            let mut i = utsim_idiomas::atual();
            if utsim_idiomas::seletor(ui, "idioma", &mut i) {
                utsim_idiomas::definir(i);
                self.state.language = Some(i.codigo().to_string());
                self.save_state();
            }
        });

        section(ui, tx("janela-abertura"));
        let mut intro = !self.state.skip_intro;
        if ui
            .checkbox(&mut intro, tx("janela-mostrar_a_animacao_ao_abrir"))
            .changed()
        {
            self.state.skip_intro = !intro;
            self.save_state();
        }

        section(ui, tx("janela-ajuda"));
        if familia::botao(
            ui,
            tx("janela-gerar_relato_de_erro"),
            Botao::Contorno,
            vec2(220.0, 30.0),
            !busy,
        )
        .on_hover_text(tx("janela-cria_um_zip_com_os"))
        .clicked()
        {
            self.start_report(ui.ctx());
            self.settings_open = false;
        }
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(fx(
                "janela-utsim_launcher",
                &[("versao", vx(&env!("CARGO_PKG_VERSION")))],
            ))
            .size(11.5)
            .color(paleta().texto_apagado),
        );
    }

    fn main_area(&mut self, ui: &mut Ui, icon_alpha: f32) {
        let items = self.items();
        self.launcher_banner(ui);
        self.missing_list_banner(ui);
        self.toolbar(ui, &items);
        let body = ui.available_rect_before_wrap();
        let Some(m) = &self.manifest else {
            let busy = self.busy();
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(if busy {
                            tx("janela-buscando_a_lista_de_aplicativos")
                        } else {
                            tx("janela-nenhuma_lista_de_aplicativos_ainda")
                        })
                        .size(15.0)
                        .color(paleta().texto_fraco),
                    );
                    ui.add_space(12.0);
                    if !busy
                        && familia::botao(
                            ui,
                            tx("janela-atualizar_lista"),
                            Botao::Contorno,
                            vec2(200.0, 34.0),
                            true,
                        )
                        .clicked()
                    {
                        self.refresh(ui.ctx());
                    }
                    if self.state.code.is_none()
                        && familia::botao(
                            ui,
                            tx("janela-inserir_codigo"),
                            Botao::Contorno,
                            vec2(200.0, 34.0),
                            true,
                        )
                        .clicked()
                    {
                        self.editing_code = true;
                    }
                });
            });
            return;
        };
        if items.is_empty() {
            let text = if m.apps().next().is_none() {
                tx("janela-a_lista_nao_tem_aplicativos").to_string()
            } else {
                fx(
                    "janela-nenhum_aplicativo_encontrado_para",
                    &[("search", vx(&self.search.trim()))],
                )
            };
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(15.0)
                            .color(paleta().texto_fraco),
                    );
                });
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.updates_banner(ui, &items);
                match self.state.view {
                    ViewMode::Grid => self.grid(ui, &items, icon_alpha),
                    ViewMode::List => self.list(ui, &items, icon_alpha),
                }
            });
    }

    fn toolbar(&mut self, ui: &mut Ui, items: &[Item]) {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
        let h = 34.0;
        let y = bar.center().y - h / 2.0;
        let busy = self.busy();
        let narrow = bar.width() < 900.0;
        let mut x = bar.right();
        // Grid and list toggle.
        for (view, icon) in [
            (ViewMode::List, Icone::Lista),
            (ViewMode::Grid, Icone::Grade),
        ] {
            x -= h;
            let r = Rect::from_min_size(pos2(x, y), Vec2::splat(h));
            let tip = if view == ViewMode::Grid {
                tx("janela-grade")
            } else {
                tx("janela-lista")
            };
            if familia::botao_icone_em(ui, r, icon, self.state.view == view, true)
                .on_hover_text(tip)
                .clicked()
                && self.state.view != view
            {
                self.state.view = view;
                self.save_state();
            }
            x -= 2.0;
        }
        // Search.
        let sw = if narrow { 150.0 } else { 220.0 };
        x -= 8.0 + sw;
        let search = Rect::from_min_size(pos2(x, y), vec2(sw, h));
        ui.scope_builder(UiBuilder::new().max_rect(search), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tx("janela-buscar_aplicativos"))
                    .desired_width(sw)
                    .min_size(vec2(sw, h))
                    .vertical_align(egui::Align::Center)
                    .margin(Margin::symmetric(10, 0)),
            );
        });
        // Order.
        let cw = if narrow { 150.0 } else { 180.0 };
        x -= 8.0 + cw;
        let combo = Rect::from_min_size(pos2(x, y), vec2(cw, h));
        let mut sort = self.state.sort;
        ui.scope_builder(UiBuilder::new().max_rect(combo), |ui| {
            ui.spacing_mut().interact_size.y = h;
            egui::ComboBox::from_id_salt("ordem")
                .width(cw - 12.0)
                .selected_text(sort_label(sort))
                .show_ui(ui, |ui| {
                    for o in SortOrder::ALL {
                        ui.selectable_value(&mut sort, o, sort_label(o));
                    }
                });
        });
        if sort != self.state.sort {
            self.state.sort = sort;
            self.save_state();
        }
        // Refresh the list.
        x -= 8.0 + h;
        let refresh = Rect::from_min_size(pos2(x, y), Vec2::splat(h));
        if familia::botao_icone_em(ui, refresh, Icone::Atualizar, false, !busy)
            .on_hover_text(tx("janela-buscar_a_lista_de_versoes"))
            .clicked()
        {
            self.refresh(ui.ctx());
        }
        // Title and summary on the left, where they fit.
        let p = ui.painter();
        let title = p.layout_no_wrap(
            tx("janela-aplicativos").to_string(),
            familia::negrito(22.0),
            paleta().branco,
        );
        if bar.left() + title.size().x + 16.0 < x {
            let top = bar.center().y - 24.0;
            p.galley(pos2(bar.left(), top), title, paleta().branco);
            let installed = items.iter().filter(|i| i.active.is_some()).count();
            let updates = items.iter().filter(|i| i.has_update()).count();
            let mut parts = vec![
                fx("janela-no_total", &[("n", vx(&items.len()))]),
                fx("janela-instalados", &[("n", vx(&installed))]),
            ];
            if updates > 0 {
                parts.push(fx(
                    "janela-atualizacoes_disponiveis",
                    &[("n", vx(&updates))],
                ));
            }
            p.galley(
                pos2(bar.left(), top + 29.0),
                p.layout_job(familia::linha(
                    &parts.join(" · "),
                    familia::sans(13.0),
                    paleta().texto_fraco,
                    x - bar.left() - 16.0,
                )),
                Color32::WHITE,
            );
        }
    }

    /// The strip that lists the updates waiting, with "Atualizar todos".
    /// "Nova versão do launcher X.Y.Z — Atualizar", above everything (not a modal). While it
    /// downloads, the progress takes the place of the button.
    /// Goes back to the address built into this launcher (forgets the override).
    fn use_built_in_url(&mut self) {
        self.state.manifest_url = None;
        self.url_input = self.env.manifest_url(&self.state).to_string();
    }

    /// The list address is a local file that is not there (desktop Drive not mounted): says
    /// where it looked and offers the built-in address.
    fn missing_list_banner(&mut self, ui: &mut Ui) {
        let url = self.env.manifest_url(&self.state).to_string();
        let has_builtin = !crate::built_in_manifest_url().trim().is_empty();
        // Only after a failed fetch (no disk check on every frame).
        if !self.status_is_error || self.busy() || !has_builtin || self.state.manifest_url.is_none()
        {
            return;
        }
        let Some(path) = crate::missing_local_list(&url) else {
            return;
        };
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect(
            r,
            CornerRadius::same(10),
            paleta().erro.gamma_multiply(0.08),
            Stroke::new(1.0, paleta().erro.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
        let bw = 150.0;
        let button = Rect::from_min_size(
            pos2(r.right() - 14.0 - bw, r.center().y - 17.0),
            vec2(bw, 34.0),
        );
        let x = r.left() + 18.0;
        let text = fx(
            "lib-lista_local_nao_encontrada",
            &[("path", vx(&path.display()))],
        );
        let g = p.layout_job(familia::linha(
            &text,
            familia::sans(13.0),
            paleta().texto,
            button.left() - x - 16.0,
        ));
        let tr = Rect::from_min_size(pos2(x, r.center().y - g.size().y / 2.0), g.size());
        p.galley(tr.min, g, Color32::WHITE);
        ui.allocate_rect(tr, Sense::hover()).on_hover_text(text);
        if familia::botao_em(
            ui,
            button,
            tx("janela-usar_o_padrao"),
            Botao::Primario,
            true,
        )
        .clicked()
        {
            self.use_built_in_url();
            self.save_state();
            self.refresh(ui.ctx());
        }
        ui.add_space(4.0);
    }

    fn launcher_banner(&mut self, ui: &mut Ui) {
        let Some((_, release)) = self.launcher_update() else {
            return;
        };
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect(
            r,
            CornerRadius::same(10),
            paleta().destaque.gamma_multiply(0.08),
            Stroke::new(1.0, paleta().destaque.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
        let bw = 150.0;
        let button = Rect::from_min_size(
            pos2(r.right() - 14.0 - bw, r.center().y - 17.0),
            vec2(bw, 34.0),
        );
        let x = r.left() + 18.0;
        let text = fx(
            "janela-nova_versao_do_launcher_voce",
            &[
                ("version", vx(&release.version)),
                ("atual", vx(&crate::VERSAO)),
            ],
        );
        p.galley(
            pos2(x, r.center().y - 9.0),
            p.layout_job(familia::linha(
                &text,
                familia::sans(14.0),
                paleta().texto,
                button.left() - x - 16.0,
            )),
            Color32::WHITE,
        );
        let running = self
            .task
            .as_ref()
            .filter(|t| t.product.as_deref() == Some(manifest::LAUNCHER))
            .map(|t| t.fraction());
        if let Some(frac) = running {
            let bar = Rect::from_min_size(pos2(button.left(), r.center().y - 2.0), vec2(bw, 4.0));
            progress_bar(ui, bar, frac);
            ui.ctx().request_repaint();
        } else if familia::botao_em(
            ui,
            button,
            tx("janela-atualizar"),
            Botao::Primario,
            !self.busy(),
        )
        .clicked()
        {
            self.start_launcher_update(ui.ctx());
        }
        ui.add_space(4.0);
    }

    fn updates_banner(&mut self, ui: &mut Ui, items: &[Item]) {
        let ups: Vec<&Item> = items.iter().filter(|i| i.has_update()).collect();
        if ups.is_empty() {
            return;
        }
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect(
            r,
            CornerRadius::same(10),
            paleta().amarelo.gamma_multiply(0.07),
            Stroke::new(1.0, paleta().amarelo.gamma_multiply(0.45)),
            StrokeKind::Inside,
        );
        let n = ups.len();
        let head = fx(
            "janela-atualizacoes_disponiveis_maiusculas",
            &[("n", vx(&n))],
        );
        let bw = 190.0;
        let button = Rect::from_min_size(
            pos2(r.right() - 14.0 - bw, r.center().y - 17.0),
            vec2(bw, 34.0),
        );
        let x = r.left() + 18.0;
        familia::pintar_texto(
            &p,
            pos2(x, r.top() + 19.0),
            familia::espacado(&head, familia::mono_negrito(11.0), paleta().amarelo, 1.6),
        );
        let list = ups
            .iter()
            .map(|i| {
                let (a, b) = i.versions();
                format!(
                    "{} {} → {}",
                    i.name,
                    a.unwrap_or_default(),
                    b.unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("   ·   ");
        p.galley(
            pos2(x, r.top() + 31.0),
            p.layout_job(familia::linha(
                &list,
                familia::sans(13.0),
                paleta().texto,
                button.left() - x - 16.0,
            )),
            Color32::WHITE,
        );
        let label = if n == 1 {
            tx("janela-atualizar").to_string()
        } else {
            fx("janela-atualizar_todos", &[("n", vx(&n))])
        };
        let busy = self.busy();
        if familia::botao_em(ui, button, &label, Botao::Primario, !busy).clicked() {
            self.update_all(ui.ctx());
        }
        ui.add_space(18.0);
    }

    /// The cards in two groups: the apps on this computer, then the others.
    fn grid(&mut self, ui: &mut Ui, items: &[Item], icon_alpha: f32) {
        let (mine, others): (Vec<&Item>, Vec<&Item>) =
            items.iter().partition(|i| i.active.is_some());
        for (title, group) in [
            (tx("janela-neste_computador"), mine),
            (tx("janela-disponiveis_para_instalar"), others),
        ] {
            if group.is_empty() {
                continue;
            }
            familia::rotulo(
                ui,
                &format!("{title} · {}", group.len()),
                11.0,
                paleta().texto_fraco,
            );
            ui.add_space(8.0);
            let w = ui.available_width();
            let cols = (((w + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
            let cw = ((w - GAP * (cols - 1) as f32) / cols as f32).floor();
            let rows = group.len().div_ceil(cols);
            let (rect, _) = ui.allocate_exact_size(
                vec2(w, rows as f32 * CARD_H + (rows - 1) as f32 * GAP),
                Sense::hover(),
            );
            for (k, item) in group.iter().enumerate() {
                let (row, col) = (k / cols, k % cols);
                let cell = Rect::from_min_size(
                    pos2(
                        rect.left() + col as f32 * (cw + GAP),
                        rect.top() + row as f32 * (CARD_H + GAP),
                    ),
                    vec2(cw, CARD_H),
                );
                ui.push_id(&item.id, |ui| self.card(ui, cell, item, icon_alpha));
            }
            ui.add_space(26.0);
        }
    }

    fn icon(&mut self, ui: &Ui, rect: Rect, item: &Item, alpha: f32) {
        let tex = self.texture(ui.ctx(), item);
        let accent = accent(item);
        let initials = familia::iniciais(&item.name);
        if alpha > 0.0 {
            match tex {
                Some(id) => {
                    ui.painter().image(
                        id,
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE.gamma_multiply(alpha),
                    );
                }
                None => familia::pintar_monograma(
                    ui.painter(),
                    rect,
                    &initials,
                    accent.gamma_multiply(alpha),
                ),
            }
        }
        self.slots.push(Voo {
            texture: tex,
            initials,
            accent,
            target: rect,
        });
    }

    fn card(&mut self, ui: &mut Ui, cell: Rect, item: &Item, icon_alpha: f32) {
        let p = ui.painter().clone();
        let acc = accent(item);
        let over = ui.rect_contains_pointer(cell);
        p.rect(
            cell,
            CornerRadius::same(10),
            if over {
                paleta().sobre
            } else {
                paleta().superficie
            },
            Stroke::new(
                1.0,
                if over {
                    paleta().linha_forte
                } else {
                    paleta().linha
                },
            ),
            StrokeKind::Inside,
        );
        // A line in the app's colour along the top edge (the card's own corners).
        p.with_clip_rect(Rect::from_min_size(cell.min, vec2(cell.width(), 3.0)))
            .rect_filled(cell, CornerRadius::same(10), acc);

        // Icon, name, subtitle and author.
        let left = cell.left() + PAD;
        let inner_w = cell.width() - 2.0 * PAD;
        let icon = Rect::from_min_size(pos2(left, cell.top() + PAD + 2.0), Vec2::splat(56.0));
        self.icon(ui, icon, item, icon_alpha);
        let text_x = icon.right() + 14.0;
        let name_w = cell.right() - PAD - text_x;
        // The name wraps to two lines, then gets a little smaller, and only as a last
        // resort ends in "…" (with the whole name in a tooltip); the badges below stay put.
        let by = icon.bottom() + 22.0;
        let name_top = icon.top() + 4.0;
        let (g, cut) = fit_name(&p, item, &NAME_CARD, name_w, by - 26.0 - name_top);
        let name_rect = Rect::from_min_size(pos2(text_x, name_top), g.size());
        p.galley(name_rect.min, g, Color32::WHITE);
        name_tooltip(ui, name_rect, item, cut);
        let tag = fx("janela-por", &[("autor", vx(&item.author.to_uppercase()))]);
        let tg = p.layout_job(familia::espacado(
            &tag,
            familia::mono(9.5),
            paleta().texto_fraco,
            1.2,
        ));
        if name_rect.right() + 10.0 + tg.size().x + 10.0 <= cell.right() - PAD {
            familia::pintar_etiqueta(
                &p,
                pos2(name_rect.right() + 10.0, name_rect.center().y + 1.0),
                &tag,
                paleta().texto_fraco,
                9.5,
            );
        }
        p.galley(
            pos2(text_x, name_rect.bottom() + 3.0),
            p.layout_job(familia::linha(
                &item.subtitle,
                familia::sans(13.5),
                paleta().texto_fraco,
                name_w,
            )),
            Color32::WHITE,
        );

        // Status badges and description.
        let b = familia::pintar_etiqueta(
            &p,
            pos2(left, by),
            item.status.label(),
            status_color(&item.status),
            10.5,
        );
        if item.open {
            familia::pintar_etiqueta(
                &p,
                pos2(b.right() + 8.0, by),
                tx("janela-aberto_para_todos"),
                paleta().verde,
                10.5,
            );
        }
        p.galley(
            pos2(left, by + 18.0),
            p.layout_job(familia::paragrafo(
                &item.description,
                familia::sans(13.5),
                paleta().texto,
                inner_w,
                2,
            )),
            Color32::WHITE,
        );

        // Versions: installed, newest and its date, with the release notes on the right.
        let bh = 34.0;
        let row = Rect::from_min_size(pos2(left, cell.bottom() - PAD - bh), vec2(inner_w, bh));
        let vy = row.top() - 22.0;
        let ly = vy - 17.0;
        p.line_segment(
            [pos2(left, ly - 13.0), pos2(cell.right() - PAD, ly - 13.0)],
            Stroke::new(1.0, paleta().linha),
        );
        let col = inner_w / 3.6;
        let newest = item.latest.as_ref().map(|r| r.version.clone());
        let columns = [
            (
                tx("janela-instalada"),
                item.active.clone().unwrap_or_else(|| "—".into()),
                if item.active.is_some() {
                    paleta().texto
                } else {
                    paleta().texto_apagado
                },
            ),
            (
                tx("janela-mais_nova"),
                newest.unwrap_or_else(|| "—".into()),
                if item.has_update() {
                    paleta().amarelo
                } else {
                    acc
                },
            ),
            (
                tx("janela-publicada"),
                item.release_date().unwrap_or_else(|| "—".into()),
                paleta().texto_fraco,
            ),
        ];
        for (k, (label, value, color)) in columns.into_iter().enumerate() {
            let x = left + k as f32 * col;
            let w = if k == 2 {
                cell.right() - PAD - x
            } else {
                col - 8.0
            };
            // Long labels (e.g. "PLUS RÉCENTE") tighten, then cut, so they never touch the
            // next column.
            let label = label.to_uppercase();
            let mut job =
                familia::espacado(&label, familia::mono(9.5), paleta().texto_apagado, 1.4);
            if p.layout_job(job.clone()).size().x > w {
                job = familia::espacado(&label, familia::mono(8.5), paleta().texto_apagado, 0.6);
                job.wrap = egui::text::TextWrapping::truncate_at_width(w.max(1.0));
            }
            familia::pintar_texto(&p, pos2(x, ly), job);
            let font = if k == 2 {
                familia::sans(12.5)
            } else {
                familia::mono(12.5)
            };
            let g = p.layout_job(familia::linha(&value, font, color, w));
            p.galley(pos2(x, vy - g.size().y / 2.0), g, Color32::WHITE);
        }
        if item.latest.is_some() {
            let g = p.layout_job(familia::espacado(
                tx("janela-notas"),
                familia::mono(10.5),
                paleta().texto_fraco,
                1.6,
            ));
            let r = Rect::from_min_size(
                pos2(cell.right() - PAD - g.size().x, ly - g.size().y / 2.0),
                g.size(),
            )
            .expand(4.0);
            let resp = ui
                .allocate_rect(r, Sense::click())
                .on_hover_text(tx("janela-notas_da_versao"));
            let color = if resp.hovered() {
                paleta().branco
            } else {
                paleta().texto_fraco
            };
            p.galley(r.min + vec2(4.0, 4.0), g, color);
            if resp.clicked() {
                self.notes = Some(item.id.clone());
            }
        }

        // Buttons.
        let more = Rect::from_min_size(pos2(row.right() - bh, row.top()), Vec2::splat(bh));
        let main = Rect::from_min_max(row.min, pos2(more.left() - 8.0, row.bottom()));
        self.actions(ui, item, main, more);
    }

    /// The main button and the "…" menu of a product (card or row).
    fn actions(&mut self, ui: &mut Ui, item: &Item, main: Rect, more: Rect) {
        let mine = self
            .task
            .as_ref()
            .is_some_and(|t| t.product.as_deref() == Some(item.id.as_str()));
        let busy = self.busy();
        if mine {
            let task = self.task.clone();
            let frac = task.as_ref().and_then(|t| t.fraction());
            let label = match frac {
                Some(f) => fx(
                    "janela-cancelar-2",
                    &[("pct", dx(format!("{:.0}", f * 100.0)))],
                ),
                None => tx("janela-cancelar").into(),
            };
            if familia::botao_em(ui, main, &label, Botao::Contorno, true).clicked() {
                if let Some(t) = &task {
                    t.cancel.store(true, Ordering::Relaxed);
                }
                self.queue.clear();
            }
            progress_bar(
                ui,
                Rect::from_min_size(
                    pos2(main.left(), main.bottom() + 6.0),
                    vec2(main.width(), 2.0),
                ),
                frac,
            );
        } else {
            let (label, kind, enabled) = match &item.status {
                Status::Update { to, .. } => (
                    fx("janela-atualizar_para", &[("to", vx(&to))]),
                    Botao::Primario,
                    !busy,
                ),
                Status::Installed { .. } => (tx("janela-abrir").to_string(), Botao::Contorno, true),
                Status::NotInstalled => (tx("janela-instalar").to_string(), Botao::Primario, !busy),
                Status::NoAccess => (
                    tx("janela-inserir_codigo").to_string(),
                    Botao::Contorno,
                    !busy,
                ),
                Status::NoPackage => (
                    tx("janela-indisponivel").to_string(),
                    Botao::Contorno,
                    false,
                ),
            };
            // A long version does not fit a narrow button: "Atualizar" alone then.
            let label = if fits(ui, &label, main.width() - 16.0) {
                label
            } else if matches!(item.status, Status::Update { .. }) {
                tx("janela-atualizar").to_string()
            } else {
                label
            };
            if familia::botao_em(ui, main, &label, kind, enabled).clicked() {
                match &item.status {
                    Status::Update { to, .. } => {
                        self.start_install(ui.ctx(), &item.id, &to.clone())
                    }
                    Status::NotInstalled => {
                        if let Some(v) = item.latest.as_ref().map(|r| r.version.clone()) {
                            self.start_install(ui.ctx(), &item.id, &v);
                        }
                    }
                    Status::Installed { .. } => self.launch(&item.id, &item.name),
                    Status::NoAccess => self.editing_code = true,
                    Status::NoPackage => {}
                }
            }
        }
        let resp = familia::botao_icone_em(ui, more, Icone::Reticencias, false, true)
            .on_hover_text(tx("janela-mais_opcoes"));
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.product_menu(ui, item));
    }

    fn launch(&mut self, id: &str, name: &str) {
        match install::launch(&self.env.layout, id) {
            Ok(_) => self.set_status(fx("janela-aberto", &[("name", vx(&name))]), false),
            Err(e) => self.set_status(e.to_string(), true),
        }
    }

    /// Draws the "…" menu of product `id` into `ui` (its pictures in the tests).
    pub fn product_menu_of(&mut self, ui: &mut Ui, id: &str) {
        if let Some(item) = self.items().into_iter().find(|i| i.id == id) {
            self.product_menu(ui, &item);
        }
    }

    fn product_menu(&mut self, ui: &mut Ui, item: &Item) {
        ui.set_min_width(260.0);
        let busy = self.busy();
        let layout = self.env.layout.clone();
        let platform = self.env.platform.clone();
        let code = self.code();
        let Some(product) = self
            .manifest
            .as_ref()
            .and_then(|m| m.product(&item.id))
            .cloned()
        else {
            return;
        };
        if ui.button(tx("janela-notas_da_versao")).clicked() {
            self.notes = Some(item.id.clone());
            ui.close();
        }
        if item.active.is_some()
            && ui
                .button(tx("janela-abrir_a_pasta_do_aplicativo"))
                .clicked()
        {
            open_folder(&layout.product(&item.id));
            ui.close();
        }
        ui.separator();
        familia::rotulo(ui, tx("janela-canal"), 10.5, paleta().texto_apagado);
        let mut channels = product.channels();
        for c in [manifest::STABLE, manifest::TESTING] {
            if !channels.iter().any(|x| x == c) {
                channels.push(c.to_string());
            }
        }
        let mut channel = item.channel.clone();
        for c in &channels {
            ui.selectable_value(&mut channel, c.clone(), channel_name(c));
        }
        if channel != item.channel {
            self.state.channels.insert(item.id.clone(), channel.clone());
            self.save_state();
        }
        let installed = install::installed_versions(&layout, &item.id);
        // Removing and uninstalling come before the versions, so they stay in reach however
        // many versions there are.
        if item.active.is_some() {
            ui.separator();
            let olds = installed.len().saturating_sub(1);
            if olds > 0
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(fx(
                            "janela-remover_versoes_antigas",
                            &[("olds", vx(&olds))],
                        )),
                    )
                    .clicked()
            {
                match install::remove_old(&layout, &item.id, 0) {
                    Ok(v) => {
                        self.set_status(fx("janela-removidas", &[("v", vx(&v.join(", ")))]), false)
                    }
                    Err(e) => self.set_status(e.to_string(), true),
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(
                        egui::RichText::new(tx("janela-desinstalar")).color(paleta().erro),
                    ),
                )
                .clicked()
            {
                self.confirm_uninstall = Some(item.id.clone());
                ui.close();
            }
        }
        ui.separator();
        familia::rotulo(ui, tx("janela-versoes"), 10.5, paleta().texto_apagado);
        let releases = product.visible(&channel);
        let ctx = MenuVersions {
            item,
            installed: &installed,
            platform: &platform,
            code: code.as_ref(),
            busy,
        };
        for r in releases.iter().take(MENU_VERSIONS) {
            self.version_button(ui, &ctx, r);
        }
        let more = releases.len().saturating_sub(MENU_VERSIONS);
        if more > 0 {
            ui.menu_button(fx("janela-mais_versoes", &[("more", vx(&more))]), |ui| {
                ui.set_min_width(260.0);
                egui::ScrollArea::vertical()
                    .max_height(MENU_MORE_H)
                    .show(ui, |ui| {
                        for r in releases.iter().skip(MENU_VERSIONS) {
                            self.version_button(ui, &ctx, r);
                        }
                    });
            });
        }
    }

    /// One version in the "…" menu: active, use it, or install it.
    fn version_button(&mut self, ui: &mut Ui, m: &MenuVersions, r: &Release) {
        let layout = self.env.layout.clone();
        let item = m.item;
        let is_active = item.active.as_deref() == Some(r.version.as_str());
        let is_installed = m.installed.contains(&r.version);
        let pkg = r.package(m.platform);
        let can = pkg.is_some_and(|p| crate::can_install(p, m.code));
        let tag = if r.channel != manifest::STABLE {
            format!(" · {}", channel_name(&r.channel))
        } else {
            String::new()
        };
        if is_active {
            ui.add_enabled(
                false,
                egui::Button::new(fx(
                    "janela-versao_ativa",
                    &[("versao", sx(format!("{}{tag}", r.version)))],
                )),
            );
        } else if is_installed {
            if ui
                .add_enabled(
                    !m.busy,
                    egui::Button::new(fx(
                        "janela-usar_a_versao",
                        &[("versao", sx(format!("{}{tag}", r.version)))],
                    )),
                )
                .clicked()
            {
                match install::activate(&layout, &item.id, &r.version) {
                    Ok(()) => self.set_status(
                        fx(
                            "janela-ativa",
                            &[("name", vx(&item.name)), ("version", vx(&r.version))],
                        ),
                        false,
                    ),
                    Err(e) => self.set_status(e.to_string(), true),
                }
                ui.close();
            }
        } else {
            let why = if pkg.is_none() {
                tx("janela-sem_pacote_para_este_sistema")
            } else if !can {
                tx("janela-sem_acesso")
            } else {
                ""
            };
            if ui
                .add_enabled(
                    !m.busy && can,
                    egui::Button::new(fx(
                        "janela-instalar_a_versao",
                        &[
                            ("versao", sx(format!("{}{tag}", r.version))),
                            ("motivo", sx(why.to_string())),
                        ],
                    )),
                )
                .clicked()
            {
                let v = r.version.clone();
                self.start_install(ui.ctx(), &item.id, &v);
                ui.close();
            }
        }
    }

    fn list(&mut self, ui: &mut Ui, items: &[Item], icon_alpha: f32) {
        let w = ui.available_width();
        let head_h = 46.0;
        let (rect, _) = ui.allocate_exact_size(
            vec2(w, head_h + items.len() as f32 * ROW_H + PAD),
            Sense::hover(),
        );
        let table = Rect::from_min_size(rect.min, vec2(w, head_h + items.len() as f32 * ROW_H));
        let p = ui.painter().clone();
        p.rect_stroke(
            table,
            0.0,
            Stroke::new(1.0, paleta().linha),
            egui::StrokeKind::Inside,
        );
        let cols = Columns::new(table);
        let hy = table.top() + head_h / 2.0;
        let label = |x: f32, t: &str| {
            familia::pintar_texto(
                &p,
                pos2(x, hy),
                familia::espacado(t, familia::mono(10.5), paleta().texto_apagado, 2.2),
            );
        };
        label(cols.app, tx("janela-aplicativo"));
        if let Some(x) = cols.status {
            label(x, tx("janela-status"));
        }
        label(cols.version, tx("janela-versao"));
        if let Some(x) = cols.date {
            label(x, tx("janela-lancamento"));
        }
        if let Some(x) = cols.author {
            label(x, tx("janela-autor"));
        }
        p.line_segment(
            [
                pos2(table.left(), table.top() + head_h),
                pos2(table.right(), table.top() + head_h),
            ],
            Stroke::new(1.0, paleta().linha),
        );
        for (k, item) in items.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(table.left(), table.top() + head_h + k as f32 * ROW_H),
                vec2(w, ROW_H),
            );
            if k + 1 < items.len() {
                p.line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, paleta().linha),
                );
            }
            ui.push_id(&item.id, |ui| self.row(ui, row, &cols, item, icon_alpha));
        }
    }

    fn row(&mut self, ui: &mut Ui, row: Rect, cols: &Columns, item: &Item, icon_alpha: f32) {
        let p = ui.painter().clone();
        let cy = row.center().y;
        let icon = Rect::from_center_size(pos2(cols.app + 19.0, cy), Vec2::splat(38.0));
        self.icon(ui, icon, item, icon_alpha);
        let tx = icon.right() + 14.0;
        let name_w = cols.status.unwrap_or(cols.version) - tx - 16.0;
        let (g, cut) = fit_name(&p, item, &NAME_ROW, name_w, f32::INFINITY);
        let name_rect = Rect::from_min_size(pos2(tx, cy - g.size().y + 1.0), g.size());
        p.galley(name_rect.min, g, Color32::WHITE);
        name_tooltip(ui, name_rect, item, cut);
        p.galley(
            pos2(tx, cy + 2.0),
            p.layout_job(familia::linha(
                &item.subtitle,
                familia::sans(12.5),
                paleta().texto_fraco,
                name_w,
            )),
            Color32::WHITE,
        );
        if let Some(x) = cols.status {
            familia::pintar_etiqueta(
                &p,
                pos2(x, cy),
                item.status.label(),
                status_color(&item.status),
                10.0,
            );
        }
        let acc = accent(item);
        let (old, new) = item.versions();
        let mut job = egui::text::LayoutJob::default();
        let fmt = |c: Color32| egui::TextFormat {
            font_id: familia::mono(13.0),
            color: c,
            ..Default::default()
        };
        match (&old, &new) {
            (Some(o), Some(n)) => {
                job.append(o, 0.0, fmt(paleta().texto));
                job.append(" → ", 0.0, fmt(paleta().texto_fraco));
                job.append(n, 0.0, fmt(acc));
            }
            (None, Some(n)) => job.append(n, 0.0, fmt(paleta().texto)),
            _ => job.append("—", 0.0, fmt(paleta().texto_apagado)),
        }
        familia::pintar_texto(&p, pos2(cols.version, cy), job);
        if let Some(x) = cols.date {
            let d = item.release_date().unwrap_or_else(|| "—".into());
            familia::pintar_texto(
                &p,
                pos2(x, cy),
                familia::linha(&d, familia::sans(13.0), paleta().texto_fraco, 150.0),
            );
        }
        if let Some(x) = cols.author {
            familia::pintar_texto(
                &p,
                pos2(x, cy),
                familia::linha(
                    &item.author,
                    familia::sans(13.0),
                    paleta().texto_fraco,
                    110.0,
                ),
            );
        }
        let bh = 34.0;
        let more = Rect::from_min_size(
            pos2(row.right() - 16.0 - bh, cy - bh / 2.0),
            Vec2::splat(bh),
        );
        let main = Rect::from_min_max(
            pos2(cols.actions, more.top()),
            pos2(more.left() - 8.0, more.bottom()),
        );
        self.actions(ui, item, main, more);
    }

    fn footer(&mut self, ui: &mut Ui) {
        let r = ui.max_rect();
        let p = ui.painter().clone();
        p.line_segment(
            [
                r.left_top() + vec2(0.0, 0.5),
                r.right_top() + vec2(0.0, 0.5),
            ],
            Stroke::new(1.0, paleta().linha),
        );
        let left = familia::pintar_texto(
            &p,
            pos2(r.left() + PAD, r.center().y),
            familia::espacado(
                &self.check_line(),
                familia::mono(10.5),
                paleta().texto_apagado,
                1.8,
            ),
        );
        let mut right = r.right() - PAD;
        if let Some(task) = self.task.clone() {
            let cancel_g = p.layout_job(familia::espacado(
                tx("janela-cancelar-3"),
                familia::mono_negrito(10.5),
                paleta().texto,
                1.6,
            ));
            let can_cancel = task.product.is_some();
            if can_cancel {
                let cr = Rect::from_min_size(
                    pos2(
                        right - cancel_g.size().x,
                        r.center().y - cancel_g.size().y / 2.0,
                    ),
                    cancel_g.size(),
                )
                .expand(4.0);
                let resp = ui.allocate_rect(cr, Sense::click());
                let color = if resp.hovered() {
                    paleta().branco
                } else {
                    paleta().texto
                };
                p.galley(cr.min + vec2(4.0, 4.0), cancel_g, color);
                if resp.clicked() {
                    task.cancel.store(true, Ordering::Relaxed);
                    self.queue.clear();
                }
                right = cr.left() - 14.0;
            }
            let frac = task.fraction();
            let bar =
                Rect::from_min_size(pos2(right - 160.0, r.center().y - 1.0), vec2(160.0, 2.0));
            progress_bar(ui, bar, frac);
            ui.ctx().request_repaint();
            let text = match (frac, task.progress.lock().ok().and_then(|p| *p)) {
                (Some(_), Some(pr)) => fx(
                    "janela-progresso",
                    &[
                        ("tarefa", sx(task.label.clone())),
                        ("feito", sx(bytes(pr.done))),
                        ("total", sx(pr.total.map_or("?".into(), bytes))),
                    ],
                ),
                _ => format!("{}...", task.label.trim_end_matches("...")),
            };
            let g = p.layout_job(familia::linha(
                &text,
                familia::sans(12.0),
                paleta().texto_fraco,
                (bar.left() - left.right() - 40.0).max(40.0),
            ));
            right = bar.left() - 12.0;
            p.galley(
                pos2(right - g.size().x, r.center().y - g.size().y / 2.0),
                g,
                Color32::WHITE,
            );
        } else if !self.status.is_empty() {
            let color = if self.status_is_error {
                paleta().erro
            } else {
                paleta().texto_fraco
            };
            let g = p.layout_job(familia::linha(
                &self.status,
                familia::sans(12.0),
                color,
                (right - left.right() - 40.0).max(40.0),
            ));
            let gr = Rect::from_min_size(
                pos2(right - g.size().x, r.center().y - g.size().y / 2.0),
                g.size(),
            );
            p.galley(gr.min, g, Color32::WHITE);
            let full = self.status.clone();
            ui.allocate_rect(gr, Sense::hover()).on_hover_text(full);
        }
    }

    /// "VERIFICADO HÁ 10 MIN · VIA DRIVE" and its variants.
    pub fn check_line(&self) -> String {
        let url = self.env.manifest_url(&self.state).trim().to_string();
        let via = if url.is_empty() {
            tx("janela-sem_endereco_da_lista")
        } else if url.starts_with("file:") {
            tx("janela-via_pasta_local")
        } else if url.contains("google.com") {
            tx("janela-via_drive")
        } else {
            tx("janela-via_rede")
        };
        let when = if self.task.as_ref().is_some_and(|t| t.is_manifest()) {
            tx("janela-verificando").to_string()
        } else if self.offline {
            tx("janela-lista_guardada_sem_conexao").to_string()
        } else if self.state.checked_at == 0 {
            tx("janela-nunca_verificado").to_string()
        } else {
            let min = state::now_secs().saturating_sub(self.state.checked_at) / 60;
            match min {
                0 => tx("janela-verificado_agora").to_string(),
                m if m < 120 => fx("janela-verificado_ha_min", &[("m", vx(&m))]),
                m => fx("janela-verificado_ha_h", &[("h", vx(&(m / 60)))]),
            }
        };
        format!("{when} · {via}")
    }

    fn modals(&mut self, ctx: &Context) {
        self.settings(ctx);
        if let Some(id) = self.notes.clone() {
            let product = self.manifest.as_ref().and_then(|m| m.product(&id)).cloned();
            let Some(product) = product else {
                self.notes = None;
                return;
            };
            let code = self.code();
            let item = catalogo::item(
                &product,
                &self.env.layout,
                &self.state,
                &self.env.platform,
                code.as_ref(),
            );
            let modal = egui::Modal::new(Id::new("notas"))
                .frame(
                    Frame::NONE
                        .fill(paleta().superficie)
                        .stroke(Stroke::new(1.0, paleta().linha_forte))
                        .inner_margin(Margin::same(22)),
                )
                .show(ctx, |ui| {
                    ui.set_width(520.0_f32.min(ctx.content_rect().width() - 80.0));
                    let (g, _) = fit_name(
                        ui.painter(),
                        &item,
                        &NAME_NOTES,
                        ui.available_width(),
                        f32::INFINITY,
                    );
                    ui.label(g);
                    familia::rotulo(
                        ui,
                        tx("janela-notas_da_versao"),
                        10.5,
                        paleta().texto_apagado,
                    );
                    ui.add_space(6.0);
                    let mut close = false;
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .show(ui, |ui| {
                            let visible = product.visible(&item.channel);
                            if visible.is_empty() {
                                ui.label(
                                    egui::RichText::new(tx("janela-nenhuma_versao_neste_canal"))
                                        .color(paleta().texto_fraco),
                                );
                            }
                            for r in visible {
                                let date = catalogo::date_text(&r.published_at).unwrap_or_default();
                                ui.label(familia::espacado(
                                    &format!(
                                        "{}  {}  {}",
                                        r.version,
                                        channel_name(&r.channel).to_uppercase(),
                                        date.to_uppercase()
                                    ),
                                    familia::mono(11.5),
                                    accent(&item),
                                    1.2,
                                ));
                                let notes = if r.notes.trim().is_empty() {
                                    tx("janela-sem_notas")
                                } else {
                                    r.notes.trim()
                                };
                                ui.label(egui::RichText::new(notes).color(paleta().texto));
                                ui.add_space(10.0);
                            }
                        });
                    ui.add_space(8.0);
                    if familia::botao(
                        ui,
                        tx("janela-fechar"),
                        Botao::Contorno,
                        vec2(120.0, 32.0),
                        true,
                    )
                    .clicked()
                    {
                        close = true;
                    }
                    close
                });
            if modal.inner || modal.should_close() {
                self.notes = None;
            }
        }
        if let Some(id) = self.confirm_uninstall.clone() {
            let name = self
                .manifest
                .as_ref()
                .and_then(|m| m.product(&id))
                .map_or(id.clone(), |p| p.name.clone());
            let modal = egui::Modal::new(Id::new("desinstalar"))
                .frame(
                    Frame::NONE
                        .fill(paleta().superficie)
                        .stroke(Stroke::new(1.0, paleta().linha_forte))
                        .inner_margin(Margin::same(22)),
                )
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(fx(
                            "janela-remover_deste_computador",
                            &[("name", vx(&name))],
                        ))
                        .size(16.0)
                        .color(paleta().branco),
                    );
                    ui.label(
                        egui::RichText::new(tx("janela-todas_as_versoes_instaladas_e"))
                            .color(paleta().texto_fraco),
                    );
                    ui.add_space(10.0);
                    let mut answer = None;
                    ui.horizontal(|ui| {
                        if familia::botao(
                            ui,
                            tx("janela-desinstalar-2"),
                            Botao::Perigo,
                            vec2(150.0, 32.0),
                            true,
                        )
                        .clicked()
                        {
                            answer = Some(true);
                        }
                        if familia::botao(
                            ui,
                            tx("janela-cancelar"),
                            Botao::Contorno,
                            vec2(120.0, 32.0),
                            true,
                        )
                        .clicked()
                        {
                            answer = Some(false);
                        }
                    });
                    answer
                });
            match modal.inner {
                Some(true) => {
                    self.confirm_uninstall = None;
                    match crate::uninstall_product(&self.env, &id, &name) {
                        Ok(()) => self
                            .set_status(fx("janela-desinstalado", &[("name", vx(&name))]), false),
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                }
                Some(false) => self.confirm_uninstall = None,
                None if modal.should_close() => self.confirm_uninstall = None,
                None => {}
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        utsim_ui_tema::maximizar_no_inicio(ui.ctx());
        self.draw(ui);
    }
}

/// Column positions of the list.
struct Columns {
    app: f32,
    status: Option<f32>,
    version: f32,
    date: Option<f32>,
    author: Option<f32>,
    actions: f32,
}

impl Columns {
    /// The columns that fit: every one from 1000 points, without release date and author
    /// from 820, and without the status (the button says it) below that.
    fn new(table: Rect) -> Self {
        let w = table.width();
        let button = if w >= 820.0 { 230.0 } else { 170.0 };
        let actions = table.right() - 16.0 - 34.0 - 8.0 - button;
        let (date, author, version) = if w >= 1000.0 {
            let author = actions - 24.0 - 110.0;
            let date = author - 150.0;
            (Some(date), Some(author), date - 170.0)
        } else {
            (None, None, actions - 24.0 - 150.0)
        };
        Self {
            app: table.left() + 16.0,
            status: (w >= 820.0).then_some(version - 210.0),
            version,
            date,
            author,
            actions,
        }
    }
}

/// Whether a button text fits `width`.
fn fits(ui: &Ui, text: &str, width: f32) -> bool {
    let job = familia::espacado(
        &text.to_uppercase(),
        familia::mono_negrito(12.0),
        Color32::WHITE,
        1.6,
    );
    ui.painter().layout_job(job).size().x <= width
}

/// The product's accent, darkened on the light palette so it still reads as text.
fn accent(item: &Item) -> Color32 {
    let [r, g, b] = item.color;
    paleta().legivel(Color32::from_rgb(r, g, b))
}

fn status_color(s: &Status) -> Color32 {
    match s {
        Status::Update { .. } => paleta().amarelo,
        Status::Installed { .. } => paleta().texto_fraco,
        Status::NotInstalled => paleta().texto,
        Status::NoAccess | Status::NoPackage => paleta().texto_apagado,
    }
}

/// The name in two tones: the first part white, the second in the product's accent.
/// How a product name may be fitted: the sizes to try, largest first, and the most lines.
struct NameFit {
    sizes: &'static [f32],
    rows: usize,
}

const NAME_CARD: NameFit = NameFit {
    sizes: &[21.0, 19.5, 18.0, 16.5],
    rows: 2,
};
const NAME_ROW: NameFit = NameFit {
    sizes: &[16.0, 15.0, 14.0],
    rows: 1,
};
const NAME_NOTES: NameFit = NameFit {
    sizes: &[20.0, 18.0],
    rows: 2,
};

/// The name laid out in `width`: the largest size whose lines fit `fit.rows` and
/// `max_h`; else the smallest, cut with "…" (then `true`).
fn fit_name(
    p: &egui::Painter,
    item: &Item,
    fit: &NameFit,
    width: f32,
    max_h: f32,
) -> (Arc<egui::Galley>, bool) {
    let lay = |size: f32| {
        let mut job = name_job(item, size);
        job.wrap.max_width = width.max(1.0);
        job.wrap.max_rows = fit.rows;
        p.layout_job(job)
    };
    for &size in fit.sizes {
        let g = lay(size);
        if !g.elided && g.size().y <= max_h && g.size().x <= width + 0.5 {
            return (g, false);
        }
    }
    let g = lay(fit.sizes.last().copied().unwrap_or(14.0));
    let cut = g.elided || g.size().x > width + 0.5;
    (g, cut)
}

/// The whole name on hover when it was cut.
fn name_tooltip(ui: &Ui, rect: Rect, item: &Item, cut: bool) {
    if cut {
        ui.interact(rect, ui.id().with("nome"), Sense::hover())
            .on_hover_text(&item.name);
    }
}

fn name_job(item: &Item, size: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c: Color32| egui::TextFormat {
        font_id: familia::negrito(size),
        color: c,
        ..Default::default()
    };
    let (a, b) = &item.name_parts;
    if !a.is_empty() {
        job.append(a, 0.0, fmt(paleta().branco));
    }
    if !b.is_empty() {
        let lead = if a.is_empty() || item.name.trim() == "UTSim" {
            0.0
        } else {
            size * 0.25
        };
        job.append(b, lead, fmt(accent(item)));
    }
    job
}

/// A thin bar: filled to `frac`, or a moving segment when the size is unknown.
fn progress_bar(ui: &Ui, rect: Rect, frac: Option<f32>) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, paleta().linha);
    let fill = match frac {
        Some(f) => Rect::from_min_size(
            rect.min,
            vec2(rect.width() * f.clamp(0.0, 1.0), rect.height()),
        ),
        None => {
            let t = (ui.input(|i| i.time) * 0.8).fract() as f32;
            let w = rect.width() * 0.25;
            let x = rect.left() + (rect.width() + w) * t - w;
            Rect::from_min_max(
                pos2(x.max(rect.left()), rect.top()),
                pos2((x + w).min(rect.right()), rect.bottom()),
            )
        }
    };
    p.rect_filled(fill, 0.0, paleta().branco);
}

fn open_folder(dir: &std::path::Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

/// Puts the window in the saved language; with none, the current one stays (the system's,
/// set by `main`).
pub fn apply_language(state: &LauncherState) {
    if let Some(i) = state
        .language
        .as_deref()
        .and_then(utsim_idiomas::Idioma::de_codigo)
    {
        utsim_idiomas::definir(i);
    }
}

/// The name of an order in the menu.
fn sort_label(s: SortOrder) -> &'static str {
    match s {
        SortOrder::Name => tx("janela-ordem_nome"),
        SortOrder::Recent => tx("janela-ordem_recentes"),
        SortOrder::Updates => tx("janela-ordem_atualizacao"),
    }
}

fn channel_name(c: &str) -> String {
    match c {
        manifest::STABLE => tx("janela-canal_estavel").into(),
        manifest::TESTING => tx("janela-canal_testes").into(),
        other => other.into(),
    }
}

fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n as f64 >= MB {
        utsim_idiomas::marca_decimal(&format!("{:.1} MB", n as f64 / MB))
    } else {
        format!("{:.0} kB", n as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use utsim_launcher_core::install::Layout;
    use utsim_launcher_core::manifest::Product;

    fn item(name: &str) -> Item {
        let product = Product {
            id: "x".into(),
            name: name.into(),
            ..Product::default()
        };
        let layout = Layout::new(std::env::temp_dir().join("utsim-sem-nada"));
        catalogo::item(
            &product,
            &layout,
            &LauncherState::default(),
            "windows-x64",
            None,
        )
    }

    /// The narrowest card: the name always stays inside it.
    #[test]
    fn long_names_stay_inside_the_card() {
        let ctx = Context::default();
        familia::install(&ctx);
        let name_w = CARD_MIN_W - 2.0 * PAD - 56.0 - 14.0;
        let max_h = 56.0 + 22.0 - 26.0 - 4.0;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let p = ui.painter();
            for (name, cut) in [
                ("UTSim LSS", false),
                ("UTSim Produto", false),
                ("UTSim Produit", false),
                ("UTSim Prodotto", false),
                ("UTSim Producto", false),
                ("UTSim Product", false),
                (
                    "UTSim Gerenciamento Integrado de Manutenção Produtiva Total e Confiabilidade",
                    true,
                ),
                (
                    "UTSimNomeSemEspaçosQueNãoCabeDeJeitoNenhumNumaLinhaSó",
                    false,
                ),
            ] {
                let it = item(name);
                let (g, c) = fit_name(p, &it, &NAME_CARD, name_w, max_h);
                assert!(g.size().x <= name_w + 0.5, "{name}: {}", g.size().x);
                assert!(g.size().y <= max_h + 0.5, "{name}: {}", g.size().y);
                assert!(g.rows.len() <= 2, "{name}");
                if cut {
                    assert!(c, "{name}");
                }
                let (g, _) = fit_name(p, &it, &NAME_ROW, 160.0, f32::INFINITY);
                assert!(g.size().x <= 160.5 && g.rows.len() == 1, "{name}");
            }
        });
        out.textures_delta.clear();
    }
}

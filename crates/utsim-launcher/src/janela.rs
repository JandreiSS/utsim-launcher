//! The launcher window: a top bar with the wordmark, a toolbar (refresh, update all, order,
//! search, grid or list), the products as a grid of cards or as a table, and a footer with
//! the last check and the progress of a download. Flat and near-black (see
//! [`utsim_ui_tema::familia`]). Long work (manifest, download, report) runs on other threads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Color32, Context, Frame, Id, LayerId, Margin, Order, Rect, Sense, Stroke, TextureHandle,
    TextureId, Ui, UiBuilder, Vec2, pos2, vec2,
};
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::download::Progress;
use utsim_launcher_core::install;
use utsim_launcher_core::manifest::{self, Manifest, Product, Release};
use utsim_launcher_core::state::{self, LauncherState, SortOrder, ViewMode};
use utsim_ui_tema::familia::{self, Botao, Icone, cores};

use crate::Env;
use crate::abertura::{self, Intro, IntroState, Voo};
use crate::catalogo::{self, Item, Status};

/// Height of a card in the grid.
pub const CARD_H: f32 = 300.0;
/// Narrowest card; the grid has as many columns as fit.
pub const CARD_MIN_W: f32 = 330.0;
/// Height of a row in the list.
pub const ROW_H: f32 = 68.0;
const TOP_H: f32 = 48.0;
const FOOT_H: f32 = 30.0;
const PAD: f32 = 20.0;

/// What a background task hands back.
enum Outcome {
    Manifest(Result<(Manifest, LauncherState), String>),
    Installed(Result<String, String>),
    Message(Result<String, String>),
}

/// A task running on another thread.
pub struct Task {
    pub label: String,
    /// The product being installed, if any (its card shows the progress).
    pub product: Option<String>,
    pub progress: Mutex<Option<Progress>>,
    pub cancel: AtomicBool,
    outcome: Mutex<Option<Outcome>>,
}

impl Task {
    fn new(label: impl Into<String>, product: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
            product,
            progress: Mutex::new(None),
            cancel: AtomicBool::new(false),
            outcome: Mutex::new(None),
        })
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
        self.product.is_none() && self.label.starts_with("Buscando")
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
    /// Manifest address typed on the code screen (advanced).
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
}

impl App {
    /// The window as it opens: reads the state and the cached manifest and starts fetching
    /// the manifest at once (the animation, if any, never delays it).
    pub fn new(ctx: &Context, env: Arc<Env>, message: Option<String>, intro: bool) -> Self {
        install::clean_leftovers(&env.layout);
        let state = LauncherState::load(&env.layout.state_file());
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
        Self {
            offline: false,
            manifest,
            code_input: String::new(),
            url_input: state.manifest_url.clone().unwrap_or_default(),
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
            self.set_status(format!("não consegui gravar as preferências: {e}"), true);
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
        let task = Task::new("Buscando a lista de versões...", None);
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
            format!("Instalando {} {}", product.name, release.version),
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
            .map(|_| format!("{} {} instalado.", product.name, release.version))
            .map_err(|e| e.to_string());
            task.finish(Outcome::Installed(r));
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
        let task = Task::new("Gerando o relato de erro...", None);
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = crate::error_report(&env, &state, &crate::downloads_dir())
                .map(|p| {
                    format!(
                        "Relato salvo em {}. Envie este arquivo ao autor (ele não contém o seu código).",
                        p.display()
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
                    self.set_status("Lista de versões atualizada.", false);
                }
            }
            Outcome::Manifest(Err(e)) => {
                let extra = if self.manifest.is_some() {
                    " Mostrando a última lista baixada."
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
            Outcome::Installed(Err(e)) | Outcome::Message(Err(e)) => {
                self.queue.clear();
                if cancelled {
                    self.set_status(
                        "Download cancelado; ele continua de onde parou na próxima vez.",
                        false,
                    );
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
                .frame(Frame::NONE.fill(cores::FUNDO))
                .show(ui, |ui| self.code_screen(ui));
        } else {
            egui::Panel::top("topo")
                .exact_size(TOP_H)
                .frame(Frame::NONE.fill(cores::FUNDO))
                .show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("rodape")
                .exact_size(FOOT_H)
                .frame(Frame::NONE.fill(cores::FUNDO))
                .show(ui, |ui| self.footer(ui));
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(cores::FUNDO)
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
                ui.label(familia::espacado("UTSIM", familia::negrito(22.0), cores::BRANCO, 4.0));
                ui.add_space(12.0);
                ui.label(familia::espacado("FAMÍLIA DE APPS", familia::mono(11.0), cores::TEXTO_FRACO, 2.5));
            });
            ui.add_space(18.0);
            familia::rotulo(ui, "Código de acesso", 11.0, cores::TEXTO_APAGADO);
            ui.label(
                egui::RichText::new(
                    "Digite o código que você recebeu do autor. Ele tem 20 letras e números, como ABCD-EFGH-JKMN-PQRS-TVWX.",
                )
                .color(cores::TEXTO_FRACO),
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
                ok = familia::botao(ui, "Continuar", Botao::Primario, vec2(180.0, 36.0), true).clicked() || enter;
                if can_skip {
                    let label = if self.state.code.is_some() { "Cancelar" } else { "Agora não" };
                    if familia::botao(ui, label, Botao::Contorno, vec2(140.0, 36.0), true).clicked() {
                        self.editing_code = false;
                        self.code_error = None;
                    }
                }
            });
            if ok {
                self.accept_code(ui.ctx());
            }
            if let Some(e) = &self.code_error {
                ui.colored_label(cores::ERRO, e);
            }
            ui.add_space(12.0);
            let no_builtin = crate::built_in_manifest_url().is_empty();
            egui::CollapsingHeader::new(familia::espacado("AVANÇADO", familia::mono(11.0), cores::TEXTO_FRACO, 2.0))
                .default_open(no_builtin)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Endereço da lista de versões (só se o autor pedir):").color(cores::TEXTO_FRACO));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.url_input)
                            .hint_text("link do manifesto.json no Google Drive")
                            .desired_width(w - 20.0)
                            .margin(Margin::symmetric(10, 7)),
                    );
                });
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("O código fica guardado só neste computador, na sua pasta de usuário. Aplicativos abertos para todos não pedem código.")
                    .size(12.0)
                    .color(cores::TEXTO_APAGADO),
            );
        });
    }

    fn accept_code(&mut self, ctx: &Context) {
        let url = self.url_input.trim().to_string();
        let typed = self.code_input.trim().to_string();
        if typed.is_empty() {
            if url.is_empty() && self.env.manifest_url(&self.state).trim().is_empty() {
                self.code_error =
                    Some("Digite o código ou, em Avançado, o endereço da lista.".into());
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
        self.state.manifest_url = (!url.is_empty()).then_some(url);
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
            Stroke::new(1.0, cores::LINHA),
        );
        let mark = familia::pintar_texto(
            &p,
            pos2(r.left() + PAD, r.center().y),
            familia::espacado("UTSIM", familia::negrito(17.0), cores::BRANCO, 3.5),
        );
        let x = mark.right() + 22.0;
        p.line_segment(
            [pos2(x, r.top()), pos2(x, r.bottom() - 1.0)],
            Stroke::new(1.0, cores::LINHA),
        );
        familia::pintar_texto(
            &p,
            pos2(x + 16.0, r.center().y),
            familia::espacado(
                "FAMÍLIA DE APPS",
                familia::mono(11.0),
                cores::TEXTO_FRACO,
                2.6,
            ),
        );

        let gear = Rect::from_min_size(pos2(r.right() - TOP_H, r.top()), vec2(TOP_H, TOP_H - 1.0));
        p.line_segment(
            [gear.left_top(), gear.left_bottom()],
            Stroke::new(1.0, cores::LINHA),
        );
        let resp = ui
            .allocate_rect(gear, Sense::click())
            .on_hover_text("Configurações");
        if resp.hovered() {
            p.rect_filled(gear.shrink2(vec2(0.5, 0.0)), 0.0, cores::SOBRE);
        }
        familia::pintar_icone(
            &p,
            Rect::from_center_size(gear.center(), Vec2::splat(18.0)),
            Icone::Engrenagem,
            cores::TEXTO,
        );
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.settings_menu(ui));
    }

    fn settings_menu(&mut self, ui: &mut Ui) {
        ui.set_min_width(280.0);
        let code = self.code();
        familia::rotulo(ui, "Testador", 10.5, cores::TEXTO_APAGADO);
        ui.label(match &code {
            Some(c) => format!("Código de {}", c.id()),
            None => "Sem código (só aplicativos abertos)".into(),
        });
        let label = if code.is_some() {
            "Trocar código"
        } else {
            "Inserir código"
        };
        if ui
            .add_enabled(!self.busy(), egui::Button::new(label))
            .clicked()
        {
            self.editing_code = true;
            ui.close();
        }
        ui.separator();
        let mut desktop = self.state.desktop_shortcuts;
        if ui
            .checkbox(&mut desktop, "Criar atalho na área de trabalho ao instalar")
            .changed()
        {
            self.state.desktop_shortcuts = desktop;
            self.save_state();
        }
        let mut intro = !self.state.skip_intro;
        if ui
            .checkbox(&mut intro, "Mostrar animação de abertura")
            .changed()
        {
            self.state.skip_intro = !intro;
            self.save_state();
        }
        ui.separator();
        if ui
            .add_enabled(!self.busy(), egui::Button::new("Gerar relato de erro"))
            .on_hover_text("Cria um .zip com os registros do launcher para enviar ao autor")
            .clicked()
        {
            self.start_report(ui.ctx());
            ui.close();
        }
        ui.label(
            egui::RichText::new(format!("UTSim Launcher {}", env!("CARGO_PKG_VERSION")))
                .size(11.5)
                .color(cores::TEXTO_APAGADO),
        );
    }

    fn main_area(&mut self, ui: &mut Ui, icon_alpha: f32) {
        let items = self.items();
        self.toolbar(ui, &items);
        let body = ui.available_rect_before_wrap();
        let Some(m) = &self.manifest else {
            let busy = self.busy();
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(if busy {
                            "Buscando a lista de aplicativos..."
                        } else {
                            "Nenhuma lista de aplicativos ainda."
                        })
                        .size(15.0)
                        .color(cores::TEXTO_FRACO),
                    );
                    ui.add_space(12.0);
                    if !busy
                        && familia::botao(
                            ui,
                            "Atualizar lista",
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
                            "Inserir código",
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
            let text = if m.products.is_empty() {
                "A lista não tem aplicativos ainda.".to_string()
            } else {
                format!(
                    "Nenhum aplicativo encontrado para \u{201c}{}\u{201d}.",
                    self.search.trim()
                )
            };
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(15.0)
                            .color(cores::TEXTO_FRACO),
                    );
                });
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.state.view {
                ViewMode::Grid => self.grid(ui, &items, icon_alpha),
                ViewMode::List => self.list(ui, &items, icon_alpha),
            });
    }

    fn toolbar(&mut self, ui: &mut Ui, items: &[Item]) {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
        let h = 34.0;
        let y = bar.center().y - h / 2.0;
        let busy = self.busy();
        let refresh = Rect::from_min_size(pos2(bar.left(), y), Vec2::splat(h));
        if familia::botao_icone_em(ui, refresh, Icone::Atualizar, false, !busy)
            .on_hover_text("Buscar a lista de novo")
            .clicked()
        {
            self.refresh(ui.ctx());
        }
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
                "Grade"
            } else {
                "Lista"
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
        let sw = if narrow { 150.0 } else { 210.0 };
        x -= 8.0 + sw;
        let search = Rect::from_min_size(pos2(x, y), vec2(sw, h));
        ui.scope_builder(UiBuilder::new().max_rect(search), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Buscar aplicativos")
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
                .selected_text(sort.label())
                .show_ui(ui, |ui| {
                    for o in SortOrder::ALL {
                        ui.selectable_value(&mut sort, o, o.label());
                    }
                });
        });
        if sort != self.state.sort {
            self.state.sort = sort;
            self.save_state();
        }
        if !narrow {
            let job = familia::espacado("ORDENAR", familia::mono(10.5), cores::TEXTO_APAGADO, 2.0);
            let g = ui.painter().layout_job(job);
            x -= 10.0 + g.size().x;
            ui.painter().galley(
                pos2(x, bar.center().y - g.size().y / 2.0),
                g,
                Color32::WHITE,
            );
        }
        // Update all.
        let n = items.iter().filter(|i| i.has_update()).count();
        let label = format!("Atualizar todos ({n})");
        let bw = 190.0;
        x -= 12.0 + bw;
        if x > refresh.right() + 8.0 {
            let r = Rect::from_min_size(pos2(x, y), vec2(bw, h));
            if familia::botao_em(ui, r, &label, Botao::Primario, n > 0 && !busy).clicked() {
                self.update_all(ui.ctx());
            }
        }
    }

    fn grid(&mut self, ui: &mut Ui, items: &[Item], icon_alpha: f32) {
        let w = ui.available_width();
        let cols = ((w / CARD_MIN_W).floor() as usize).max(1);
        let cw = (w / cols as f32).floor();
        let rows = items.len().div_ceil(cols);
        let (rect, _) = ui.allocate_exact_size(vec2(w, rows as f32 * CARD_H + PAD), Sense::hover());
        for (k, item) in items.iter().enumerate() {
            let (row, col) = (k / cols, k % cols);
            let x0 = rect.left() + col as f32 * cw;
            let x1 = if col + 1 == cols {
                rect.right()
            } else {
                x0 + cw
            };
            let cell = Rect::from_min_max(
                pos2(x0, rect.top() + row as f32 * CARD_H),
                pos2(x1, rect.top() + (row + 1) as f32 * CARD_H),
            );
            // Hairlines shared by neighbours: each card draws its top and left edges, and its
            // right and bottom ones only where no card follows (one pixel everywhere).
            let p = ui.painter();
            let line = Stroke::new(1.0, cores::LINHA);
            let (l, t) = (cell.left() + 0.5, cell.top() + 0.5);
            let (r, b) = (cell.right() - 0.5, cell.bottom() - 0.5);
            p.line_segment([pos2(l, t), pos2(cell.right(), t)], line);
            p.line_segment([pos2(l, t), pos2(l, cell.bottom())], line);
            if col + 1 == cols || k + 1 == items.len() {
                p.line_segment([pos2(r, t), pos2(r, cell.bottom())], line);
            }
            if k + cols >= items.len() {
                p.line_segment([pos2(l, b), pos2(cell.right(), b)], line);
            }
            ui.push_id(&item.id, |ui| self.card(ui, cell, item, icon_alpha));
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
        // Header strip: versions and release notes.
        let head = Rect::from_min_size(cell.min, vec2(cell.width(), 40.0));
        p.line_segment(
            [
                head.left_bottom() + vec2(0.0, 0.5),
                head.right_bottom() + vec2(0.0, 0.5),
            ],
            Stroke::new(1.0, cores::LINHA),
        );
        let (old, new) = item.versions();
        let mut job = egui::text::LayoutJob::default();
        let fmt = |c: Color32| egui::TextFormat {
            font_id: familia::mono(12.0),
            color: c,
            extra_letter_spacing: 1.3,
            ..Default::default()
        };
        match (&old, &new) {
            (Some(o), Some(n)) => {
                job.append(o, 0.0, fmt(cores::TEXTO));
                job.append("  →  ", 0.0, fmt(cores::TEXTO_FRACO));
                job.append(n, 0.0, fmt(acc));
            }
            (None, Some(n)) => job.append(&format!("V{n}"), 0.0, fmt(acc)),
            _ => job.append("—", 0.0, fmt(cores::TEXTO_APAGADO)),
        }
        familia::pintar_texto(&p, pos2(head.left() + PAD, head.center().y), job);
        if item.latest.is_some() {
            let g = p.layout_job(familia::espacado(
                "NOTAS ↗",
                familia::mono(11.0),
                cores::TEXTO_FRACO,
                1.6,
            ));
            let r = Rect::from_min_size(
                pos2(
                    head.right() - PAD - g.size().x,
                    head.center().y - g.size().y / 2.0,
                ),
                g.size(),
            )
            .expand(4.0);
            let resp = ui
                .allocate_rect(r, Sense::click())
                .on_hover_text("Notas da versão");
            let color = if resp.hovered() {
                cores::BRANCO
            } else {
                cores::TEXTO_FRACO
            };
            p.galley(r.min + vec2(4.0, 4.0), g, color);
            if resp.clicked() {
                self.notes = Some(item.id.clone());
            }
        }

        // Body.
        let left = cell.left() + PAD;
        let inner_w = cell.width() - 2.0 * PAD;
        let icon = Rect::from_min_size(pos2(left, head.bottom() + PAD), Vec2::splat(56.0));
        self.icon(ui, icon, item, icon_alpha);
        let tx = icon.right() + 14.0;
        let name_w = cell.right() - PAD - tx;
        let name = name_job(item, 21.0);
        let g = p.layout_job(name);
        let name_rect = Rect::from_min_size(pos2(tx, icon.top() + 2.0), g.size());
        p.galley(name_rect.min, g, Color32::WHITE);
        let tag = format!("POR {}", item.author.to_uppercase());
        let tg = p.layout_job(familia::espacado(
            &tag,
            familia::mono(9.5),
            cores::TEXTO_FRACO,
            1.2,
        ));
        if name_rect.right() + 10.0 + tg.size().x + 10.0 <= cell.right() - PAD {
            familia::pintar_etiqueta(
                &p,
                pos2(name_rect.right() + 10.0, name_rect.center().y + 1.0),
                &tag,
                cores::TEXTO_FRACO,
                9.5,
            );
        }
        p.galley(
            pos2(tx, name_rect.bottom() + 2.0),
            p.layout_job(familia::linha(
                &item.subtitle,
                familia::sans(13.5),
                cores::TEXTO_FRACO,
                name_w,
            )),
            Color32::WHITE,
        );

        // Status badges.
        let by = icon.bottom() + 26.0;
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
                "Aberto para todos",
                cores::VERDE,
                10.5,
            );
        }
        // Description and date.
        let dg = p.layout_job(familia::paragrafo(
            &item.description,
            familia::sans(13.5),
            cores::TEXTO,
            inner_w,
            2,
        ));
        let dy = by + 20.0;
        let dh = dg.size().y;
        p.galley(pos2(left, dy), dg, Color32::WHITE);
        if let Some(d) = item.release_date() {
            p.galley(
                pos2(left, dy + dh + 8.0),
                p.layout_job(familia::linha(
                    &format!("Última atualização: {d}"),
                    familia::sans(12.5),
                    cores::TEXTO_APAGADO,
                    inner_w,
                )),
                Color32::WHITE,
            );
        }

        // Buttons.
        let bh = 34.0;
        let row = Rect::from_min_size(pos2(left, cell.bottom() - PAD - bh), vec2(inner_w, bh));
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
                Some(f) => format!("Cancelar · {:.0}%", f * 100.0),
                None => "Cancelar".into(),
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
                Status::Update { to, .. } => {
                    (format!("Atualizar para {to}"), Botao::Primario, !busy)
                }
                Status::Installed { .. } => ("Abrir".to_string(), Botao::Contorno, true),
                Status::NotInstalled => ("Instalar".to_string(), Botao::Primario, !busy),
                Status::NoAccess => ("Inserir código".to_string(), Botao::Contorno, !busy),
                Status::NoPackage => ("Indisponível".to_string(), Botao::Contorno, false),
            };
            // A long version does not fit a narrow button: "Atualizar" alone then.
            let label = if fits(ui, &label, main.width() - 16.0) {
                label
            } else if matches!(item.status, Status::Update { .. }) {
                "Atualizar".to_string()
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
            .on_hover_text("Mais opções");
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.product_menu(ui, item));
    }

    fn launch(&mut self, id: &str, name: &str) {
        match install::launch(&self.env.layout, id) {
            Ok(_) => self.set_status(format!("{name} aberto."), false),
            Err(e) => self.set_status(e.to_string(), true),
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
        if ui.button("Notas da versão").clicked() {
            self.notes = Some(item.id.clone());
            ui.close();
        }
        if item.active.is_some() && ui.button("Abrir a pasta do aplicativo").clicked() {
            open_folder(&layout.product(&item.id));
            ui.close();
        }
        ui.separator();
        familia::rotulo(ui, "Canal", 10.5, cores::TEXTO_APAGADO);
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
        ui.separator();
        familia::rotulo(ui, "Versões", 10.5, cores::TEXTO_APAGADO);
        let installed = install::installed_versions(&layout, &item.id);
        for r in product.visible(&channel) {
            let is_active = item.active.as_deref() == Some(r.version.as_str());
            let is_installed = installed.contains(&r.version);
            let pkg = r.package(&platform);
            let can = pkg.is_some_and(|p| crate::can_install(p, code.as_ref()));
            let tag = if r.channel != manifest::STABLE {
                format!(" · {}", channel_name(&r.channel))
            } else {
                String::new()
            };
            if is_active {
                ui.add_enabled(
                    false,
                    egui::Button::new(format!("{}{tag} (ativa)", r.version)),
                );
            } else if is_installed {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(format!("Usar a {}{tag}", r.version)),
                    )
                    .clicked()
                {
                    match install::activate(&layout, &item.id, &r.version) {
                        Ok(()) => {
                            self.set_status(format!("{} {} ativa.", item.name, r.version), false)
                        }
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                    ui.close();
                }
            } else {
                let why = if pkg.is_none() {
                    " (sem pacote para este sistema)"
                } else if !can {
                    " (sem acesso)"
                } else {
                    ""
                };
                if ui
                    .add_enabled(
                        !busy && can,
                        egui::Button::new(format!("Instalar a {}{tag}{why}", r.version)),
                    )
                    .clicked()
                {
                    let v = r.version.clone();
                    self.start_install(ui.ctx(), &item.id, &v);
                    ui.close();
                }
            }
        }
        if item.active.is_some() {
            ui.separator();
            let olds = installed.len().saturating_sub(1);
            if olds > 0
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(format!("Remover versões antigas ({olds})")),
                    )
                    .clicked()
            {
                match install::remove_old(&layout, &item.id, 0) {
                    Ok(v) => self.set_status(format!("Removidas: {}.", v.join(", ")), false),
                    Err(e) => self.set_status(e.to_string(), true),
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(egui::RichText::new("Desinstalar...").color(cores::ERRO)),
                )
                .clicked()
            {
                self.confirm_uninstall = Some(item.id.clone());
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
            Stroke::new(1.0, cores::LINHA),
            egui::StrokeKind::Inside,
        );
        let cols = Columns::new(table);
        let hy = table.top() + head_h / 2.0;
        let label = |x: f32, t: &str| {
            familia::pintar_texto(
                &p,
                pos2(x, hy),
                familia::espacado(t, familia::mono(10.5), cores::TEXTO_APAGADO, 2.2),
            );
        };
        label(cols.app, "APLICATIVO");
        if let Some(x) = cols.status {
            label(x, "STATUS");
        }
        label(cols.version, "VERSÃO");
        if let Some(x) = cols.date {
            label(x, "LANÇAMENTO");
        }
        if let Some(x) = cols.author {
            label(x, "AUTOR");
        }
        p.line_segment(
            [
                pos2(table.left(), table.top() + head_h),
                pos2(table.right(), table.top() + head_h),
            ],
            Stroke::new(1.0, cores::LINHA),
        );
        for (k, item) in items.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(table.left(), table.top() + head_h + k as f32 * ROW_H),
                vec2(w, ROW_H),
            );
            if k + 1 < items.len() {
                p.line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(1.0, cores::LINHA),
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
        let g = p.layout_job(name_job(item, 16.0));
        p.galley(pos2(tx, cy - g.size().y + 1.0), g, Color32::WHITE);
        p.galley(
            pos2(tx, cy + 2.0),
            p.layout_job(familia::linha(
                &item.subtitle,
                familia::sans(12.5),
                cores::TEXTO_FRACO,
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
                job.append(o, 0.0, fmt(cores::TEXTO));
                job.append(" → ", 0.0, fmt(cores::TEXTO_FRACO));
                job.append(n, 0.0, fmt(acc));
            }
            (None, Some(n)) => job.append(n, 0.0, fmt(cores::TEXTO)),
            _ => job.append("—", 0.0, fmt(cores::TEXTO_APAGADO)),
        }
        familia::pintar_texto(&p, pos2(cols.version, cy), job);
        if let Some(x) = cols.date {
            let d = item.release_date().unwrap_or_else(|| "—".into());
            familia::pintar_texto(
                &p,
                pos2(x, cy),
                familia::linha(&d, familia::sans(13.0), cores::TEXTO_FRACO, 150.0),
            );
        }
        if let Some(x) = cols.author {
            familia::pintar_texto(
                &p,
                pos2(x, cy),
                familia::linha(&item.author, familia::sans(13.0), cores::TEXTO_FRACO, 110.0),
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
            Stroke::new(1.0, cores::LINHA),
        );
        let left = familia::pintar_texto(
            &p,
            pos2(r.left() + PAD, r.center().y),
            familia::espacado(
                &self.check_line(),
                familia::mono(10.5),
                cores::TEXTO_APAGADO,
                1.8,
            ),
        );
        let mut right = r.right() - PAD;
        if let Some(task) = self.task.clone() {
            let cancel_g = p.layout_job(familia::espacado(
                "CANCELAR",
                familia::mono_negrito(10.5),
                cores::TEXTO,
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
                    cores::BRANCO
                } else {
                    cores::TEXTO
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
                (Some(_), Some(pr)) => format!(
                    "{} · {} de {}",
                    task.label,
                    bytes(pr.done),
                    pr.total.map_or("?".into(), bytes)
                ),
                _ => format!("{}...", task.label.trim_end_matches("...")),
            };
            let g = p.layout_job(familia::linha(
                &text,
                familia::sans(12.0),
                cores::TEXTO_FRACO,
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
                cores::ERRO
            } else {
                cores::TEXTO_FRACO
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
            "SEM ENDEREÇO DA LISTA"
        } else if url.starts_with("file:") {
            "VIA PASTA LOCAL"
        } else if url.contains("google.com") {
            "VIA DRIVE"
        } else {
            "VIA REDE"
        };
        let when = if self.task.as_ref().is_some_and(|t| t.is_manifest()) {
            "VERIFICANDO...".to_string()
        } else if self.offline {
            "LISTA GUARDADA · SEM CONEXÃO".to_string()
        } else if self.state.checked_at == 0 {
            "NUNCA VERIFICADO".to_string()
        } else {
            let min = state::now_secs().saturating_sub(self.state.checked_at) / 60;
            match min {
                0 => "VERIFICADO AGORA".to_string(),
                m if m < 120 => format!("VERIFICADO HÁ {m} MIN"),
                m => format!("VERIFICADO HÁ {} H", m / 60),
            }
        };
        format!("{when} · {via}")
    }

    fn modals(&mut self, ctx: &Context) {
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
                        .fill(cores::SUPERFICIE)
                        .stroke(Stroke::new(1.0, cores::LINHA_FORTE))
                        .inner_margin(Margin::same(22)),
                )
                .show(ctx, |ui| {
                    ui.set_width(520.0_f32.min(ctx.content_rect().width() - 80.0));
                    ui.label(name_job(&item, 20.0));
                    familia::rotulo(ui, "Notas da versão", 10.5, cores::TEXTO_APAGADO);
                    ui.add_space(6.0);
                    let mut close = false;
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .show(ui, |ui| {
                            let visible = product.visible(&item.channel);
                            if visible.is_empty() {
                                ui.label(
                                    egui::RichText::new("Nenhuma versão neste canal.")
                                        .color(cores::TEXTO_FRACO),
                                );
                            }
                            for r in visible {
                                let date = catalogo::date_pt(&r.published_at).unwrap_or_default();
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
                                    "Sem notas."
                                } else {
                                    r.notes.trim()
                                };
                                ui.label(egui::RichText::new(notes).color(cores::TEXTO));
                                ui.add_space(10.0);
                            }
                        });
                    ui.add_space(8.0);
                    if familia::botao(ui, "Fechar", Botao::Contorno, vec2(120.0, 32.0), true)
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
                .frame(Frame::NONE.fill(cores::SUPERFICIE).stroke(Stroke::new(1.0, cores::LINHA_FORTE)).inner_margin(Margin::same(22)))
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!("Remover {name} deste computador?")).size(16.0).color(cores::BRANCO));
                    ui.label(egui::RichText::new("Todas as versões instaladas e os atalhos saem. Os seus arquivos de trabalho não são tocados.").color(cores::TEXTO_FRACO));
                    ui.add_space(10.0);
                    let mut answer = None;
                    ui.horizontal(|ui| {
                        if familia::botao(ui, "Desinstalar", Botao::Perigo, vec2(150.0, 32.0), true).clicked() {
                            answer = Some(true);
                        }
                        if familia::botao(ui, "Cancelar", Botao::Contorno, vec2(120.0, 32.0), true).clicked() {
                            answer = Some(false);
                        }
                    });
                    answer
                });
            match modal.inner {
                Some(true) => {
                    self.confirm_uninstall = None;
                    match crate::uninstall_product(&self.env, &id, &name) {
                        Ok(()) => self.set_status(format!("{name} desinstalado."), false),
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

fn accent(item: &Item) -> Color32 {
    let [r, g, b] = item.color;
    Color32::from_rgb(r, g, b)
}

fn status_color(s: &Status) -> Color32 {
    match s {
        Status::Update { .. } => cores::AMARELO,
        Status::Installed { .. } => cores::TEXTO_FRACO,
        Status::NotInstalled => cores::TEXTO,
        Status::NoAccess | Status::NoPackage => cores::TEXTO_APAGADO,
    }
}

/// The name in two tones: the first part white, the second in the product's accent.
fn name_job(item: &Item, size: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c: Color32| egui::TextFormat {
        font_id: familia::negrito(size),
        color: c,
        ..Default::default()
    };
    let (a, b) = &item.name_parts;
    if !a.is_empty() {
        job.append(a, 0.0, fmt(cores::BRANCO));
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
    p.rect_filled(rect, 0.0, cores::LINHA);
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
    p.rect_filled(fill, 0.0, cores::BRANCO);
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

fn channel_name(c: &str) -> String {
    match c {
        manifest::STABLE => "estável".into(),
        manifest::TESTING => "testes".into(),
        other => other.into(),
    }
}

fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n as f64 >= MB {
        format!("{:.1} MB", n as f64 / MB)
    } else {
        format!("{:.0} kB", n as f64 / 1024.0)
    }
}

//! UTSim Launcher: the window (eframe) over the logic in `lib.rs`.
//!
//! `utsim-launcher --abrir <produto>` opens the active version of a product without showing
//! the launcher (this is what the shortcuts run); if that fails the window opens with the
//! reason.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use utsim_launcher::Env;
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::download::Progress;
use utsim_launcher_core::install;
use utsim_launcher_core::manifest::{self, Manifest, Product, Release};
use utsim_launcher_core::state::LauncherState;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = match Env::standard() {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("erro: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut first_message = None;
    let mut select = None;
    match args.first().map(String::as_str) {
        Some("--abrir") => {
            let Some(id) = args.get(1) else {
                eprintln!("uso: utsim-launcher --abrir <produto>");
                return ExitCode::FAILURE;
            };
            match install::launch(&env.layout, id) {
                Ok(_) => return ExitCode::SUCCESS,
                Err(e) => {
                    env.log.line(&format!("--abrir {id}: {e}"));
                    first_message = Some(format!("Não consegui abrir {id}: {e}"));
                    select = Some(id.clone());
                }
            }
        }
        Some("--versao" | "--version" | "-V") => {
            println!("utsim-launcher {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("--ajuda" | "--help" | "-h") => {
            println!(
                "utsim-launcher [--abrir <produto>]\n  sem argumentos: abre a janela do launcher\n  --abrir <produto>: abre a versão ativa do produto"
            );
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("UTSim Launcher")
            .with_inner_size([860.0, 560.0])
            .with_min_inner_size([620.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "UTSim Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, env, first_message, select)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro na janela: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What a background task hands back.
enum Outcome {
    Manifest(Result<(Manifest, LauncherState), String>),
    Installed(Result<String, String>),
    Message(Result<String, String>),
}

/// A task running on another thread.
struct Task {
    label: String,
    progress: Mutex<Option<Progress>>,
    cancel: AtomicBool,
    outcome: Mutex<Option<Outcome>>,
}

impl Task {
    fn new(label: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            label: label.into(),
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
}

struct App {
    env: Arc<Env>,
    state: LauncherState,
    manifest: Option<Manifest>,
    /// The manifest shown came from the local copy (no network).
    offline: bool,
    code_input: String,
    /// Manifest address typed on the code screen (advanced).
    url_input: String,
    code_error: Option<String>,
    editing_code: bool,
    selected: Option<String>,
    selected_version: Option<String>,
    task: Option<Arc<Task>>,
    status: String,
    status_is_error: bool,
    confirm_uninstall: bool,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        env: Arc<Env>,
        message: Option<String>,
        select: Option<String>,
    ) -> Self {
        cc.egui_ctx
            .set_pixels_per_point(cc.egui_ctx.pixels_per_point().max(1.1));
        install::clean_leftovers(&env.layout);
        let state = LauncherState::load(&env.layout.state_file());
        let manifest = utsim_launcher::cached_manifest(&env);
        let mut app = Self {
            offline: manifest.is_some(),
            manifest,
            code_input: String::new(),
            url_input: state.manifest_url.clone().unwrap_or_default(),
            code_error: None,
            editing_code: state.code.is_none(),
            selected: select,
            selected_version: None,
            task: None,
            status: message.clone().unwrap_or_default(),
            status_is_error: message.is_some(),
            confirm_uninstall: false,
            state,
            env,
        };
        if !app.editing_code {
            app.refresh(&cc.egui_ctx);
        }
        app
    }

    fn code(&self) -> Option<AccessCode> {
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

    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = msg.into();
        self.status_is_error = error;
        if error {
            self.env.log.line(&format!("erro: {}", self.status));
        }
    }

    fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Fetches the manifest in the background.
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let task = Task::new("Buscando a lista de versões...");
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let mut state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = utsim_launcher::fetch_manifest(&env, &mut state)
                .map(|m| (m, state))
                .map_err(|e| e.to_string());
            task.finish(Outcome::Manifest(r));
            ctx.request_repaint();
        });
    }

    fn start_install(&mut self, ctx: &egui::Context, product: &Product, release: &Release) {
        let Some(code) = self.code() else {
            self.editing_code = true;
            return;
        };
        let task = Task::new(format!(
            "Instalando {} {}...",
            product.name, release.version
        ));
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let (product, release) = (product.clone(), release.clone());
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
            let r = utsim_launcher::install_release(
                &env,
                &state,
                &product,
                &release,
                &code,
                &task.cancel,
                &mut progress,
            )
            .map(|_| format!("{} {} instalado.", product.name, release.version))
            .map_err(|e| e.to_string());
            task.finish(Outcome::Installed(r));
            ctx.request_repaint();
        });
    }

    fn start_report(&mut self, ctx: &egui::Context) {
        let task = Task::new("Gerando o relato de erro...");
        self.task = Some(Arc::clone(&task));
        let env = Arc::clone(&self.env);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = utsim_launcher::error_report(&env, &state, &utsim_launcher::downloads_dir())
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

    fn poll_task(&mut self) {
        let Some(task) = &self.task else {
            return;
        };
        let outcome = task.outcome.lock().ok().and_then(|mut o| o.take());
        let Some(outcome) = outcome else {
            return;
        };
        self.task = None;
        match outcome {
            Outcome::Manifest(Ok((m, state))) => {
                self.state = state;
                self.save_state();
                self.offline = false;
                if self.selected.is_none() {
                    self.selected = m.products.first().map(|p| p.id.clone());
                }
                self.manifest = Some(m);
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
            Outcome::Installed(Ok(msg)) | Outcome::Message(Ok(msg)) => {
                self.selected_version = None;
                self.set_status(msg, false);
            }
            Outcome::Installed(Err(e)) | Outcome::Message(Err(e)) => self.set_status(e, true),
        }
    }

    fn code_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.heading("UTSim Launcher");
            ui.add_space(12.0);
            ui.label("Digite o código de acesso que você recebeu do autor.");
            ui.label("Ele tem 20 letras e números, como ABCD-EFGH-JKMN-PQRS-TVWX.");
            ui.add_space(12.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.code_input)
                    .hint_text("XXXX-XXXX-XXXX-XXXX-XXXX")
                    .desired_width(320.0)
                    .font(egui::TextStyle::Monospace),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 180.0).max(0.0) / 2.0);
                let ok = ui.button("Continuar").clicked() || enter;
                if self.state.code.is_some() && ui.button("Cancelar").clicked() {
                    self.editing_code = false;
                    self.code_error = None;
                }
                if ok {
                    match AccessCode::parse(&self.code_input) {
                        Ok(code) => {
                            let url = self.url_input.trim();
                            self.state.manifest_url = (!url.is_empty()).then(|| url.to_string());
                            self.state.code = Some(code.formatted());
                            self.code_input.clear();
                            self.code_error = None;
                            self.editing_code = false;
                            self.save_state();
                            self.refresh(ui.ctx());
                        }
                        Err(e) => self.code_error = Some(e.to_string()),
                    }
                }
            });
            if let Some(e) = &self.code_error {
                ui.add_space(8.0);
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(16.0);
            let no_builtin = utsim_launcher::built_in_manifest_url().is_empty();
            egui::CollapsingHeader::new("Avançado")
                .default_open(no_builtin)
                .show(ui, |ui| {
                    ui.label("Endereço da lista de versões (só se o autor pedir):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.url_input)
                            .hint_text("link do manifesto.json no Google Drive")
                            .desired_width(420.0),
                    );
                });
            ui.add_space(16.0);
            ui.small("O código fica guardado só neste computador, na sua pasta de usuário.");
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("UTSim Launcher");
            ui.separator();
            if let Some(code) = self.code() {
                ui.label(format!("Testador {}", code.id()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Gerar relato de erro"))
                    .on_hover_text("Cria um .zip com os registros do launcher para enviar ao autor")
                    .clicked()
                {
                    self.start_report(ui.ctx());
                }
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Trocar código"))
                    .clicked()
                {
                    self.editing_code = true;
                }
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Atualizar lista"))
                    .clicked()
                {
                    self.refresh(ui.ctx());
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        if let Some(task) = self.task.clone() {
            ui.horizontal(|ui| {
                ui.label(&task.label);
                let p = task.progress.lock().ok().and_then(|p| *p);
                if let Some(p) = p {
                    let frac = p.total.map_or(0.0, |t| p.done as f32 / t.max(1) as f32);
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(260.0)
                            .text(format!(
                                "{} de {}",
                                bytes(p.done),
                                p.total.map_or("?".into(), bytes)
                            )),
                    );
                    if ui.button("Cancelar").clicked() {
                        task.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    ui.spinner();
                }
            });
        } else if !self.status.is_empty() {
            let color = if self.status_is_error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, &self.status);
        } else {
            ui.label(" ");
        }
    }

    fn products_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Aplicativos");
        ui.add_space(4.0);
        let Some(m) = &self.manifest else {
            ui.label("Nenhuma lista de versões ainda.");
            return;
        };
        let mut clicked = None;
        for p in &m.products {
            let active = install::active_version(&self.env.layout, &p.id);
            let latest = p
                .latest(self.state.channel(&p.id), &self.env.platform)
                .map(|r| r.version.clone());
            let badge = match (&active, &latest) {
                (Some(a), Some(l)) if manifest::compare_versions(l, a).is_gt() => " (atualização)",
                (Some(_), _) => " (instalado)",
                _ => "",
            };
            let selected = self.selected.as_deref() == Some(p.id.as_str());
            if ui
                .selectable_label(selected, format!("{}{badge}", p.name))
                .clicked()
            {
                clicked = Some(p.id.clone());
            }
        }
        if let Some(id) = clicked {
            self.selected = Some(id);
            self.selected_version = None;
            self.confirm_uninstall = false;
        }
        if self.offline {
            ui.add_space(8.0);
            ui.small("Lista guardada (sem conexão).");
        }
    }

    fn product_view(&mut self, ui: &mut egui::Ui) {
        let Some(m) = &self.manifest else {
            ui.label("Clique em \"Atualizar lista\" para buscar os aplicativos disponíveis.");
            return;
        };
        let Some(product) = self
            .selected
            .as_deref()
            .and_then(|id| m.product(id))
            .cloned()
        else {
            ui.label("Escolha um aplicativo à esquerda.");
            return;
        };
        let code = self.code();
        let layout = self.env.layout.clone();
        let platform = self.env.platform.clone();
        let active = install::active_version(&layout, &product.id);
        let installed = install::installed_versions(&layout, &product.id);

        ui.heading(&product.name);
        if !product.description.is_empty() {
            ui.label(&product.description);
        }
        ui.add_space(6.0);

        // Channel.
        let mut channel = self.state.channel(&product.id).to_string();
        let mut channels = product.channels();
        for c in [manifest::STABLE, manifest::TESTING] {
            if !channels.iter().any(|x| x == c) {
                channels.push(c.to_string());
            }
        }
        ui.horizontal(|ui| {
            ui.label("Canal:");
            egui::ComboBox::from_id_salt("canal")
                .selected_text(channel_name(&channel))
                .show_ui(ui, |ui| {
                    for c in &channels {
                        ui.selectable_value(&mut channel, c.clone(), channel_name(c));
                    }
                });
            if let Some(a) = &active {
                ui.separator();
                ui.label(format!("Versão ativa: {a}"));
            }
        });
        if channel != self.state.channel(&product.id) {
            self.state
                .channels
                .insert(product.id.clone(), channel.clone());
            self.selected_version = None;
            self.save_state();
        }

        let visible = product.visible(&channel);
        let latest = product
            .latest(&channel, &platform)
            .map(|r| r.version.clone());
        let selected_version = self
            .selected_version
            .clone()
            .or_else(|| active.clone())
            .or_else(|| latest.clone());

        ui.add_space(6.0);
        ui.columns(2, |cols| {
            cols[0].strong("Versões");
            egui::ScrollArea::vertical()
                .id_salt("versoes")
                .max_height(220.0)
                .show(&mut cols[0], |ui| {
                    if visible.is_empty() {
                        ui.label("Nenhuma versão neste canal.");
                    }
                    for r in &visible {
                        let mut tags = Vec::new();
                        if r.channel != manifest::STABLE {
                            tags.push(channel_name(&r.channel));
                        }
                        if active.as_deref() == Some(r.version.as_str()) {
                            tags.push("ativa".into());
                        } else if installed.contains(&r.version) {
                            tags.push("instalada".into());
                        }
                        match r.package(&platform) {
                            None => tags.push("sem pacote para este sistema".into()),
                            Some(p) => {
                                if code
                                    .as_ref()
                                    .is_some_and(|c| !utsim_launcher::listed_for(p, c))
                                {
                                    tags.push("sem acesso".into());
                                }
                            }
                        }
                        let text = if tags.is_empty() {
                            r.version.clone()
                        } else {
                            format!("{} ({})", r.version, tags.join(", "))
                        };
                        let sel = selected_version.as_deref() == Some(r.version.as_str());
                        if ui.selectable_label(sel, text).clicked() {
                            self.selected_version = Some(r.version.clone());
                        }
                    }
                });
            cols[1].strong("Notas da versão");
            let notes = selected_version
                .as_deref()
                .and_then(|v| product.release(v))
                .map(|r| r.notes.clone())
                .unwrap_or_default();
            egui::ScrollArea::vertical()
                .id_salt("notas")
                .max_height(220.0)
                .show(&mut cols[1], |ui| {
                    if notes.trim().is_empty() {
                        ui.weak("Sem notas.");
                    } else {
                        ui.label(notes);
                    }
                });
        });

        ui.add_space(10.0);
        let release = selected_version
            .as_deref()
            .and_then(|v| product.release(v))
            .cloned();
        let busy = self.busy();
        ui.horizontal_wrapped(|ui| {
            if let Some(r) = &release {
                let is_installed = installed.contains(&r.version);
                let is_active = active.as_deref() == Some(r.version.as_str());
                if !is_installed {
                    let newer = active
                        .as_deref()
                        .is_some_and(|a| manifest::compare_versions(&r.version, a).is_gt());
                    let label = if active.is_none() {
                        format!("Instalar {}", r.version)
                    } else if newer {
                        format!("Atualizar para {}", r.version)
                    } else {
                        format!("Instalar {} (anterior)", r.version)
                    };
                    let can = r.package(&platform).is_some();
                    if ui
                        .add_enabled(!busy && can, egui::Button::new(label))
                        .clicked()
                    {
                        self.start_install(ui.ctx(), &product, r);
                    }
                } else if !is_active
                    && ui
                        .add_enabled(!busy, egui::Button::new(format!("Usar a {}", r.version)))
                        .clicked()
                {
                    match install::activate(&layout, &product.id, &r.version) {
                        Ok(()) => {
                            self.set_status(format!("{} {} ativa.", product.name, r.version), false)
                        }
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                }
            }
            if active.is_some() {
                if ui
                    .add_enabled(!busy, egui::Button::new(format!("Abrir {}", product.name)))
                    .clicked()
                {
                    match install::launch(&layout, &product.id) {
                        Ok(_) => self.set_status(format!("{} aberto.", product.name), false),
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                }
                let olds = installed.len().saturating_sub(1);
                if olds > 0
                    && ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(format!("Remover versões antigas ({olds})")),
                        )
                        .clicked()
                {
                    match install::remove_old(&layout, &product.id, 0) {
                        Ok(v) => self.set_status(format!("Removidas: {}.", v.join(", ")), false),
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                }
                if !self.confirm_uninstall {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Desinstalar"))
                        .clicked()
                    {
                        self.confirm_uninstall = true;
                    }
                } else {
                    ui.label(format!("Remover {} deste computador?", product.name));
                    if ui.button("Sim, desinstalar").clicked() {
                        self.confirm_uninstall = false;
                        match utsim_launcher::uninstall_product(
                            &self.env,
                            &product.id,
                            &product.name,
                        ) {
                            Ok(()) => {
                                self.set_status(format!("{} desinstalado.", product.name), false)
                            }
                            Err(e) => self.set_status(e.to_string(), true),
                        }
                    }
                    if ui.button("Não").clicked() {
                        self.confirm_uninstall = false;
                    }
                }
            }
        });
        ui.add_space(6.0);
        let mut desktop = self.state.desktop_shortcuts;
        if ui
            .checkbox(&mut desktop, "Criar atalho na área de trabalho ao instalar")
            .changed()
        {
            self.state.desktop_shortcuts = desktop;
            self.save_state();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_task();
        if self.editing_code {
            egui::CentralPanel::default().show(ui, |ui| self.code_screen(ui));
            return;
        }
        egui::Panel::top("topo").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("estado").show(ui, |ui| {
            ui.add_space(3.0);
            self.status_bar(ui);
            ui.add_space(3.0);
        });
        egui::Panel::left("produtos")
            .default_size(220.0)
            .show(ui, |ui| self.products_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.product_view(ui));
    }
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

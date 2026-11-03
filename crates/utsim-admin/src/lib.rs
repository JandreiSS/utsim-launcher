//! The author's window over `utsim-publicar`: the same data folder as the command line
//! (`%APPDATA%\UTSim`: keys, `testadores.json`) and a publication folder chosen here
//! (remembered; first `G:\Meu Drive\UTSim - testes`).
//!
//! Four screens: **Testadores** (add, with the code shown once; revoke, reactivate, channels),
//! **Publicações** (products, versions, link and access of each package, check, re-wrap),
//! **Acesso** (per product and channel: with code or open to everyone) and **Chaves** (the
//! public key; the private key is never shown). Without the private key the window only
//! says how to create it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Color32, Context, Frame, Id, Margin, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::manifest::{self, Access, Manifest};
use utsim_launcher_core::signature;
use utsim_publicar::{Output, Store, Tester};
use utsim_ui_tema::familia::{self, Botao, cores};

const TOP_H: f32 = 48.0;
const FOOT_H: f32 = 30.0;
const PAD: f32 = 20.0;

/// The screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tela {
    Testadores,
    Publicacoes,
    Acesso,
    Chaves,
}

impl Tela {
    pub const ALL: [Tela; 4] = [
        Tela::Testadores,
        Tela::Publicacoes,
        Tela::Acesso,
        Tela::Chaves,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tela::Testadores => "Testadores",
            Tela::Publicacoes => "Publicações",
            Tela::Acesso => "Acesso",
            Tela::Chaves => "Chaves",
        }
    }
}

/// A change of access waiting for confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessChange {
    pub product: String,
    pub channel: String,
    pub access: Access,
}

/// Something running on another thread (re-wrapping takes Argon2 time per tester).
struct Job {
    label: String,
    result: Mutex<Option<Result<String, String>>>,
}

/// The admin window.
pub struct Admin {
    pub data: PathBuf,
    /// The data folder, or why it cannot be used (no private key, inside a repository).
    pub store: Result<Store, String>,
    pub output: String,
    pub tela: Tela,
    pub testers: Vec<Tester>,
    pub manifest: Option<Manifest>,
    pub manifest_error: Option<String>,
    pub check: Option<(String, usize)>,
    pub new_name: String,
    pub new_channels: [bool; 2],
    /// The tester just added: the code is shown once.
    pub shown_code: Option<Tester>,
    /// Tester whose channels are being edited, and the text typed.
    pub editing: Option<(String, String)>,
    pub confirm: Option<AccessChange>,
    pub status: String,
    pub status_is_error: bool,
    job: Option<Arc<Job>>,
    params: Argon2Params,
}

impl Admin {
    /// Opens the data folder `data` (checks the private key) and the last publication folder.
    pub fn open(data: PathBuf) -> Self {
        Self::open_with(data, Argon2Params::DEFAULT)
    }

    /// [`Admin::open`] with other Argon2 parameters (tests use the fast ones).
    pub fn open_with(data: PathBuf, params: Argon2Params) -> Self {
        let mut a = Self {
            store: Err(String::new()),
            output: String::new(),
            tela: Tela::Testadores,
            testers: Vec::new(),
            manifest: None,
            manifest_error: None,
            check: None,
            new_name: String::new(),
            new_channels: [true, true],
            shown_code: None,
            editing: None,
            confirm: None,
            status: String::new(),
            status_is_error: false,
            job: None,
            params,
            data,
        };
        a.reopen();
        a
    }

    /// Opens the data folder again (after creating the key, for example).
    pub fn reopen(&mut self) {
        self.store = Store::open(&self.data, false)
            .map_err(|e| e.to_string())
            .and_then(|s| match s.signing_key() {
                Ok(_) => Ok(s),
                Err(_) => Err(format!(
                    "Chave privada não encontrada em {}.\nRode 'utsim-publicar chaves gerar' (uma vez só) e abra o administrador de novo.",
                    s.private_key_path().display()
                )),
            });
        if let Ok(store) = &self.store {
            self.output = store.admin_settings().output_dir().display().to_string();
        }
        self.reload();
    }

    fn out(&self) -> Output {
        Output {
            dir: PathBuf::from(self.output.trim()),
        }
    }

    /// Reads the testers and the manifest again.
    pub fn reload(&mut self) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        match store.testers() {
            Ok(t) => self.testers = t,
            Err(e) => self.set_status(e.to_string(), true),
        }
        self.manifest = None;
        self.manifest_error = None;
        let out = self.out();
        if !out.manifest_path().is_file() {
            self.manifest_error = Some(format!(
                "Nenhum manifesto em {} ainda; a primeira publicação (utsim-publicar publicar) cria.",
                out.dir.display()
            ));
            return;
        }
        match store.signing_key().and_then(|k| out.load(&k)) {
            Ok(m) => self.manifest = Some(m),
            Err(e) => self.manifest_error = Some(e.to_string()),
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = msg.into();
        self.status_is_error = error;
    }

    fn report<T>(&mut self, r: utsim_launcher_core::Result<T>, ok: impl FnOnce(T) -> String) {
        match r {
            Ok(v) => {
                let msg = ok(v);
                self.set_status(msg, false);
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
        self.reload();
    }

    /// Adds a tester; the code is shown once.
    pub fn add_tester(&mut self) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let mut channels = Vec::new();
        if self.new_channels[0] {
            channels.push(manifest::STABLE.to_string());
        }
        if self.new_channels[1] {
            channels.push(manifest::TESTING.to_string());
        }
        match store.add_tester(&self.new_name, &channels) {
            Ok(t) => {
                self.set_status(
                    format!("{} cadastrado. Ele entra nas próximas publicações; para as já publicadas use Publicações › Refazer acesso.", t.name),
                    false,
                );
                self.shown_code = Some(t);
                self.new_name.clear();
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
        self.reload();
    }

    pub fn set_active(&mut self, who: &str, active: bool) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let r = store.set_active(who, active);
        self.report(r, |t| {
            if active {
                format!("{} reativado.", t.name)
            } else {
                format!("{} revogado: as próximas publicações não o incluem; Refazer acesso tira dele o que ainda não baixou.", t.name)
            }
        });
    }

    pub fn save_channels(&mut self, who: &str, text: &str) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let channels: Vec<String> = text
            .split([',', ' ', ';'])
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(String::from)
            .collect();
        let r = store.set_channels(who, &channels);
        self.report(r, |t| {
            format!("Canais de {}: {}.", t.name, t.channels.join(", "))
        });
    }

    /// Uses the publication folder typed, remembering it.
    pub fn use_output(&mut self) {
        if let Ok(store) = &self.store {
            let mut s = store.admin_settings();
            s.output = Some(PathBuf::from(self.output.trim()));
            if let Err(e) = store.save_admin_settings(&s) {
                self.set_status(e.to_string(), true);
            }
        }
        self.check = None;
        self.reload();
    }

    pub fn run_check(&mut self) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        match utsim_publicar::check(&store, &self.out()) {
            Ok(r) => {
                let msg = if r.1 == 0 {
                    "Verificação: tudo certo.".to_string()
                } else {
                    format!("Verificação: {} problema(s).", r.1)
                };
                self.check = Some(r);
                self.set_status(msg, false);
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
    }

    /// Applies an access change (already confirmed).
    pub fn apply_access(&mut self, c: &AccessChange) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let r = utsim_publicar::set_access(&store, &self.out(), &c.product, &c.channel, c.access);
        let (p, ch) = (c.product.clone(), c.channel.clone());
        let access = c.access;
        self.report(r, move |n| match access {
            Access::Open => format!("{p} no canal {ch}: aberto para todos ({n} pacote(s) já publicados abertos); manifesto reassinado."),
            Access::Code => format!("{p} no canal {ch}: com código a partir da próxima publicação; manifesto reassinado."),
        });
    }

    /// Re-wraps every published package for the active testers (in the background).
    pub fn start_rewrap(&mut self, ctx: &Context) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let job = Arc::new(Job {
            label: "Refazendo o acesso (alguns segundos por testador e pacote)...".into(),
            result: Mutex::new(None),
        });
        self.job = Some(Arc::clone(&job));
        let out = self.out();
        let params = self.params;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = utsim_publicar::rewrap(&store, &out, None, params)
                .map(|n| format!("{n} pacote(s) com o acesso refeito para os testadores ativos; manifesto reassinado."))
                .map_err(|e| e.to_string());
            if let Ok(mut slot) = job.result.lock() {
                *slot = Some(r);
            }
            ctx.request_repaint();
        });
    }

    /// Waits for the background job (tests).
    pub fn wait(&mut self) {
        while self.job.is_some() {
            self.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let r = job.result.lock().ok().and_then(|mut r| r.take());
        if let Some(r) = r {
            self.job = None;
            match r {
                Ok(m) => self.set_status(m, false),
                Err(e) => self.set_status(e, true),
            }
            self.reload();
        }
    }

    /// Draws the window (one frame).
    pub fn draw(&mut self, ui: &mut Ui) {
        self.poll();
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
            .show(ui, |ui| {
                if let Err(e) = self.store.clone() {
                    self.no_key(ui, &e);
                    return;
                }
                self.toolbar(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tela {
                        Tela::Testadores => self.testers_screen(ui),
                        Tela::Publicacoes => self.publications_screen(ui),
                        Tela::Acesso => self.access_screen(ui),
                        Tela::Chaves => self.keys_screen(ui),
                    });
            });
        let ctx = ui.ctx().clone();
        self.modals(&ctx);
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
        let label = familia::pintar_texto(
            &p,
            pos2(x + 16.0, r.center().y),
            familia::espacado(
                "ADMINISTRAÇÃO",
                familia::mono(11.0),
                cores::TEXTO_FRACO,
                2.6,
            ),
        );
        // Screens as tabs, the active one underlined in white.
        let mut tx = label.right() + 40.0;
        for t in Tela::ALL {
            let on = self.tela == t;
            let g = p.layout_job(familia::espacado(
                &t.label().to_uppercase(),
                familia::mono_negrito(11.0),
                if on {
                    cores::BRANCO
                } else {
                    cores::TEXTO_FRACO
                },
                1.8,
            ));
            let tab = Rect::from_min_size(pos2(tx, r.top()), vec2(g.size().x + 28.0, TOP_H - 1.0));
            let resp = ui.allocate_rect(tab, Sense::click());
            if resp.hovered() && !on {
                p.rect_filled(tab, 0.0, cores::SOBRE);
            }
            p.galley(tab.center() - g.size() / 2.0, g, Color32::WHITE);
            if on {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(tab.left(), tab.bottom() - 2.0),
                        vec2(tab.width(), 2.0),
                    ),
                    0.0,
                    cores::BRANCO,
                );
            }
            if resp.clicked() {
                self.tela = t;
            }
            tx = tab.right();
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
        let h = 34.0;
        let y = bar.center().y - h / 2.0;
        let p = ui.painter().clone();
        let lg = p.layout_job(familia::espacado(
            "PASTA DE PUBLICAÇÃO",
            familia::mono(10.5),
            cores::TEXTO_APAGADO,
            2.0,
        ));
        let lw = lg.size().x;
        p.galley(
            pos2(bar.left(), bar.center().y - lg.size().y / 2.0),
            lg,
            Color32::WHITE,
        );
        let bw = 110.0;
        let field = Rect::from_min_max(
            pos2(bar.left() + lw + 14.0, y),
            pos2(bar.right() - bw - 8.0 - 130.0 - 8.0, y + h),
        );
        let mut enter = false;
        ui.scope_builder(UiBuilder::new().max_rect(field), |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.output)
                    .desired_width(field.width())
                    .min_size(vec2(field.width(), h))
                    .vertical_align(egui::Align::Center)
                    .font(familia::mono(13.0))
                    .margin(Margin::symmetric(10, 0)),
            );
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        let use_r = Rect::from_min_size(pos2(field.right() + 8.0, y), vec2(bw, h));
        if familia::botao_em(ui, use_r, "Usar", Botao::Contorno, true).clicked() || enter {
            self.use_output();
        }
        let reload = Rect::from_min_size(pos2(use_r.right() + 8.0, y), vec2(130.0, h));
        if familia::botao_em(ui, reload, "Recarregar", Botao::Contorno, true).clicked() {
            self.reload();
        }
    }

    fn no_key(&mut self, ui: &mut Ui, why: &str) {
        ui.add_space(70.0);
        ui.vertical_centered(|ui| {
            familia::rotulo(ui, "Sem chave de assinatura", 12.0, cores::AMARELO);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(why).size(14.0).color(cores::TEXTO));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("O administrador nunca cria nem mostra a chave privada: ela é gerada uma vez pela linha de comando e fica só nesta máquina.")
                    .size(12.5)
                    .color(cores::TEXTO_FRACO),
            );
            ui.add_space(14.0);
            if familia::botao(ui, "Tentar de novo", Botao::Contorno, vec2(180.0, 34.0), true).clicked() {
                self.reopen();
            }
        });
    }

    fn section(ui: &mut Ui, title: &str) {
        ui.add_space(4.0);
        familia::rotulo(ui, title, 10.5, cores::TEXTO_APAGADO);
        ui.add_space(2.0);
    }

    fn testers_screen(&mut self, ui: &mut Ui) {
        Self::section(ui, "Novo testador");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_name)
                    .hint_text("Nome do testador")
                    .desired_width(280.0)
                    .min_size(vec2(280.0, 34.0))
                    .vertical_align(egui::Align::Center)
                    .margin(Margin::symmetric(10, 0)),
            );
            ui.checkbox(&mut self.new_channels[0], "estável");
            ui.checkbox(&mut self.new_channels[1], "testes");
            let ok = !self.new_name.trim().is_empty() && self.new_channels.iter().any(|c| *c);
            if familia::botao(ui, "Adicionar", Botao::Primario, vec2(140.0, 34.0), ok).clicked() {
                self.add_tester();
            }
        });
        ui.add_space(14.0);
        Self::section(ui, &format!("Testadores ({})", self.testers.len()));
        let w = ui.available_width();
        let cols = [0.0, 0.28, 0.38, 0.58, 0.7, 0.82];
        let head = ["NOME", "ID", "CANAIS", "SITUAÇÃO", "DESDE", ""];
        // Rows touch: the hairlines are the only separation.
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        let (hr, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_stroke(
            hr,
            0.0,
            Stroke::new(1.0, cores::LINHA),
            egui::StrokeKind::Inside,
        );
        for (k, h) in head.iter().enumerate() {
            familia::pintar_texto(
                &p,
                pos2(hr.left() + 16.0 + cols[k] * w, hr.center().y),
                familia::espacado(h, familia::mono(10.5), cores::TEXTO_APAGADO, 2.0),
            );
        }
        if self.testers.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Nenhum testador ainda.").color(cores::TEXTO_FRACO));
        }
        let testers = self.testers.clone();
        for t in &testers {
            let (row, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
            p.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(1.0, cores::LINHA),
            );
            p.line_segment(
                [row.left_top(), row.left_bottom()],
                Stroke::new(1.0, cores::LINHA),
            );
            p.line_segment(
                [row.right_top(), row.right_bottom()],
                Stroke::new(1.0, cores::LINHA),
            );
            let x = |k: usize| row.left() + 16.0 + cols[k] * w;
            let cy = row.center().y;
            familia::pintar_texto(
                &p,
                pos2(x(0), cy),
                familia::linha(
                    &t.name,
                    familia::negrito(15.0),
                    cores::BRANCO,
                    cols[1] * w - 20.0,
                ),
            );
            familia::pintar_texto(
                &p,
                pos2(x(1), cy),
                familia::espacado(&t.id, familia::mono(13.0), cores::TEXTO, 1.0),
            );
            let editing = self.editing.as_ref().is_some_and(|(id, _)| id == &t.id);
            if editing {
                let field = Rect::from_min_size(
                    pos2(x(2), cy - 15.0),
                    vec2(cols[3] * w - cols[2] * w - 70.0, 30.0),
                );
                let save =
                    Rect::from_min_size(pos2(field.right() + 6.0, cy - 15.0), vec2(56.0, 30.0));
                let mut done = false;
                if let Some((_, text)) = &mut self.editing {
                    ui.scope_builder(UiBuilder::new().max_rect(field), |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(field.width())
                                .min_size(field.size())
                                .vertical_align(egui::Align::Center)
                                .margin(Margin::symmetric(8, 0)),
                        );
                        done = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                }
                if (familia::botao_em(ui, save, "Ok", Botao::Primario, true).clicked() || done)
                    && let Some((id, text)) = self.editing.take()
                {
                    self.save_channels(&id, &text);
                }
            } else {
                familia::pintar_texto(
                    &p,
                    pos2(x(2), cy),
                    familia::linha(
                        &t.channels.join(", "),
                        familia::mono(12.5),
                        cores::TEXTO_FRACO,
                        cols[3] * w - cols[2] * w - 16.0,
                    ),
                );
            }
            let (txt, color) = if t.active {
                ("Ativo", cores::VERDE)
            } else {
                ("Revogado", cores::ERRO)
            };
            familia::pintar_etiqueta(&p, pos2(x(3), cy), txt, color, 10.0);
            let since = t.created_at.get(..10).unwrap_or("");
            familia::pintar_texto(
                &p,
                pos2(x(4), cy),
                familia::espacado(since, familia::mono(12.0), cores::TEXTO_FRACO, 0.8),
            );
            let b1 = Rect::from_min_size(
                pos2(row.right() - 16.0 - 120.0, cy - 15.0),
                vec2(120.0, 30.0),
            );
            let b0 =
                Rect::from_min_size(pos2(b1.left() - 8.0 - 100.0, cy - 15.0), vec2(100.0, 30.0));
            ui.push_id(&t.id, |ui| {
                if !editing && familia::botao_em(ui, b0, "Canais", Botao::Contorno, true).clicked()
                {
                    self.editing = Some((t.id.clone(), t.channels.join(", ")));
                }
                let (label, kind) = if t.active {
                    ("Revogar", Botao::Perigo)
                } else {
                    ("Reativar", Botao::Contorno)
                };
                if familia::botao_em(ui, b1, label, kind, true).clicked() {
                    self.set_active(&t.id, !t.active);
                }
            });
        }
        ui.spacing_mut().item_spacing.y = spacing;
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Revogar e reativar valem para as próximas publicações. Para mudar o acesso às versões já publicadas, use Publicações › Refazer acesso.")
                .size(12.5)
                .color(cores::TEXTO_FRACO),
        );
    }

    fn manifest_missing(&self, ui: &mut Ui) -> bool {
        if let Some(e) = &self.manifest_error {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(e).color(cores::TEXTO_FRACO));
            return true;
        }
        self.manifest.is_none()
    }

    fn publications_screen(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.horizontal(|ui| {
            if familia::botao(ui, "Verificar", Botao::Contorno, vec2(140.0, 34.0), self.manifest.is_some()).clicked() {
                self.run_check();
            }
            if familia::botao(ui, "Refazer acesso", Botao::Primario, vec2(180.0, 34.0), self.manifest.is_some() && !busy)
                .on_hover_text("Reembrulha a chave de cada pacote para os testadores ativos agora (inclui os novos, tira os revogados)")
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_rewrap(&ctx);
            }
            if let Some(job) = &self.job {
                ui.spinner();
                ui.label(egui::RichText::new(&job.label).color(cores::TEXTO_FRACO));
            }
        });
        ui.add_space(10.0);
        if self.manifest_missing(ui) {
            return;
        }
        let Some(m) = self.manifest.clone() else {
            return;
        };
        let names: std::collections::HashMap<String, String> = self
            .testers
            .iter()
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();
        Self::section(
            ui,
            &format!("Manifesto · sequência {} · {}", m.sequence, m.published_at),
        );
        for p in &m.products {
            let accent = p
                .color
                .as_deref()
                .and_then(familia::cor_hex)
                .unwrap_or(cores::DESTAQUE);
            Frame::NONE
                .stroke(Stroke::new(1.0, cores::LINHA))
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&p.name)
                                .font(familia::negrito(18.0))
                                .color(accent),
                        );
                        ui.label(familia::espacado(
                            &p.id,
                            familia::mono(12.0),
                            cores::TEXTO_FRACO,
                            1.0,
                        ));
                    });
                    if p.releases.is_empty() {
                        ui.label(egui::RichText::new("Sem versões.").color(cores::TEXTO_FRACO));
                    }
                    for r in
                        p.visible(manifest::TESTING)
                            .into_iter()
                            .chain(p.releases.iter().filter(|r| {
                                r.channel != manifest::STABLE && r.channel != manifest::TESTING
                            }))
                    {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(familia::espacado(
                                &r.version,
                                familia::mono_negrito(13.0),
                                cores::BRANCO,
                                1.0,
                            ));
                            familia::etiqueta(ui, &r.channel, cores::TEXTO_FRACO);
                            ui.label(
                                egui::RichText::new(r.published_at.get(..10).unwrap_or(""))
                                    .color(cores::TEXTO_APAGADO),
                            );
                        });
                        for pkg in &r.packages {
                            ui.horizontal(|ui| {
                                ui.add_space(18.0);
                                ui.label(familia::espacado(
                                    &pkg.platform,
                                    familia::mono(12.0),
                                    cores::TEXTO,
                                    0.8,
                                ));
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{:.1} MB",
                                        pkg.size as f64 / 1_048_576.0
                                    ))
                                    .color(cores::TEXTO_FRACO),
                                );
                                if pkg.url.trim().is_empty() {
                                    familia::etiqueta(ui, "Sem link", cores::AMARELO);
                                } else {
                                    familia::etiqueta(ui, "Link ok", cores::VERDE);
                                }
                                if pkg.is_open() {
                                    familia::etiqueta(ui, "Aberto para todos", cores::VERDE);
                                }
                                let who: Vec<String> = pkg
                                    .wraps
                                    .iter()
                                    .map(|w| {
                                        names
                                            .get(&w.id)
                                            .cloned()
                                            .unwrap_or_else(|| format!("{} (?)", w.id))
                                    })
                                    .collect();
                                let text = if who.is_empty() {
                                    "nenhum testador".to_string()
                                } else {
                                    who.join(", ")
                                };
                                ui.label(
                                    egui::RichText::new(format!("acesso: {text}"))
                                        .color(cores::TEXTO_FRACO),
                                );
                            });
                        }
                    }
                });
            ui.add_space(8.0);
        }
        if let Some((report, problems)) = &self.check {
            Self::section(ui, &format!("Verificação · {problems} problema(s)"));
            Frame::NONE
                .stroke(Stroke::new(
                    1.0,
                    if *problems > 0 {
                        cores::AMARELO
                    } else {
                        cores::LINHA
                    },
                ))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(report)
                            .font(familia::mono(12.0))
                            .color(cores::TEXTO),
                    );
                });
        }
    }

    fn access_screen(&mut self, ui: &mut Ui) {
        ui.label(
            egui::RichText::new("\"Com código\": só testadores cadastrados instalam. \"Aberto para todos\": qualquer pessoa com o launcher instala, sem código (a chave do pacote vai no manifesto assinado).")
                .color(cores::TEXTO_FRACO),
        );
        ui.add_space(8.0);
        if self.manifest_missing(ui) {
            return;
        }
        let Some(m) = self.manifest.clone() else {
            return;
        };
        for p in &m.products {
            let accent = p
                .color
                .as_deref()
                .and_then(familia::cor_hex)
                .unwrap_or(cores::DESTAQUE);
            let mut channels = p.channels();
            for c in [manifest::STABLE, manifest::TESTING] {
                if !channels.iter().any(|x| x == c) {
                    channels.push(c.to_string());
                }
            }
            Frame::NONE
                .stroke(Stroke::new(1.0, cores::LINHA))
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(&p.name)
                            .font(familia::negrito(18.0))
                            .color(accent),
                    );
                    for c in &channels {
                        let now = p.access(c);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                vec2(110.0, 30.0),
                                egui::Label::new(familia::espacado(
                                    &c.to_uppercase(),
                                    familia::mono(12.0),
                                    cores::TEXTO,
                                    1.6,
                                )),
                            );
                            let n = p.releases.iter().filter(|r| &r.channel == c).count();
                            ui.push_id(c, |ui| {
                                for (access, label) in [
                                    (Access::Code, "Com código"),
                                    (Access::Open, "Aberto para todos"),
                                ] {
                                    let on = now == access;
                                    let kind = if on { Botao::Primario } else { Botao::Contorno };
                                    // The current choice is the white one; clicking it does nothing.
                                    if familia::botao(ui, label, kind, vec2(190.0, 30.0), true)
                                        .clicked()
                                        && !on
                                    {
                                        self.confirm = Some(AccessChange {
                                            product: p.id.clone(),
                                            channel: c.clone(),
                                            access,
                                        });
                                    }
                                }
                            });
                            ui.label(
                                egui::RichText::new(format!("{n} versão(ões) neste canal"))
                                    .color(cores::TEXTO_APAGADO),
                            );
                        });
                    }
                });
            ui.add_space(8.0);
        }
    }

    fn keys_screen(&mut self, ui: &mut Ui) {
        let Ok(store) = self.store.clone() else {
            return;
        };
        let key = match store.signing_key() {
            Ok(k) => k,
            Err(e) => {
                ui.colored_label(cores::ERRO, e.to_string());
                return;
            }
        };
        let public = key.verifying_key();
        let text = signature::public_to_text(&public);
        Self::section(ui, "Chave pública (embutida no launcher)");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&text)
                    .font(familia::mono(13.0))
                    .color(cores::BRANCO),
            );
            if familia::botao(ui, "Copiar", Botao::Contorno, vec2(100.0, 30.0), true).clicked() {
                ui.ctx().copy_text(text.clone());
                self.set_status("Chave pública copiada.", false);
            }
        });
        ui.add_space(8.0);
        Self::section(ui, "Id da chave");
        ui.label(
            egui::RichText::new(signature::key_id(&public))
                .font(familia::mono(13.0))
                .color(cores::TEXTO),
        );
        ui.add_space(8.0);
        Self::section(ui, "Arquivos");
        ui.label(
            egui::RichText::new(format!("privada: {}", store.private_key_path().display()))
                .font(familia::mono(12.0))
                .color(cores::TEXTO_FRACO),
        );
        ui.label(
            egui::RichText::new(format!("pública: {}", store.public_key_path().display()))
                .font(familia::mono(12.0))
                .color(cores::TEXTO_FRACO),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("A chave privada nunca é mostrada aqui. Guarde uma cópia dela fora deste computador (pendrive ou gerenciador de senhas); sem ela, só um launcher novo aceita versões novas.")
                .color(cores::AMARELO),
        );
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
                &format!("DADOS · {}", self.data.display()).to_uppercase(),
                familia::mono(10.5),
                cores::TEXTO_APAGADO,
                1.4,
            ),
        );
        if !self.status.is_empty() {
            let color = if self.status_is_error {
                cores::ERRO
            } else {
                cores::TEXTO_FRACO
            };
            let g = p.layout_job(familia::linha(
                &self.status,
                familia::sans(12.0),
                color,
                (r.right() - PAD - left.right() - 40.0).max(40.0),
            ));
            p.galley(
                pos2(
                    r.right() - PAD - g.size().x,
                    r.center().y - g.size().y / 2.0,
                ),
                g,
                Color32::WHITE,
            );
        }
    }

    fn modals(&mut self, ctx: &Context) {
        let frame = Frame::NONE
            .fill(cores::SUPERFICIE)
            .stroke(Stroke::new(1.0, cores::LINHA_FORTE))
            .inner_margin(Margin::same(22));
        if let Some(t) = self.shown_code.clone() {
            let m = egui::Modal::new(Id::new("codigo")).frame(frame).show(ctx, |ui| {
                ui.set_width(520.0);
                familia::rotulo(ui, &format!("Código de acesso de {}", t.name), 11.0, cores::TEXTO_APAGADO);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&t.code).font(familia::mono_negrito(26.0)).color(cores::BRANCO));
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Envie este código só para essa pessoa, por um canal privado. Ele não volta a aparecer nesta janela (fica em testadores.json, fora do repositório).")
                        .color(cores::AMARELO),
                );
                ui.add_space(12.0);
                let mut close = false;
                ui.horizontal(|ui| {
                    if familia::botao(ui, "Copiar", Botao::Primario, vec2(130.0, 34.0), true).clicked() {
                        ui.ctx().copy_text(t.code.clone());
                    }
                    if familia::botao(ui, "Já anotei", Botao::Contorno, vec2(130.0, 34.0), true).clicked() {
                        close = true;
                    }
                });
                close
            });
            if m.inner {
                self.shown_code = None;
            }
        }
        if let Some(c) = self.confirm.clone() {
            let m = egui::Modal::new(Id::new("acesso")).frame(frame).show(ctx, |ui| {
                ui.set_width(540.0);
                let title = match c.access {
                    Access::Open => format!("Abrir {} ({}) para todos?", c.product, c.channel),
                    Access::Code => format!("Voltar {} ({}) a exigir código?", c.product, c.channel),
                };
                ui.label(egui::RichText::new(title).size(17.0).color(cores::BRANCO));
                ui.add_space(6.0);
                let body = match c.access {
                    Access::Open => "Qualquer pessoa com o launcher (ou com o link do manifesto) poderá instalar as versões já publicadas neste canal e as próximas, sem código. O manifesto é reassinado agora.",
                    Access::Code => "As próximas publicações voltam a exigir código. ATENÇÃO: as versões publicadas enquanto o canal esteve aberto continuam abrindo para quem baixou o manifesto nesse período, porque a chave delas foi pública. Para fechar de fato, publique uma versão nova (e use Refazer acesso para tirar as chaves antigas do manifesto).",
                };
                ui.label(egui::RichText::new(body).color(if c.access == Access::Code { cores::AMARELO } else { cores::TEXTO }));
                ui.add_space(12.0);
                let mut answer = None;
                ui.horizontal(|ui| {
                    let label = if c.access == Access::Open { "Abrir para todos" } else { "Exigir código" };
                    if familia::botao(ui, label, Botao::Primario, vec2(190.0, 34.0), true).clicked() {
                        answer = Some(true);
                    }
                    if familia::botao(ui, "Cancelar", Botao::Contorno, vec2(120.0, 34.0), true).clicked() {
                        answer = Some(false);
                    }
                });
                answer
            });
            match m.inner {
                Some(true) => {
                    self.confirm = None;
                    self.apply_access(&c);
                }
                Some(false) => self.confirm = None,
                None if m.should_close() => self.confirm = None,
                None => {}
            }
        }
    }
}

impl eframe::App for Admin {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

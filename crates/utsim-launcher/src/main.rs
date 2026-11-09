//! UTSim Launcher: starts the window (eframe) over [`utsim_launcher::janela`].
//!
//! `utsim-launcher --abrir <produto>` opens the active version of a product without showing
//! the launcher (this is what the shortcuts run); if that fails the window opens with the
//! reason.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::sync::Arc;

use eframe::egui;
use utsim_launcher::Env;
use utsim_launcher::catalogo::ICON_LAUNCHER;
use utsim_launcher::janela::App;
use utsim_launcher_core::install;

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
    let mut intro = true;
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
                    intro = false;
                }
            }
        }
        Some("--sem-animacao") => intro = false,
        Some("--versao" | "--version" | "-V") => {
            println!("utsim-launcher {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("--ajuda" | "--help" | "-h") => {
            println!(
                "utsim-launcher [--abrir <produto> | --sem-animacao]\n  sem argumentos: abre a janela do launcher\n  --abrir <produto>: abre a versão ativa do produto\n  --sem-animacao: abre a janela sem a animação de abertura"
            );
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("UTSim Launcher")
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([660.0, 440.0])
        .with_maximized(true);
    if let Some((w, h, rgba)) = utsim_ui_tema::familia::png_rgba(ICON_LAUNCHER) {
        viewport = viewport.with_icon(Arc::new(egui::IconData {
            rgba,
            width: w as u32,
            height: h as u32,
        }));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        "UTSim Launcher",
        options,
        Box::new(move |cc| {
            utsim_ui_tema::familia::install(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx, env, first_message, intro)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro na janela: {e}");
            ExitCode::FAILURE
        }
    }
}

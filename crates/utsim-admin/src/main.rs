//! UTSim Admin: the author's window over `utsim-publicar` (testers, publications, access,
//! keys). Runs on the author's machine only; it is not in the launcher's installer.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use utsim_admin::Admin;

const ICON: &[u8] = include_bytes!("../../../assets/icone/familia/launcher_256.png");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(
        args.first().map(String::as_str),
        Some("--ajuda" | "--help" | "-h")
    ) {
        println!(
            "utsim-admin [--dados <pasta>]\n  abre a janela do administrador (testadores, publicações, acesso, chaves)\n  --dados: pasta dos dados privados (padrão: %APPDATA%\\UTSim, ou UTSIM_PUBLICAR_DADOS)"
        );
        return ExitCode::SUCCESS;
    }
    let data = match args.first().map(String::as_str) {
        Some("--dados") => args.get(1).map(std::path::PathBuf::from),
        _ => None,
    }
    .or_else(utsim_publicar::default_data_dir);
    let Some(data) = data else {
        eprintln!("erro: não sei onde estão os dados; use --dados <pasta>");
        return ExitCode::FAILURE;
    };
    let options = eframe::NativeOptions {
        viewport: utsim_ui_tema::janela::viewport(
            "UTSim Admin",
            [1180.0, 760.0],
            [760.0, 480.0],
            Some(ICON),
        ),
        ..Default::default()
    };
    let result = eframe::run_native(
        "UTSim Admin",
        options,
        Box::new(move |cc| {
            utsim_ui_tema::familia::install(&cc.egui_ctx);
            Ok(Box::new(Admin::open(data)))
        }),
    );
    utsim_ui_tema::janela::saida(result)
}

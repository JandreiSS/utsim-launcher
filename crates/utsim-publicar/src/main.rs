use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::manifest;
use utsim_launcher_core::signature;
use utsim_publicar::{Output, PublishRequest, Source, Store};

#[derive(Parser)]
#[command(
    name = "utsim-publicar",
    version,
    about = "Ferramenta do autor do launcher do UTSim: chaves, testadores, pacotes cifrados e manifesto assinado.\nSó escreve arquivos locais; não envia nada à internet."
)]
struct Cli {
    /// Pasta dos dados privados (padrão: %APPDATA%\UTSim, ou UTSIM_PUBLICAR_DADOS)
    #[arg(long, global = true)]
    dados: Option<PathBuf>,
    /// Aceita uma pasta de dados dentro de um repositório git (não recomendado)
    #[arg(long, global = true, hide = true)]
    permitir_repo: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Par de chaves de assinatura
    Chaves {
        #[command(subcommand)]
        acao: Chaves,
    },
    /// Cadastro de testadores e seus códigos de acesso
    Testador {
        #[command(subcommand)]
        acao: Testador,
    },
    /// Cifra e assina um pacote e atualiza o manifesto na pasta de saída
    Publicar(Box<PublicarArgs>),
    /// Grava no manifesto o link do Drive de um pacote já sincronizado
    Link {
        #[arg(long)]
        saida: PathBuf,
        #[arg(long)]
        produto: String,
        #[arg(long)]
        versao: String,
        #[arg(long)]
        plataforma: Option<String>,
        /// Link de compartilhamento do Drive ("qualquer pessoa com o link")
        link: String,
    },
    /// Refaz o acesso às versões publicadas para os testadores ativos agora
    Reembrulhar {
        #[arg(long)]
        saida: PathBuf,
        #[arg(long)]
        produto: Option<String>,
        #[arg(long, requires = "produto")]
        versao: Option<String>,
    },
    /// Tira uma versão do manifesto (o arquivo do pacote fica; apague à mão se quiser)
    Remover {
        #[arg(long)]
        saida: PathBuf,
        #[arg(long)]
        produto: String,
        #[arg(long)]
        versao: String,
    },
    /// Confere assinatura, links e arquivos da pasta de publicação
    Verificar {
        #[arg(long)]
        saida: PathBuf,
    },
}

#[derive(clap::Args)]
struct PublicarArgs {
    /// Pasta de publicação (a pasta sincronizada do Google Drive)
    #[arg(long)]
    saida: PathBuf,
    /// Id do produto (utsim, utsim-3d, ...)
    #[arg(long)]
    produto: String,
    /// Nome mostrado aos testadores (obrigatório num produto novo)
    #[arg(long)]
    nome: Option<String>,
    #[arg(long)]
    descricao: Option<String>,
    #[arg(long)]
    versao: String,
    /// Canal: estavel ou testes
    #[arg(long, default_value = manifest::TESTING)]
    canal: String,
    /// Plataforma do pacote (padrão: a desta máquina)
    #[arg(long)]
    plataforma: Option<String>,
    /// Pasta do programa (vira um .zip dentro do pacote)
    #[arg(long, conflicts_with = "zip", required_unless_present = "zip")]
    pasta: Option<PathBuf>,
    /// Um .zip pronto do programa
    #[arg(long)]
    zip: Option<PathBuf>,
    /// Programa a abrir, relativo à pasta (ex.: bin/utsim-view.exe)
    #[arg(long)]
    executavel: String,
    /// Argumento passado ao programa (repetível)
    #[arg(long = "arg", allow_hyphen_values = true)]
    args: Vec<String>,
    /// Arquivo com as notas da versão (Markdown)
    #[arg(long, conflicts_with = "notas_texto")]
    notas: Option<PathBuf>,
    /// Notas da versão no próprio comando
    #[arg(long)]
    notas_texto: Option<String>,
    /// Troca um pacote já publicado desta versão e plataforma
    #[arg(long)]
    substituir: bool,
}

#[derive(Subcommand)]
enum Chaves {
    /// Gera o par de chaves (uma vez só)
    Gerar,
    /// Mostra a chave pública (para embutir no launcher)
    Mostrar,
}

#[derive(Subcommand)]
enum Testador {
    /// Cadastra um testador e mostra o código de acesso dele
    Adicionar {
        nome: String,
        /// Canais permitidos (repetível; padrão: estavel e testes)
        #[arg(long = "canal")]
        canais: Vec<String>,
    },
    /// Lista os testadores
    Listar {
        /// Mostra também os códigos completos
        #[arg(long)]
        codigos: bool,
    },
    /// Revoga um testador (as próximas publicações não o incluem)
    Revogar { quem: String },
    /// Reativa um testador revogado
    Reativar { quem: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let dir = cli
        .dados
        .or_else(utsim_publicar::default_data_dir)
        .ok_or("não sei onde guardar os dados; use --dados")?;
    let store = Store::open(dir, cli.permitir_repo).map_err(|e| e.to_string())?;
    let platform = |p: Option<String>| p.unwrap_or_else(|| manifest::current_platform().into());
    let e = |e: utsim_launcher_core::Error| e.to_string();
    match cli.cmd {
        Cmd::Chaves {
            acao: Chaves::Gerar,
        } => {
            let public = store.generate_keys().map_err(e)?;
            println!("Par de chaves gerado.");
            println!("  privada: {}", store.private_key_path().display());
            println!("  pública: {}", store.public_key_path().display());
            println!();
            println!("AVISO: a chave privada é o que prova que uma versão veio de você.");
            println!("  - Nunca a coloque num repositório, no Drive ou em e-mail.");
            println!("  - Guarde uma cópia num pendrive ou gerenciador de senhas: sem ela,");
            println!("    só um launcher novo (com outra chave) aceita versões novas.");
            println!();
            println!("Chave pública (embutida no launcher):");
            println!("  {}", signature::public_to_text(&public));
            println!(
                "Copie-a para crates/utsim-launcher/confianca/chave_publica.txt antes de gerar o launcher."
            );
        }
        Cmd::Chaves {
            acao: Chaves::Mostrar,
        } => {
            let key = store.signing_key().map_err(e)?;
            let public = key.verifying_key();
            println!("{}", signature::public_to_text(&public));
            println!("id: {}", signature::key_id(&public));
        }
        Cmd::Testador {
            acao: Testador::Adicionar { nome, canais },
        } => {
            let canais = if canais.is_empty() {
                vec![manifest::STABLE.to_string(), manifest::TESTING.to_string()]
            } else {
                canais
            };
            let t = store.add_tester(&nome, &canais).map_err(e)?;
            println!(
                "Testador cadastrado: {} (id {}, canais {})",
                t.name,
                t.id,
                t.channels.join(", ")
            );
            println!();
            println!("  Código de acesso: {}", t.code);
            println!();
            println!("Envie o código só para essa pessoa, por um canal privado.");
            println!(
                "Ele entra nas próximas publicações; para as já publicadas rode 'utsim-publicar reembrulhar --saida <pasta>'."
            );
        }
        Cmd::Testador {
            acao: Testador::Listar { codigos },
        } => {
            let list = store.testers().map_err(e)?;
            if list.is_empty() {
                println!("Nenhum testador cadastrado.");
            }
            for t in list {
                let status = if t.active { "ativo" } else { "REVOGADO" };
                print!(
                    "{:<4}  {:<30} {:<9} canais: {}",
                    t.id,
                    t.name,
                    status,
                    t.channels.join(", ")
                );
                if codigos {
                    print!("  código: {}", t.code);
                }
                println!();
            }
        }
        Cmd::Testador {
            acao: Testador::Revogar { quem },
        } => {
            let t = store.set_active(&quem, false).map_err(e)?;
            println!(
                "{} revogado. As próximas publicações não o incluem.",
                t.name
            );
            println!(
                "Versões já publicadas continuam abertas para ele; para tirar o acesso às que ele ainda não baixou, rode 'utsim-publicar reembrulhar --saida <pasta>'."
            );
        }
        Cmd::Testador {
            acao: Testador::Reativar { quem },
        } => {
            let t = store.set_active(&quem, true).map_err(e)?;
            println!("{} reativado.", t.name);
        }
        Cmd::Publicar(a) => {
            let PublicarArgs {
                saida,
                produto,
                nome,
                descricao,
                versao,
                canal,
                plataforma,
                pasta,
                zip,
                executavel,
                args,
                notas,
                notas_texto,
                substituir,
            } = *a;
            let notes = match (notas, notas_texto) {
                (Some(p), _) => std::fs::read_to_string(&p)
                    .map_err(|err| format!("notas {}: {err}", p.display()))?,
                (None, Some(t)) => t,
                (None, None) => String::new(),
            };
            let source = match (pasta, zip) {
                (Some(p), _) => Source::Folder(p),
                (None, Some(z)) => Source::Zip(z),
                (None, None) => return Err("informe --pasta ou --zip".into()),
            };
            let req = PublishRequest {
                product: produto.clone(),
                name: nome,
                description: descricao,
                version: versao.clone(),
                channel: canal,
                platform: platform(plataforma),
                source,
                executable: executavel,
                args,
                notes,
                replace: substituir,
            };
            let out = Output { dir: saida };
            println!(
                "Cifrando e embrulhando a chave para cada testador (alguns segundos por testador)..."
            );
            let p =
                utsim_publicar::publish(&store, &out, &req, Argon2Params::DEFAULT).map_err(e)?;
            println!("Pacote: {} ({} bytes)", p.package_file.display(), p.size);
            println!("Testadores com acesso: {}", p.wraps);
            println!(
                "Manifesto: {} (sequência {})",
                out.manifest_path().display(),
                p.sequence
            );
            println!();
            println!("Próximos passos:");
            println!("  1. Espere o Google Drive para computador enviar o pacote.");
            println!(
                "  2. No Drive, compartilhe o pacote como \"qualquer pessoa com o link\" e copie o link."
            );
            println!(
                "  3. utsim-publicar link --saida \"{}\" --produto {produto} --versao {versao} --plataforma {} \"<link>\"",
                out.dir.display(),
                req.platform
            );
        }
        Cmd::Link {
            saida,
            produto,
            versao,
            plataforma,
            link,
        } => {
            let out = Output { dir: saida };
            let url = utsim_publicar::set_link(
                &store,
                &out,
                &produto,
                &versao,
                &platform(plataforma),
                &link,
            )
            .map_err(e)?;
            println!("Link gravado: {url}");
            println!("O manifesto foi reassinado; o Drive sincroniza sozinho.");
        }
        Cmd::Reembrulhar {
            saida,
            produto,
            versao,
        } => {
            let out = Output { dir: saida };
            let only = produto.as_deref().map(|p| (p, versao.as_deref()));
            let n = utsim_publicar::rewrap(&store, &out, only, Argon2Params::DEFAULT).map_err(e)?;
            println!(
                "{n} pacote(s) reembrulhado(s) para os testadores ativos; manifesto reassinado."
            );
        }
        Cmd::Remover {
            saida,
            produto,
            versao,
        } => {
            let out = Output { dir: saida };
            utsim_publicar::remove_release(&store, &out, &produto, &versao).map_err(e)?;
            println!(
                "{produto} {versao} saiu do manifesto. O arquivo do pacote continua na pasta."
            );
        }
        Cmd::Verificar { saida } => {
            let out = Output { dir: saida };
            let (report, problems) = utsim_publicar::check(&store, &out).map_err(e)?;
            print!("{report}");
            if problems > 0 {
                return Err(format!("{problems} problema(s) encontrado(s)"));
            }
        }
    }
    Ok(())
}

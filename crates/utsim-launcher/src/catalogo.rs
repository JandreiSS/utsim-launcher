//! What the launcher shows for each product: the look (accent colour, subtitle, author,
//! icon) from the manifest with built-in defaults, and the state on this machine (installed,
//! update available, no access...). No window here: the cards and rows in [`crate::janela`]
//! only read these.

use crate::texto::{fx, sx, tx, vx};
use utsim_launcher_core::code::AccessCode;
use utsim_launcher_core::install::{self, Layout};
use utsim_launcher_core::manifest::{self, Manifest, Product, Release};
use utsim_launcher_core::state::{LauncherState, SortOrder};

/// Built-in look of the products known when this launcher was built. A product published
/// later brings its own look in the manifest (`cor`, `subtitulo`, `autor`, `icone`).
pub struct Builtin {
    pub id: &'static str,
    pub color: [u8; 3],
    /// Key of the subtitle in `lancador.ftl`.
    pub subtitle: &'static str,
    pub icon: Option<&'static [u8]>,
}

/// Icons of the family, 256 x 256 PNG with transparent rounded corners.
pub const ICON_UTSIM: &[u8] = include_bytes!("../../../assets/icone/familia/utsim_256.png");
pub const ICON_SEIS_SIGMA: &[u8] =
    include_bytes!("../../../assets/icone/familia/lean-seis-sigma_256.png");
pub const ICON_ENSINO: &[u8] = include_bytes!("../../../assets/icone/familia/ensino_256.png");
pub const ICON_QUALIDADE: &[u8] = include_bytes!("../../../assets/icone/familia/qualidade_256.png");
pub const ICON_SEGURANCA: &[u8] = include_bytes!("../../../assets/icone/familia/seguranca_256.png");
pub const ICON_ESTATISTICA: &[u8] =
    include_bytes!("../../../assets/icone/familia/estatistica_256.png");
/// The launcher's own icon (window and taskbar).
/// The official logo (the three metallic gears), for the top bar.
pub const LOGO_UTSIM: &[u8] = include_bytes!("../../../assets/icone/utsim-128.png");
pub const ICON_LAUNCHER: &[u8] = include_bytes!("../../../assets/icone/familia/launcher_256.png");

pub const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "utsim",
        color: [0x22, 0xB5, 0xFF],
        subtitle: "catalogo-simulacao_de_processos",
        icon: Some(ICON_UTSIM),
    },
    Builtin {
        id: "utsim-3d",
        color: [0x22, 0xB5, 0xFF],
        subtitle: "catalogo-previa_3d_do_modelo",
        icon: Some(ICON_UTSIM),
    },
    Builtin {
        id: "lean-seis-sigma",
        color: [0x4F, 0xD0, 0x6A],
        subtitle: "catalogo-projetos_lean_seis_sigma",
        icon: Some(ICON_SEIS_SIGMA),
    },
    Builtin {
        id: "ensino",
        color: [0x8A, 0x5C, 0xE6],
        subtitle: "catalogo-ensino_pela_simulacao",
        icon: Some(ICON_ENSINO),
    },
    Builtin {
        id: "qualidade",
        color: [0x3A, 0x6E, 0xE0],
        subtitle: "catalogo-engenharia_da_qualidade",
        icon: Some(ICON_QUALIDADE),
    },
    Builtin {
        id: "seguranca",
        color: [0xF2, 0x8C, 0x1E],
        subtitle: "catalogo-seguranca_do_trabalho",
        icon: Some(ICON_SEGURANCA),
    },
    Builtin {
        id: "estatistica",
        color: [0x1F, 0xB5, 0xA3],
        subtitle: "catalogo-analise_estatistica",
        icon: Some(ICON_ESTATISTICA),
    },
];

/// Accent when neither the manifest nor the built-in list has one.
pub const DEFAULT_COLOR: [u8; 3] = [0x22, 0xB5, 0xFF];
/// Author when the manifest has none.
pub const DEFAULT_AUTHOR: &str = "UTSim";

pub fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTIN.iter().find(|b| b.id == id)
}

/// Where the product stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Installed, and a newer version this tester can install exists.
    Update { from: String, to: String },
    /// Installed and up to date (or nothing newer this tester can install).
    Installed { version: String },
    /// Not installed; `latest` can be installed.
    NotInstalled,
    /// Not installed and the tester has no access (no code, or not listed).
    NoAccess,
    /// No version for this system in the channel followed.
    NoPackage,
}

impl Status {
    /// Badge text.
    pub fn label(&self) -> &'static str {
        match self {
            Status::Update { .. } => tx("catalogo-atualizacao_disponivel"),
            Status::Installed { .. } => tx("catalogo-instalado"),
            Status::NotInstalled => tx("catalogo-nao_instalado"),
            Status::NoAccess => tx("catalogo-sem_acesso"),
            Status::NoPackage => tx("catalogo-sem_versao_para_este_sistema"),
        }
    }
}

/// One product as the cards and rows show it.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// The name in two tones: `(white part, accent part)`.
    pub name_parts: (String, String),
    pub color: [u8; 3],
    pub subtitle: String,
    pub description: String,
    pub author: String,
    /// PNG of the icon: the manifest's, else the built-in one.
    pub icon: Option<Vec<u8>>,
    /// Version that opens, if installed.
    pub active: Option<String>,
    /// Newest version visible in the channel with a package for this system.
    pub latest: Option<Release>,
    pub status: Status,
    /// `latest` is published open to everyone.
    pub open: bool,
    pub channel: String,
}

impl Item {
    /// Version line of the card: `(old, new)`; `old` is shown only when an update exists.
    pub fn versions(&self) -> (Option<String>, Option<String>) {
        match &self.status {
            Status::Update { from, to } => (Some(from.clone()), Some(to.clone())),
            Status::Installed { version } => (None, Some(version.clone())),
            _ => (None, self.latest.as_ref().map(|r| r.version.clone())),
        }
    }

    /// Date of the newest version, as `2 de nov. de 2026`.
    pub fn release_date(&self) -> Option<String> {
        self.latest
            .as_ref()
            .and_then(|r| date_text(&r.published_at))
    }

    pub fn has_update(&self) -> bool {
        matches!(self.status, Status::Update { .. })
    }

    /// Matches the search text (name, subtitle, id, description).
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty()
            || [&self.name, &self.subtitle, &self.id, &self.description]
                .iter()
                .any(|t| t.to_lowercase().contains(&q))
    }
}

/// Splits a name in two tones: "UTSim Seis Sigma" gives ("UTSim", "Seis Sigma"), "UTSim"
/// gives ("UT", "Sim"), one other word is all accent.
pub fn name_parts(name: &str) -> (String, String) {
    let name = name.trim();
    if name == "UTSim" {
        return ("UT".into(), "Sim".into());
    }
    match name.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.trim().to_string()),
        None => (String::new(), name.to_string()),
    }
}

/// `2026-11-02T12:00:00Z` as `2 de nov. de 2026` (in the language of the moment).
pub fn date_text(rfc3339: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "catalogo-mes_1",
        "catalogo-mes_2",
        "catalogo-mes_3",
        "catalogo-mes_4",
        "catalogo-mes_5",
        "catalogo-mes_6",
        "catalogo-mes_7",
        "catalogo-mes_8",
        "catalogo-mes_9",
        "catalogo-mes_10",
        "catalogo-mes_11",
        "catalogo-mes_12",
    ];
    let date = rfc3339.get(..10)?;
    let mut it = date.split('-');
    let y: u32 = it.next()?.parse().ok()?;
    let m: usize = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let month = MONTHS.get(m.checked_sub(1)?)?;
    Some(fx(
        "catalogo-data",
        &[
            ("dia", vx(&d)),
            ("mes", tx(month).into()),
            ("ano", sx(y.to_string())),
        ],
    ))
}

/// The item of one product.
pub fn item(
    product: &Product,
    layout: &Layout,
    state: &LauncherState,
    platform: &str,
    code: Option<&AccessCode>,
) -> Item {
    let channel = state.channel(&product.id).to_string();
    let builtin = builtin(&product.id);
    let active = install::active_version(layout, &product.id);
    let latest = product.latest(&channel, platform).cloned();
    let can = |r: &Release| {
        r.package(platform)
            .is_some_and(|p| crate::can_install(p, code))
    };
    let status = match (&active, &latest) {
        (Some(a), Some(l)) if manifest::compare_versions(&l.version, a).is_gt() && can(l) => {
            Status::Update {
                from: a.clone(),
                to: l.version.clone(),
            }
        }
        (Some(a), _) => Status::Installed { version: a.clone() },
        (None, Some(l)) if can(l) => Status::NotInstalled,
        (None, Some(_)) => Status::NoAccess,
        (None, None) => Status::NoPackage,
    };
    let color = product
        .color
        .as_deref()
        .and_then(manifest::parse_color)
        .or(builtin.map(|b| b.color))
        .unwrap_or(DEFAULT_COLOR);
    let text = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(String::from)
    };
    let subtitle = text(&product.subtitle)
        .or(builtin.map(|b| tx(b.subtitle).to_string()))
        .unwrap_or_default();
    let open = latest
        .as_ref()
        .and_then(|r| r.package(platform))
        .is_some_and(|p| p.is_open());
    Item {
        id: product.id.clone(),
        name: product.name.clone(),
        name_parts: name_parts(&product.name),
        color,
        subtitle,
        description: product.description.clone(),
        author: text(&product.author).unwrap_or_else(|| DEFAULT_AUTHOR.into()),
        icon: product
            .icon_png()
            .or_else(|| builtin.and_then(|b| b.icon).map(<[u8]>::to_vec)),
        active,
        latest,
        status,
        open,
        channel,
    }
}

/// Every app of `m` (not the launcher itself), filtered by `query` and sorted.
pub fn items(
    m: &Manifest,
    layout: &Layout,
    state: &LauncherState,
    platform: &str,
    code: Option<&AccessCode>,
    query: &str,
) -> Vec<Item> {
    let mut v: Vec<Item> = m
        .apps()
        .map(|p| item(p, layout, state, platform, code))
        .filter(|i| i.matches(query))
        .collect();
    sort(&mut v, state.sort);
    v
}

pub fn sort(v: &mut [Item], order: SortOrder) {
    let name = |i: &Item| i.name.to_lowercase();
    match order {
        SortOrder::Name => v.sort_by_key(name),
        SortOrder::Recent => v.sort_by(|a, b| {
            let date = |i: &Item| {
                i.latest
                    .as_ref()
                    .map(|r| r.published_at.clone())
                    .unwrap_or_default()
            };
            date(b).cmp(&date(a)).then_with(|| name(a).cmp(&name(b)))
        }),
        SortOrder::Updates => v.sort_by(|a, b| {
            b.has_update()
                .cmp(&a.has_update())
                .then_with(|| name(a).cmp(&name(b)))
        }),
    }
}

/// Whether some product needs a code this launcher does not have (the code screen is offered
/// then; products open to everyone never ask for one).
pub fn needs_code(items: &[Item]) -> bool {
    items.iter().any(|i| i.status == Status::NoAccess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_dates_and_defaults() {
        assert_eq!(
            name_parts("UTSim Seis Sigma"),
            ("UTSim".into(), "Seis Sigma".into())
        );
        assert_eq!(name_parts("UTSim"), ("UT".into(), "Sim".into()));
        assert_eq!(
            name_parts("Ferramenta"),
            (String::new(), "Ferramenta".into())
        );
        assert_eq!(
            date_text("2026-11-02T12:00:00Z").as_deref(),
            Some("2 de nov. de 2026")
        );
        assert_eq!(date_text("2026-13-02"), None);
        assert_eq!(date_text(""), None);
        assert!(builtin("lean-seis-sigma").is_some());
        assert!(builtin("qualidade").is_some());
        assert!(builtin("seguranca").is_some());
        assert!(builtin("estatistica").is_some());
        for b in BUILTIN {
            let png = b.icon.unwrap();
            assert!(png.starts_with(b"\x89PNG"), "{}", b.id);
            assert!(png.len() <= manifest::ICON_MAX_BYTES, "{}", b.id);
        }
    }
}

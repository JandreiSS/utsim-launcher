//! The GitHub flow with a fake `gh` that keeps releases and assets in memory.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use utsim_launcher_core::crypto::Argon2Params;
use utsim_launcher_core::manifest::{Manifest, TESTING};
use utsim_launcher_core::{Error, Result};
use utsim_publicar::github::{self, Gh};
use utsim_publicar::{Output, PublishRequest, Source, Store};

/// repo -> tag -> (prerelease, asset name -> (id, bytes))
type Releases = BTreeMap<String, BTreeMap<String, (bool, BTreeMap<String, (u64, Vec<u8>)>)>>;

#[derive(Default)]
struct FakeGh {
    releases: RefCell<Releases>,
    calls: RefCell<Vec<String>>,
    next_id: RefCell<u64>,
}

impl FakeGh {
    fn assets(&self, repo: &str, tag: &str) -> Vec<String> {
        self.releases.borrow()[repo][tag]
            .1
            .keys()
            .cloned()
            .collect()
    }
    fn tags(&self, repo: &str) -> Vec<String> {
        self.releases
            .borrow()
            .get(repo)
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default()
    }
    fn asset(&self, repo: &str, tag: &str, name: &str) -> Vec<u8> {
        self.releases.borrow()[repo][tag].1[name].1.clone()
    }
}

fn opt<'a>(args: &'a [&str], name: &str) -> Option<&'a str> {
    args.iter().position(|a| *a == name).map(|i| args[i + 1])
}

impl Gh for FakeGh {
    fn run(&self, args: &[&str]) -> Result<String> {
        self.calls.borrow_mut().push(args.join(" "));
        let mut rel = self.releases.borrow_mut();
        let fail = || Err(Error::Network("not found".into()));
        match args {
            ["release", "view", tag, ..] => {
                let repo = opt(args, "--repo").unwrap();
                if rel.get(repo).is_some_and(|r| r.contains_key(*tag)) {
                    Ok("{}".into())
                } else {
                    fail()
                }
            }
            ["release", "create", tag, ..] => {
                let repo = opt(args, "--repo").unwrap();
                let pre = args.contains(&"--prerelease");
                rel.entry(repo.into())
                    .or_default()
                    .insert(tag.to_string(), (pre, BTreeMap::new()));
                Ok(String::new())
            }
            ["release", "upload", tag, file, ..] => {
                let repo = opt(args, "--repo").unwrap();
                let name = Path::new(file)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let bytes = fs::read(file).unwrap();
                let mut id = self.next_id.borrow_mut();
                *id += 1;
                rel.get_mut(repo)
                    .unwrap()
                    .get_mut(*tag)
                    .unwrap()
                    .1
                    .insert(name, (*id, bytes));
                Ok(String::new())
            }
            ["release", "delete", tag, ..] => {
                let repo = opt(args, "--repo").unwrap();
                rel.get_mut(repo).unwrap().remove(*tag);
                Ok(String::new())
            }
            ["api", path, "--jq", _] => {
                let (repo, tag) = path
                    .strip_prefix("repos/")
                    .unwrap()
                    .split_once("/releases/tags/")
                    .unwrap();
                Ok(rel[repo][tag]
                    .1
                    .iter()
                    .map(|(n, (id, _))| format!("{id} {n}\n"))
                    .collect())
            }
            ["api", "-X", method, path, rest @ ..] => {
                let id: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                for r in rel.values_mut() {
                    for (_, assets) in r.values_mut() {
                        let Some(name) = assets
                            .iter()
                            .find(|(_, (i, _))| *i == id)
                            .map(|(n, _)| n.clone())
                        else {
                            continue;
                        };
                        let a = assets.remove(&name).unwrap();
                        if *method == "PATCH" {
                            let new = rest[1].strip_prefix("name=").unwrap();
                            assets.insert(new.into(), a);
                        }
                        return Ok(String::new());
                    }
                }
                fail()
            }
            _ => panic!("unexpected gh call: {args:?}"),
        }
    }
}

fn publish(store: &Store, out: &Output, src: &Path, version: &str) {
    utsim_publicar::publish(
        store,
        out,
        &PublishRequest {
            product: "qualidade".into(),
            name: Some("UTSim Qualidade".into()),
            description: None,
            version: version.into(),
            channel: TESTING.into(),
            platform: "windows-x64".into(),
            source: Source::Folder(src.to_path_buf()),
            executable: "bin/q.exe".into(),
            args: vec![],
            notes: format!("Novidades da {version}"),
            replace: false,
            appearance: Default::default(),
        },
        Argon2Params::TEST,
    )
    .unwrap();
}

#[test]
fn upload_links_prunes_and_sends_the_manifest() {
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    fs::create_dir_all(src.path().join("bin")).unwrap();
    fs::write(src.path().join("bin/q.exe"), b"exe").unwrap();
    let store = Store::open(data.path(), false).unwrap();
    store.generate_keys().unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let out = Output {
        dir: dir.path().join("saida"),
    };
    let gh = FakeGh::default();
    let repo = github::full_repo("qualidade", None).unwrap();
    assert_eq!(repo, "JandreiSS/utsim-qualidade");
    let launcher_repo = "JandreiSS/utsim-launcher";

    // An old version still on the Drive.
    publish(&store, &out, src.path(), "0.9.0");
    utsim_publicar::set_link(
        &store,
        &out,
        "qualidade",
        "0.9.0",
        "windows-x64",
        "https://drive.google.com/file/d/1AbCdEfGhIjKlMnOpQrStUvWxYz012345/view",
    )
    .unwrap();

    for v in ["0.10.0", "0.10.1", "0.11.0"] {
        publish(&store, &out, src.path(), v);
        let r = github::upload(&store, &out, &gh, "qualidade", v, "windows-x64", &repo, 3).unwrap();
        assert_eq!(
            r.url,
            format!(
                "https://github.com/{repo}/releases/download/v{v}/qualidade-{v}-windows-x64.utpk"
            )
        );
        if v == "0.11.0" {
            assert_eq!(r.pruned, vec!["0.9.0".to_string()]);
            assert_eq!(r.elsewhere.len(), 1);
            assert!(r.elsewhere[0].1.contains("drive"));
        } else {
            assert!(r.pruned.is_empty());
        }
    }
    assert_eq!(gh.tags(&repo), vec!["v0.10.0", "v0.10.1", "v0.11.0"]);
    assert_eq!(
        gh.assets(&repo, "v0.11.0"),
        vec!["qualidade-0.11.0-windows-x64.utpk"]
    );
    assert!(
        gh.releases.borrow()[&repo]["v0.11.0"].0,
        "testes is a pre-release"
    );

    // The manifest on GitHub is the signed local one, under its final name only.
    assert_eq!(
        gh.assets(launcher_repo, "manifesto"),
        vec!["manifesto.json"]
    );
    let sent = gh.asset(launcher_repo, "manifesto", "manifesto.json");
    assert_eq!(sent, fs::read(out.manifest_path()).unwrap());
    let key = store.signing_key().unwrap();
    let m =
        Manifest::from_signed(std::str::from_utf8(&sent).unwrap(), &[key.verifying_key()]).unwrap();
    let versions: Vec<_> = m
        .product("qualidade")
        .unwrap()
        .releases
        .iter()
        .map(|r| r.version.clone())
        .collect();
    assert_eq!(versions.len(), 3);
    assert!(!versions.contains(&"0.9.0".to_string()));

    // A fourth GitHub version deletes the oldest GitHub release, after the manifest is sent.
    publish(&store, &out, src.path(), "0.12.0");
    let r = github::upload(
        &store,
        &out,
        &gh,
        "qualidade",
        "0.12.0",
        "windows-x64",
        &repo,
        3,
    )
    .unwrap();
    assert_eq!(r.pruned, vec!["0.10.0".to_string()]);
    assert!(r.elsewhere.is_empty());
    assert_eq!(gh.tags(&repo), vec!["v0.10.1", "v0.11.0", "v0.12.0"]);
    let calls = gh.calls.borrow();
    let send = calls
        .iter()
        .rposition(|c| c.contains("name=manifesto.json"))
        .unwrap();
    let delete = calls
        .iter()
        .rposition(|c| c.starts_with("release delete v0.10.0"))
        .unwrap();
    assert!(send < delete);
    assert_eq!(
        gh.assets(launcher_repo, "manifesto"),
        vec!["manifesto.json"]
    );
}

#[test]
fn refuses_a_package_that_does_not_match_the_manifest() {
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    fs::create_dir_all(src.path().join("bin")).unwrap();
    fs::write(src.path().join("bin/q.exe"), b"exe").unwrap();
    let store = Store::open(data.path(), false).unwrap();
    store.generate_keys().unwrap();
    store.add_tester("Ana", &[TESTING.into()]).unwrap();
    let out = Output {
        dir: dir.path().join("saida"),
    };
    publish(&store, &out, src.path(), "1.0.0");
    fs::write(
        out.dir.join("pacotes/qualidade-1.0.0-windows-x64.utpk"),
        b"outro",
    )
    .unwrap();
    let gh = FakeGh::default();
    let err = github::upload(
        &store,
        &out,
        &gh,
        "qualidade",
        "1.0.0",
        "windows-x64",
        "o/r",
        3,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Integrity(_)), "{err}");
    assert!(gh.calls.borrow().is_empty(), "nothing is sent");
}

#[test]
fn manifest_url_and_repos() {
    assert_eq!(
        github::manifest_url(),
        "https://github.com/JandreiSS/utsim-launcher/releases/download/manifesto/manifesto.json"
    );
    // The address built into the launcher is the one this tool sends to.
    assert_eq!(
        include_str!("../../utsim-launcher/confianca/manifesto_url.txt").trim(),
        github::manifest_url()
    );
    for p in [
        "launcher",
        "utsim",
        "lean-seis-sigma",
        "qualidade",
        "seguranca",
        "estatistica",
        "ensino",
        "produto",
    ] {
        assert!(github::repo_for(p).is_some(), "{p}");
    }
    assert!(github::full_repo("desconhecido", None).is_err());
    assert_eq!(github::full_repo("x", Some("outro/r")).unwrap(), "outro/r");
    assert_eq!(
        github::tag_of_asset(
            "https://github.com/o/r/releases/download/v1.0/a.utpk",
            "o/r"
        ),
        Some("v1.0")
    );
}

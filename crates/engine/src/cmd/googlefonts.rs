//! Type › Browse Google Fonts…: the open-licence families of the google/fonts repository on
//! GitHub (OFL, Apache and UFL), listed from the repository's file tree (`text.googleFonts`) and
//! downloaded into VectorCraft's own Fonts folder (`text.installGoogleFont`), with their licence.
//!
//! The engine does no networking of its own: the desktop app installs a downloader
//! ([`set_fetcher`]). Without one (the web, tests) the commands answer `unavailable`. Downloads run
//! on threads of their own; `text.googleFonts` reports them and catalogs the fonts once they land.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};

use super::*;

/// Downloads the body at a URL (GET), at most `max` bytes, for [`set_fetcher`].
pub type Fetch = fn(url: &str, max: u64) -> std::result::Result<Vec<u8>, String>;

static FETCHER: OnceLock<Fetch> = OnceLock::new();

/// Have `text.googleFonts` and `text.installGoogleFont` download with `f`. Only the first call
/// counts.
pub fn set_fetcher(f: Fetch) {
    // A second call keeps the first downloader, as documented.
    let _ = FETCHER.set(f);
}

/// The licence folders of the repository, each a folder of families.
const LICENCES: &[&str] = &["ofl", "apache", "ufl"];
/// The licence files kept next to a family's fonts.
const LICENCE_FILES: &[&str] = &["OFL.txt", "LICENSE.txt", "UFL.txt", "LICENCE.txt"];
/// Families shown first while nothing is searched for: Adobe's open-source Source fonts (Source Han
/// is Noto Sans/Serif CJK), and Arimo, Tinos and Cousine, which have the metrics of the PDF
/// standard fonts Helvetica, Times and Courier.
pub const FEATURED: &[&str] = &[
    "ofl/sourcesans3",
    "ofl/sourceserif4",
    "ofl/sourcecodepro",
    "ofl/notosansjp",
    "ofl/notoserifjp",
    "ofl/notosanssc",
    "ofl/notosanskr",
    "ofl/arimo",
    "ofl/tinos",
    "ofl/cousine",
];
const TREE_URL: &str = "https://api.github.com/repos/google/fonts/git/trees/main:";
const RAW_URL: &str = "https://raw.githubusercontent.com/google/fonts/main/";
/// The largest file tree read (the OFL folder's is about 5 MB).
const MAX_TREE: u64 = 64 << 20;
/// The most families listed in one answer.
const MAX_LIMIT: usize = 1_000;
/// The longest query (bytes).
const MAX_QUERY: usize = 256;
/// The folder inside VectorCraft's Fonts folder the families go to, one folder each.
pub const FOLDER: &str = "Google Fonts";
/// The catalog kept between runs, next to the Fonts folder.
const CACHE: &str = "google-fonts-catalog.json";

/// A family of the repository.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Family {
    /// `licence/folder`, e.g. `ofl/sourcesans3`.
    pub id: String,
    /// The family's name, from its file names (the font's own name may differ a little).
    pub name: String,
    /// Its font files and licence file, in its folder.
    pub files: Vec<String>,
}

impl Family {
    fn fonts(&self) -> impl Iterator<Item = &String> {
        self.files.iter().filter(|f| is_font_name(f))
    }
}

#[derive(Default)]
struct State {
    catalog: Option<Arc<Vec<Family>>>,
    loading: bool,
    error: Option<String>,
    installs: Vec<Install>,
}

#[derive(Clone)]
struct Install {
    id: String,
    /// None while downloading; then the files written, or why it failed.
    done: Option<std::result::Result<Vec<PathBuf>, String>>,
    /// Its fonts were cataloged ([`super::fonts::rescan`]).
    cataloged: bool,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.googleFonts",
            "Google Fonts",
            [],
            None,
            "{query?: text in the family's name, refresh?: false (true: download the catalog again), limit?: 100 (1 to 1000)} the open-licence families of the google/fonts repository: the catalog is downloaded once (on a thread; state: loading) and kept next to VectorCraft's Fonts folder; while query is empty the featured families (the open-source Source fonts, Noto CJK, and Arimo, Tinos and Cousine with the metrics of Helvetica, Times and Courier) come first. Downloads that finished are cataloged so their fonts are listed. Not available without a downloader (the web) → {state: unavailable|loading|ready|failed, error?, total, families: [{id, name, licence, fonts, featured, installed}], installs: [{id, state: downloading|done|failed, files?, error?}]}",
            always,
            list
        ),
        cmd!(
            query "text.installGoogleFont",
            "Add Google Font",
            [],
            None,
            "{id: a family's id from text.googleFonts (licence/folder)} download the family's font files and licence on a thread into VectorCraft's Fonts folder › Google Fonts › folder, never replacing a file there; text.googleFonts reports the download and catalogs the fonts once it is done → {id, state: downloading}",
            always,
            install
        ),
    ]
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.googleFonts";
    let query = str_param(p, "query").unwrap_or("");
    if query.len() > MAX_QUERY {
        return Err(bad(C, "`query` is longer than 256 bytes"));
    }
    let limit = match p.get("limit") {
        None | Some(Value::Null) => 100,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| bad(C, "`limit` must be 1 to 1000"))?,
    };
    let refresh = p.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    let Some(&fetch) = FETCHER.get() else {
        return Ok(
            json!({ "state": "unavailable", "error": "Google Fonts downloads need the desktop app", "total": 0, "families": [], "installs": [] }),
        );
    };
    // Downloads that landed since: their fonts are listed and type set in them redraws.
    let landed = {
        let mut st = state();
        let mut any = false;
        for i in st.installs.iter_mut().filter(|i| matches!(i.done, Some(Ok(_))) && !i.cataloged) {
            i.cataloged = true;
            any = true;
        }
        any
    };
    if landed {
        super::fonts::rescan(s, &Value::Null)?;
    }
    let (catalog, loading, error, installs) = {
        let mut st = state();
        if st.catalog.is_none() && !st.loading && !refresh {
            st.catalog = read_cache().map(Arc::new);
        }
        if (st.catalog.is_none() || refresh) && !st.loading {
            st.loading = true;
            st.error = None;
            let spawned = std::thread::Builder::new().name("google-fonts-catalog".into()).spawn(move || {
                let r = download_catalog(fetch);
                let mut st = state();
                st.loading = false;
                match r {
                    Ok(c) => {
                        write_cache(&c);
                        st.catalog = Some(Arc::new(c));
                    }
                    Err(e) => st.error = Some(e),
                }
            });
            if let Err(e) = spawned {
                st.loading = false;
                st.error = Some(e.to_string());
            }
        }
        (st.catalog.clone(), st.loading, st.error.clone(), st.installs.clone())
    };
    let state_name = match (&catalog, loading, &error) {
        (_, true, _) => "loading",
        (Some(_), _, _) => "ready",
        (None, _, Some(_)) => "failed",
        (None, _, None) => "loading",
    };
    let families = catalog.as_deref().map(|c| search(c, query)).unwrap_or_default();
    let installed_dir = vectorcraft_text::app_font_dir().map(|d| d.join(FOLDER));
    let db = vectorcraft_text::FontDb::global();
    let rows: Vec<Value> = families
        .iter()
        .take(limit)
        .map(|f| {
            let folder = f.id.split_once('/').map_or("", |(_, d)| d);
            let installed = installed_dir.as_ref().is_some_and(|d| d.join(folder).is_dir()) || db.has_family(&f.name);
            json!({
                "id": f.id, "name": f.name, "licence": f.id.split('/').next().unwrap_or_default(),
                "fonts": f.fonts().count(), "featured": FEATURED.contains(&f.id.as_str()), "installed": installed,
            })
        })
        .collect();
    let installs: Vec<Value> = installs
        .iter()
        .map(|i| match &i.done {
            None => json!({ "id": i.id, "state": "downloading" }),
            Some(Ok(files)) => json!({ "id": i.id, "state": "done", "files": files.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>() }),
            Some(Err(e)) => json!({ "id": i.id, "state": "failed", "error": e }),
        })
        .collect();
    let mut out = json!({ "state": state_name, "total": families.len(), "families": rows, "installs": installs });
    if let Some(e) = error {
        out["error"] = json!(e);
    }
    Ok(out)
}

fn install(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.installGoogleFont";
    let id = str_param(p, "id").ok_or_else(|| bad(C, "give `id`, a family's id from text.googleFonts"))?.to_string();
    let Some(&fetch) = FETCHER.get() else {
        return Err(EngineError::Other("Google Fonts downloads need the desktop app".into()));
    };
    let dir = vectorcraft_text::app_font_dir()
        .ok_or_else(|| EngineError::Other("VectorCraft's Fonts folder isn't set here: the desktop app and vectorcraft-cli set it".into()))?
        .join(FOLDER);
    let mut st = state();
    let family = st
        .catalog
        .as_ref()
        .and_then(|c| c.iter().find(|f| f.id == id).cloned())
        .ok_or_else(|| bad(C, format!("`{id}` is not a family text.googleFonts listed")))?;
    if st.installs.iter().any(|i| i.id == id && i.done.is_none()) {
        return Ok(json!({ "id": id, "state": "downloading" }));
    }
    st.installs.retain(|i| i.id != id);
    st.installs.push(Install { id: id.clone(), done: None, cataloged: false });
    let thread_id = id.clone();
    let spawned = std::thread::Builder::new().name("google-fonts-download".into()).spawn(move || {
        let r = download_family(fetch, &family, &dir);
        if let Some(i) = state().installs.iter_mut().find(|i| i.id == thread_id) {
            i.done = Some(r);
        }
    });
    if let Err(e) = spawned {
        st.installs.retain(|i| i.id != id);
        return Err(EngineError::Other(e.to_string()));
    }
    Ok(json!({ "id": id, "state": "downloading" }))
}

/// The families whose name or folder holds `query` (any case, spaces ignored): the featured ones
/// first while it is empty, then by name.
fn search<'a>(catalog: &'a [Family], query: &str) -> Vec<&'a Family> {
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
    let q = squash(query);
    let mut out: Vec<&Family> = catalog.iter().filter(|f| q.is_empty() || squash(&f.name).contains(&q) || f.id.contains(&q)).collect();
    let rank = |f: &Family| if q.is_empty() { FEATURED.iter().position(|x| *x == f.id).unwrap_or(FEATURED.len()) } else { 0 };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn is_font_name(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    lower.ends_with(".ttf") || lower.ends_with(".otf")
}

/// A file or folder name from the tree that is safe to use as one: no separators, no `..`, and
/// only the characters the repository's names use.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '[' | ']' | ',' | '.' | ' '))
}

/// The family name a font file's name gives: `SourceSans3-Italic[wght].ttf` → `Source Sans 3`.
pub fn family_name(file: &str) -> String {
    let stem = file.split(['-', '[', '.']).next().unwrap_or(file);
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in stem.chars() {
        if let Some(p) = prev {
            let split = (c.is_ascii_uppercase() && p.is_ascii_lowercase()) || (c.is_ascii_digit() && p.is_ascii_alphabetic());
            if split {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// The catalog from the repository's file trees, one per licence folder (GitHub's API).
fn download_catalog(fetch: Fetch) -> std::result::Result<Vec<Family>, String> {
    let mut out = vec![];
    for licence in LICENCES {
        let body = fetch(&format!("{TREE_URL}{licence}?recursive=1"), MAX_TREE)?;
        let tree: Value = serde_json::from_slice(&body).map_err(|e| format!("the catalog isn't readable: {e}"))?;
        out.extend(parse_tree(licence, &tree));
    }
    if out.is_empty() {
        return Err("the catalog lists no fonts".into());
    }
    Ok(out)
}

/// The families in licence folder `licence`'s file tree (GitHub's `git/trees` answer): each
/// folder's own font files (not its `static` subfolder) and licence file.
pub fn parse_tree(licence: &str, tree: &Value) -> Vec<Family> {
    let mut families: Vec<Family> = vec![];
    for entry in tree["tree"].as_array().into_iter().flatten() {
        if entry["type"] != "blob" {
            continue;
        }
        let Some((folder, file)) = entry["path"].as_str().and_then(|p| p.split_once('/')) else { continue };
        if file.contains('/') || !safe_name(folder) || !safe_name(file) || !(is_font_name(file) || LICENCE_FILES.contains(&file)) {
            continue;
        }
        let id = format!("{licence}/{folder}");
        match families.last_mut().filter(|f| f.id == id) {
            Some(f) => f.files.push(file.to_string()),
            None => families.push(Family { id, name: String::new(), files: vec![file.to_string()] }),
        }
    }
    families.retain(|f| f.fonts().next().is_some());
    for f in &mut families {
        let name = f.fonts().next().map(|file| family_name(file)).unwrap_or_default();
        f.name = name;
    }
    families
}

/// Download `family`'s files into `dir`/its folder, never replacing a file there → the files
/// written (or kept: there already).
fn download_family(fetch: Fetch, family: &Family, dir: &Path) -> std::result::Result<Vec<PathBuf>, String> {
    let (licence, folder) = family.id.split_once('/').ok_or("not a family id")?;
    if !safe_name(folder) {
        return Err("not a family folder".into());
    }
    let to = dir.join(folder);
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    let mut written = vec![];
    for file in family.files.iter().filter(|f| safe_name(f)) {
        let path = to.join(file);
        if path.exists() {
            written.push(path);
            continue;
        }
        let bytes = fetch(&format!("{RAW_URL}{licence}/{folder}/{file}"), super::fontfiles::MAX_FONT_FILE)?;
        if is_font_name(file) && !looks_like_font(&bytes) {
            return Err(format!("{file} isn't a font file"));
        }
        // Written beside, then renamed: a half-written font is never scanned.
        let part = to.join(format!(".{file}.part"));
        std::fs::write(&part, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        written.push(path);
    }
    Ok(written)
}

/// Does `bytes` start as a TrueType, OpenType or collection font file does?
fn looks_like_font(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf"))
}

fn cache_path() -> Option<PathBuf> {
    vectorcraft_text::app_font_dir().map(|d| d.with_file_name(CACHE))
}

fn read_cache() -> Option<Vec<Family>> {
    let bytes = crate::store::read_capped(&cache_path()?, MAX_TREE).ok()?;
    serde_json::from_slice::<Vec<Family>>(&bytes).ok().filter(|c| !c.is_empty())
}

fn write_cache(catalog: &[Family]) {
    if let (Some(path), Ok(bytes)) = (cache_path(), serde_json::to_vec(catalog)) {
        // Only a cache: without it the catalog is downloaded again next time.
        let _ = std::fs::write(path, bytes);
    }
}

/// Forget the catalog and the downloads (tests).
#[cfg(test)]
pub(crate) fn reset() {
    *state() = State::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_come_from_file_names() {
        assert_eq!(family_name("SourceSans3-Italic[wght].ttf"), "Source Sans 3");
        assert_eq!(family_name("SourceSerif4[opsz,wght].ttf"), "Source Serif 4");
        assert_eq!(family_name("NotoSansJP[wght].ttf"), "Noto Sans JP");
        assert_eq!(family_name("Tinos-Regular.ttf"), "Tinos");
    }

    #[test]
    fn the_tree_lists_each_folders_fonts_and_licence() {
        let tree = json!({ "tree": [
            {"path": "arimo", "type": "tree"},
            {"path": "arimo/Arimo[wght].ttf", "type": "blob"},
            {"path": "arimo/OFL.txt", "type": "blob"},
            {"path": "arimo/METADATA.pb", "type": "blob"},
            {"path": "arimo/static/Arimo-Bold.ttf", "type": "blob"},
            {"path": "docsonly/OFL.txt", "type": "blob"},
            {"path": "../evil/x.ttf", "type": "blob"},
            {"path": "bad/..ttf", "type": "blob"},
            {"path": 3, "type": "blob"},
        ]});
        let families = parse_tree("ofl", &tree);
        assert_eq!(families, vec![Family { id: "ofl/arimo".into(), name: "Arimo".into(), files: vec!["Arimo[wght].ttf".into(), "OFL.txt".into()] }]);
        assert!(parse_tree("ofl", &json!({})).is_empty());
    }

    #[test]
    fn search_puts_featured_families_first_and_ignores_case_and_spaces() {
        let fam = |id: &str, name: &str| Family { id: id.into(), name: name.into(), files: vec!["a.ttf".into()] };
        let c = vec![fam("ofl/abel", "Abel"), fam("ofl/tinos", "Tinos"), fam("ofl/sourcesans3", "Source Sans 3")];
        let ids = |v: Vec<&Family>| v.iter().map(|f| f.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(search(&c, "")), ["ofl/sourcesans3", "ofl/tinos", "ofl/abel"]);
        assert_eq!(ids(search(&c, "SOURCESANS")), ["ofl/sourcesans3"]);
        assert_eq!(ids(search(&c, "source sans 3")), ["ofl/sourcesans3"]);
        assert!(search(&c, "zzz").is_empty());
    }

    #[test]
    fn without_a_downloader_it_is_unavailable() {
        reset();
        let mut s = Session::new();
        let r = s.execute("text.googleFonts", &json!({})).unwrap();
        assert_eq!(r["state"], "unavailable");
        assert!(s.execute("text.installGoogleFont", &json!({"id": "ofl/arimo"})).is_err());
        for junk in [json!({"limit": 0}), json!({"limit": 1001}), json!({"limit": "x"}), json!({"query": "q".repeat(300)})] {
            assert!(s.execute("text.googleFonts", &junk).is_err(), "{junk}");
        }
    }

    #[test]
    fn only_font_files_are_accepted() {
        assert!(looks_like_font(&[0, 1, 0, 0, 9]) && looks_like_font(b"OTTO..") && !looks_like_font(b"<html>") && !looks_like_font(b""));
    }
}

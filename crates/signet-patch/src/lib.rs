//! Signet's retail patcher for FINAL FANTASY XI: brings an install up to
//! Square Enix's current version without PlayOnline Viewer.
//!
//! Steps: version check, catalog fetch, survey (hash the install against the
//! catalog), stage (download and verify every changed file outside the
//! install), install (back up and replace one by one), and restore.
//!
//! API for the launcher: [`plan`], [`apply`] (with a progress callback and a
//! [`Cancel`] token), and [`restore`]. Network access is only through a
//! [`source::Source`]; [`source::LiveServer::retail_ffxi`] is the real server.

pub mod catalog;
pub mod codec;
pub mod error;
#[cfg(feature = "fake-server")]
pub mod fake;
pub mod guard;
pub mod install;
pub mod source;
pub mod survey;
pub mod wire;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use catalog::{Catalog, Signature};
pub use error::{PatchError, Result};
pub use install::RestoreReport;
pub use source::{LiveServer, MemoryServer, Source};
pub use survey::{Lineage, LineageVerdict, Survey};
pub use wire::ServerVersion;

fn data_dir(name: &str) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("TEMP"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Signet").join(name)
}

/// Where downloads are staged: `%LOCALAPPDATA%\Signet\patch-staging`.
pub fn default_staging() -> PathBuf {
    data_dir("patch-staging")
}

/// Where replaced files are backed up: `%LOCALAPPDATA%\Signet\patch-backup`.
pub fn default_backups() -> PathBuf {
    data_dir("patch-backup")
}

/// Cancel token. Cloning shares the flag.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Progress events, in the order they happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Connecting { server: String },
    Version { server: String, local: Option<String> },
    Catalog { bytes: usize, files: usize },
    Scanning { done: usize, total: usize },
    Planned { files: usize, bytes: u64 },
    Downloading { done_bytes: u64, total_bytes: u64 },
    Staged { done: usize, total: usize, path: String },
    Installing { done: usize, total: usize, path: String },
    Restoring { done: usize, total: usize, path: String },
}

/// What `plan` found. Nothing in the install was written.
#[derive(Debug, Clone)]
pub struct PlanReport {
    pub install: PathBuf,
    pub server: ServerVersion,
    /// Newest stamp in the install's own patch.cfg.
    pub local_version: Option<String>,
    pub catalog_bytes: Vec<u8>,
    pub catalog: Catalog,
    pub survey: Survey,
    pub lineage: LineageVerdict,
    /// Where the fetched catalog was saved, if saving was asked for.
    pub saved_catalog: Option<PathBuf>,
    /// Cross-check of the install's patch.cfg with the server catalog:
    /// (shared version lines, lines that disagree). It vouches for pruned
    /// versions only with at least `survey::LOCAL_MIN_OVERLAP` shared lines
    /// and no disagreement.
    pub local_check: Option<(usize, usize)>,
}

impl PlanReport {
    /// True when the install's patch.cfg differs from the server's catalog.
    pub fn catalog_differs(&self) -> bool {
        install::local_catalog(&self.install).as_deref() != Some(self.catalog_bytes.as_slice())
    }
}

#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub install: PathBuf,
    /// Save the fetched catalog as `<dir>\catalog-<version>.cfg`. Must be
    /// outside the install.
    pub save_catalog_in: Option<PathBuf>,
    pub scan_threads: usize,
}

impl PlanOptions {
    pub fn new(install: PathBuf) -> Self {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        PlanOptions { install, save_catalog_in: None, scan_threads: threads }
    }
}

#[derive(Debug, Clone)]
pub struct ApplyOptions {
    pub plan: PlanOptions,
    pub staging: PathBuf,
    pub backups: PathBuf,
    /// Connections held open while downloading.
    pub connections: usize,
}

impl ApplyOptions {
    pub fn new(install: PathBuf) -> Self {
        let mut plan = PlanOptions::new(install);
        plan.save_catalog_in = Some(default_staging());
        ApplyOptions {
            plan,
            staging: default_staging(),
            backups: default_backups(),
            connections: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub version: String,
    pub installed: usize,
    pub downloaded_bytes: u64,
    /// `None` when the install was already current and nothing was written.
    pub backup: Option<PathBuf>,
}

fn norm(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut s = abs.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

/// True when `inner` is `outer` or below it.
pub fn is_within(inner: &Path, outer: &Path) -> bool {
    norm(inner).starts_with(&norm(outer))
}

fn refuse_inside_install(dir: &Path, install: &Path, what: &str) -> Result<()> {
    if is_within(dir, install) || is_within(install, dir) {
        return Err(PatchError::refused(format!(
            "the {what} folder {} overlaps the install {}",
            dir.display(),
            install.display()
        )));
    }
    Ok(())
}

/// Version check, catalog, survey. Never writes to the install. Downloads no
/// game files: only the version reply and the catalog.
pub fn plan(
    opts: &PlanOptions,
    source: &dyn Source,
    progress: &(dyn Fn(Progress) + Sync),
    cancel: &Cancel,
) -> Result<PlanReport> {
    let install = opts.install.clone();
    guard::check_install_shape(&install)?;
    if let Some(dir) = &opts.save_catalog_in {
        refuse_inside_install(dir, &install, "catalog")?;
    }
    let local_catalog = install::local_catalog(&install).and_then(|b| catalog::parse_bytes(&b).ok());
    let local_version = local_catalog.as_ref().and_then(|c| c.newest_stamp().map(str::to_owned));

    progress(Progress::Connecting { server: source.describe() });
    let server = source.open()?.version().map_err(|e| e.context("version check"))?;
    progress(Progress::Version { server: server.version.clone(), local: local_version.clone() });
    if cancel.is_cancelled() {
        return Err(PatchError::Cancelled);
    }
    // The catalog comes on a fresh connection, as the viewer does it.
    let catalog_bytes = source.open()?.catalog().map_err(|e| e.context("catalog"))?;
    let catalog = catalog::parse_bytes(&catalog_bytes)?;
    progress(Progress::Catalog { bytes: catalog_bytes.len(), files: catalog.entries.len() });

    let saved_catalog = match &opts.save_catalog_in {
        Some(dir) => {
            let safe: String =
                server.version.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
            let path = dir.join(format!("catalog-{safe}.cfg"));
            error::io_at(fs::create_dir_all(dir), dir)?;
            error::io_at(fs::write(&path, &catalog_bytes), &path)?;
            Some(path)
        }
        None => None,
    };

    let history = local_catalog.as_ref().map(|l| survey::LocalHistory::new(l, &catalog));
    let survey = survey::survey(&install, &catalog, progress, cancel, opts.scan_threads, history.as_ref())?;
    let lineage = survey::judge_lineage(&survey.lineage, local_version.is_some());
    progress(Progress::Planned { files: survey.work.len(), bytes: survey.bytes });
    let local_check = history.map(|h| (h.overlap, h.mismatched));
    Ok(PlanReport { install, server, local_version, catalog_bytes, catalog, survey, lineage, saved_catalog, local_check })
}

/// Plan, stage and verify every changed file, then install with backups.
/// Cancel works until the install step starts; after that the run finishes
/// so the install is never left half done without a journal.
pub fn apply(
    opts: &ApplyOptions,
    source: &dyn Source,
    progress: &(dyn Fn(Progress) + Sync),
    cancel: &Cancel,
) -> Result<ApplyReport> {
    let install = opts.plan.install.clone();
    refuse_inside_install(&opts.staging, &install, "staging")?;
    refuse_inside_install(&opts.backups, &install, "backup")?;
    guard::refuse_if_running(&install)?;

    let report = plan(&opts.plan, source, progress, cancel)?;
    if let LineageVerdict::Foreign(why) = &report.lineage {
        return Err(PatchError::refused(why.clone()));
    }
    let version = report.server.version.clone();
    if version.is_empty() || version.contains(['/', '\\', ':', '.']) {
        return Err(PatchError::failed(format!("server version {version:?} is not usable as a folder name")));
    }
    if report.survey.work.is_empty() && !report.catalog_differs() {
        return Ok(ApplyReport { version, installed: 0, downloaded_bytes: 0, backup: None });
    }
    let mut seen = HashSet::new();
    if let Some(dup) = report.survey.work.iter().find(|w| !seen.insert(w.path.to_ascii_lowercase())) {
        return Err(PatchError::failed(format!("catalog lists {} twice", dup.path)));
    }

    let stage_dir = opts.staging.join(&version);
    let files_dir = stage_dir.join("files");
    let downloaded = install::stage_all(
        &install,
        &report.catalog,
        &report.survey.work,
        &files_dir,
        source,
        opts.connections,
        progress,
        cancel,
    )?;
    let staged_catalog = stage_dir.join(catalog::LOCAL_CATALOG);
    let catalog_sig = Signature::of_bytes(&report.catalog_bytes);
    error::io_at(fs::write(&staged_catalog, &report.catalog_bytes), &staged_catalog)?;
    if Signature::of_file(&staged_catalog)? != catalog_sig {
        return Err(PatchError::failed("staged patch.cfg does not match the catalog"));
    }
    if cancel.is_cancelled() {
        return Err(PatchError::Cancelled);
    }

    // Final check of everything staged, then the install step.
    let mut items = Vec::with_capacity(report.survey.work.len() + 1);
    for w in &report.survey.work {
        let want = report.catalog.entries[w.entry].current().sig;
        let staged = files_dir.join(&w.path);
        if Signature::of_file(&staged)? != want {
            return Err(PatchError::failed(format!("staged {} changed after it was checked", w.path)));
        }
        items.push(install::Placement { rel: w.path.clone(), staged, want });
    }
    items.push(install::Placement { rel: catalog::LOCAL_CATALOG.into(), staged: staged_catalog, want: catalog_sig });

    guard::refuse_if_running(&install)?;
    let mut backup = opts.backups.join(install::timestamp());
    let mut n = 1;
    while backup.exists() {
        n += 1;
        backup = opts.backups.join(format!("{}-{n}", install::timestamp()));
    }
    let installed = install::install_all(&install, &backup, &version, &items, progress)?;
    Ok(ApplyReport { version, installed, downloaded_bytes: downloaded, backup: Some(backup) })
}

/// Marker written into a backup folder once it has been restored.
pub const RESTORED_MARK: &str = "restored.txt";

/// Undo an apply from its backup folder. A restored backup is marked so
/// [`latest_backup`] skips it.
pub fn restore(
    backup_dir: &Path,
    install_override: Option<&Path>,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<RestoreReport> {
    let report = install::restore(backup_dir, install_override, progress)?;
    let _ = fs::write(backup_dir.join(RESTORED_MARK), format!("restored {}
", install::timestamp()));
    Ok(report)
}

/// The newest backup folder under `backups` that holds a journal and has not
/// been restored. Folder names are UTC timestamps, so name order is time order.
pub fn latest_backup(backups: &Path) -> Option<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    for e in fs::read_dir(backups).ok()?.flatten() {
        let dir = e.path();
        if !dir.join(install::JOURNAL).is_file() || dir.join(RESTORED_MARK).exists() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if best.as_ref().is_none_or(|(b, _)| sortable(&name) > sortable(b)) {
            best = Some((name, dir));
        }
    }
    best.map(|(_, p)| p)
}

/// `YYYYMMDD-HHMMSS` and `YYYYMMDD-HHMMSS-N` in time order: pad the suffix.
fn sortable(name: &str) -> String {
    match name.splitn(3, '-').collect::<Vec<_>>().as_slice() {
        [d, t, n] => format!("{d}-{t}-{:>6}", n),
        [d, t] => format!("{d}-{t}-{:>6}", "0"),
        _ => name.to_string(),
    }
}

/// Version check only: one small request, no file list. Never writes anything.
pub fn check_version(source: &dyn Source) -> Result<ServerVersion> {
    source.open()?.version().map_err(|e| e.context("version check"))
}

/// Newest version stamp in the install's own patch.cfg.
pub fn local_version(install: &Path) -> Option<String> {
    let bytes = install::local_catalog(install)?;
    catalog::parse_bytes(&bytes).ok()?.newest_stamp().map(str::to_owned)
}

/// Order two `NNNNNNNN_N` version stamps (date, then revision). Anything that
/// does not parse sorts as equal to nothing: callers compare for equality first.
pub fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(s: &str) -> Option<(u32, u32)> {
        let (d, n) = s.split_once('_')?;
        Some((d.parse().ok()?, n.parse().ok()?))
    }
    match (key(a), key(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Processes that would block patching this install (PlayOnline, FFXI, anything from its folder).
pub fn blockers(install: &Path) -> Vec<guard::Blocker> {
    guard::running_images().into_iter().filter(|b| guard::is_blocking(&b.image, install)).collect()
}

/// Install folder from the registry, unless overridden.
pub fn find_install(override_dir: Option<&Path>) -> Result<PathBuf> {
    match override_dir {
        Some(p) => Ok(p.to_path_buf()),
        None => guard::registry_install(),
    }
}

/// A fresh, empty folder for one test.
#[cfg(test)]
pub(crate) fn testdir(name: &str) -> PathBuf {
    use std::sync::atomic::AtomicUsize;
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir()
        .join("signet-patch-tests")
        .join(format!("{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod tests;

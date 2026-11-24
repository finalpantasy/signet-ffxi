//! signet-patch: update FINAL FANTASY XI from Square Enix's patch server.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use signet_patch::{
    ApplyOptions, Cancel, LineageVerdict, LiveServer, MemoryServer, PatchError, PlanOptions, PlanReport, Progress,
    Source, catalog, find_install,
};

const HELP: &str = "\
signet-patch: update FINAL FANTASY XI without PlayOnline Viewer

Usage:
  signet-patch plan    [--install DIR] [--manifest FILE] [--staging DIR] [--no-save] [--list N]
  signet-patch apply   [--install DIR] [--staging DIR] [--backup DIR] [--connections N] [--yes]
  signet-patch restore BACKUP_DIR [--install DIR]
  signet-patch manifest FILE

plan     Check the server version, fetch the file list, and compare it with the
         install. Downloads no game files and never writes to the install.
         Saves the file list in the staging folder unless --no-save.
         --manifest uses a saved file list instead of the server.
apply    Same check, then download and verify every changed file in the
         staging folder, back up each original, and replace it.
         Without --yes it stops after the plan.
restore  Put back the files from a backup folder made by apply.
manifest Read a saved file list (or patch.cfg) and print its counts.

The install folder comes from the registry unless --install is given.";

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        match self.rest.iter().position(|a| a == name) {
            Some(i) => {
                self.rest.remove(i);
                true
            }
            None => false,
        }
    }
    fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        match self.rest.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.rest.len() => {
                let v = self.rest.remove(i + 1);
                self.rest.remove(i);
                Ok(Some(v))
            }
            Some(_) => Err(format!("{name} needs a value")),
            None => Ok(None),
        }
    }
    fn finish(&self) -> Result<(), String> {
        match self.rest.first() {
            Some(a) => Err(format!("unknown argument: {a}")),
            None => Ok(()),
        }
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn printer() -> impl Fn(Progress) + Sync {
    let last_pct = AtomicU64::new(u64::MAX);
    move |p| match p {
        Progress::Connecting { server } => eprintln!("Connecting to {server}..."),
        Progress::Version { server, local } => {
            eprintln!("Server version {server}, install {}", local.as_deref().unwrap_or("unknown"))
        }
        Progress::Catalog { bytes, files } => eprintln!("File list: {files} files ({bytes} bytes)"),
        Progress::Scanning { done, total } => eprintln!("Checking files: {done} of {total}"),
        Progress::Planned { .. } => {}
        Progress::Downloading { done_bytes, total_bytes } => {
            let pct = (done_bytes * 100).checked_div(total_bytes).unwrap_or(100);
            if pct / 5 != last_pct.swap(pct / 5, Ordering::Relaxed) {
                eprintln!("Downloading: {pct}% ({} of {})", mb(done_bytes), mb(total_bytes));
            }
        }
        Progress::Staged { done, total, .. } if done == total || done % 500 == 0 => {
            eprintln!("Verified {done} of {total} files")
        }
        Progress::Installing { done, total, .. } if done == total || done % 500 == 0 => {
            eprintln!("Installed {done} of {total} files")
        }
        Progress::Restoring { done, total, .. } if done == total || done % 500 == 0 => {
            eprintln!("Restored {done} of {total} files")
        }
        _ => {}
    }
}

fn print_plan(r: &PlanReport, list: usize) {
    let s = &r.survey;
    println!("Install:        {}", r.install.display());
    println!("Server version: {}", r.server.version);
    println!("Install version: {}", r.local_version.as_deref().unwrap_or("unknown (no patch.cfg)"));
    println!("File list:      {} bytes, {} files", r.catalog_bytes.len(), s.total);
    println!("Current:        {}", s.current);
    println!("Outdated:       {}", s.outdated);
    println!("Missing:        {}", s.missing);
    println!("Unrecognized:   {}", s.unrecognized);
    println!(
        "To fetch:       {} files ({} whole, {} by delta), {} bytes ({})",
        s.work.len(),
        s.images,
        s.delta_chains,
        s.bytes,
        mb(s.bytes)
    );
    let l = &s.lineage;
    let verdict = match &r.lineage {
        LineageVerdict::Retail => "retail".to_string(),
        LineageVerdict::UnpatchedBase => "unpatched base install (allowed)".to_string(),
        LineageVerdict::Foreign(_) => "NOT retail, apply will refuse".to_string(),
    };
    println!(
        "Lineage:        {verdict} (programs {}/{} known, data {}/{} known, gap {:.1} points)",
        l.code_known,
        l.code_present,
        l.data_known,
        l.data_present,
        l.gap()
    );
    match r.local_check {
        Some((shared, bad)) => println!(
            "Install patch.cfg: {shared} lines shared with the server, {bad} disagree; {} files known only from it",
            s.known_from_local
        ),
        None => println!("Install patch.cfg: none"),
    }
    if let Some(p) = &r.saved_catalog {
        println!("Saved file list: {}", p.display());
    }
    for w in s.work.iter().take(list) {
        println!("  {:?} {:?} {} bytes  {}", w.local, w.step, w.bytes, w.path);
    }
}

fn install_arg(args: &mut Args) -> Result<PathBuf, PatchError> {
    let given = args.value("--install").map_err(PatchError::failed)?;
    find_install(given.as_deref().map(std::path::Path::new))
}

fn run() -> Result<(), PatchError> {
    let mut all: Vec<String> = std::env::args().skip(1).collect();
    if all.is_empty() || all.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    let command = all.remove(0);
    let mut args = Args { rest: all };
    let progress = printer();
    let cancel = Cancel::new();
    match command.as_str() {
        "plan" => {
            let install = install_arg(&mut args)?;
            let manifest = args.value("--manifest").map_err(PatchError::failed)?;
            let staging = args.value("--staging").map_err(PatchError::failed)?;
            let no_save = args.flag("--no-save");
            let list: usize = args
                .value("--list")
                .map_err(PatchError::failed)?
                .map(|v| v.parse().map_err(|_| PatchError::failed("--list needs a number")))
                .transpose()?
                .unwrap_or(0);
            args.finish().map_err(PatchError::failed)?;
            let mut opts = PlanOptions::new(install);
            let source: Box<dyn Source> = match &manifest {
                Some(f) => {
                    let bytes = std::fs::read(f).map_err(|e| PatchError::failed(format!("{f}: {e}")))?;
                    let stamp = catalog::parse_bytes(&bytes)?.newest_stamp().unwrap_or("unknown").to_string();
                    Box::new(MemoryServer::from_saved_catalog(stamp, bytes))
                }
                None => Box::new(LiveServer::retail_ffxi()),
            };
            if !no_save && manifest.is_none() {
                opts.save_catalog_in =
                    Some(staging.map(PathBuf::from).unwrap_or_else(signet_patch::default_staging));
            }
            let r = signet_patch::plan(&opts, source.as_ref(), &progress, &cancel)?;
            print_plan(&r, list);
            Ok(())
        }
        "apply" => {
            let install = install_arg(&mut args)?;
            let mut opts = ApplyOptions::new(install);
            if let Some(s) = args.value("--staging").map_err(PatchError::failed)? {
                opts.staging = PathBuf::from(&s);
                opts.plan.save_catalog_in = Some(PathBuf::from(s));
            }
            if let Some(b) = args.value("--backup").map_err(PatchError::failed)? {
                opts.backups = PathBuf::from(b);
            }
            if let Some(c) = args.value("--connections").map_err(PatchError::failed)? {
                opts.connections = c.parse().map_err(|_| PatchError::failed("--connections needs a number"))?;
            }
            let yes = args.flag("--yes");
            let dry = args.flag("--dry-run");
            args.finish().map_err(PatchError::failed)?;
            let live = LiveServer::retail_ffxi();
            if !yes || dry {
                let r = signet_patch::plan(&opts.plan, &live, &progress, &cancel)?;
                print_plan(&r, 0);
                println!("Nothing was changed. Run again with --yes to download and install.");
                return Ok(());
            }
            let r = signet_patch::apply(&opts, &live, &progress, &cancel)?;
            match &r.backup {
                None => println!("Already up to date ({}). Nothing was changed.", r.version),
                Some(b) => {
                    println!("Updated to {}: {} files, {} downloaded.", r.version, r.installed, mb(r.downloaded_bytes));
                    println!("Backup: {}", b.display());
                    println!("To undo: signet-patch restore \"{}\"", b.display());
                }
            }
            Ok(())
        }
        "restore" => {
            if args.rest.is_empty() || args.rest[0].starts_with("--") {
                return Err(PatchError::failed("restore needs a backup folder"));
            }
            let dir = PathBuf::from(args.rest.remove(0));
            let install = args.value("--install").map_err(PatchError::failed)?.map(PathBuf::from);
            args.finish().map_err(PatchError::failed)?;
            let r = signet_patch::restore(&dir, install.as_deref(), &progress)?;
            println!("Restored {} files and removed {} added files in {}.", r.restored, r.removed, r.install.display());
            if r.was_partial {
                println!("That apply had stopped part way; everything it changed is now undone.");
            }
            Ok(())
        }
        "manifest" => {
            if args.rest.len() != 1 {
                return Err(PatchError::failed("manifest needs one file"));
            }
            let f = args.rest.remove(0);
            let bytes = std::fs::read(&f).map_err(|e| PatchError::failed(format!("{f}: {e}")))?;
            let c = catalog::parse_bytes(&bytes)?;
            let revisions: usize = c.entries.iter().map(|e| e.revisions.len()).sum();
            let deltas: usize = c.entries.iter().flat_map(|e| &e.revisions).filter(|r| r.delta.is_some()).count();
            let size: u64 = c.entries.iter().map(|e| e.current().sig.size).sum();
            println!("Files: {}", c.entries.len());
            println!("Version lines: {revisions} ({deltas} with a delta)");
            println!("Newest version: {}", c.newest_stamp().unwrap_or("none"));
            println!("Current size of all files: {size} bytes ({})", mb(size));
            Ok(())
        }
        other => Err(PatchError::failed(format!("unknown command: {other}. Try --help."))),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(PatchError::Refused(m)) => {
            eprintln!("Refused: {m}");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(1)
        }
    }
}

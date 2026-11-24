//! Staging, install with backup, and restore.
//!
//! Order of work in an apply:
//! 1. every changed file is built in the staging folder and checked against
//!    its catalog signature (the install is only read);
//! 2. only when all of them pass, each install file is copied to the backup
//!    folder, the copy is checked, a journal line is written, and the staged
//!    file replaces the original through a checked temp file and a rename;
//! 3. `patch.cfg` goes last, so an interrupted run still looks unpatched to
//!    PlayOnline.
//!
//! `restore` replays the journal backwards.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::catalog::{self, Catalog, Entry, LOCAL_CATALOG, Signature};
use crate::codec;
use crate::error::{PatchError, Result, io_at};
use crate::source::{Session, Source};
use crate::survey::{FileWork, Step};
use crate::{Cancel, Progress};

pub const JOURNAL: &str = "signet-backup.txt";
const JOURNAL_HEAD: &str = "signet-patch backup 1";
const NEW_SUFFIX: &str = ".signet-new";
const PART_SUFFIX: &str = ".part";
const ATTEMPTS: usize = 3;
/// Decoded files larger than this are refused (largest retail DAT is far smaller).
const FILE_LIMIT: usize = 1 << 30;

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn make_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        io_at(fs::create_dir_all(parent), parent)?;
    }
    Ok(())
}

/// Write bytes through a temp file and a rename, then check them on disk.
fn write_checked(dest: &Path, data: &[u8], want: Signature) -> Result<()> {
    make_parent(dest)?;
    let tmp = with_suffix(dest, PART_SUFFIX);
    io_at(fs::write(&tmp, data), &tmp)?;
    let got = Signature::of_file(&tmp)?;
    if got != want {
        fs::remove_file(&tmp).ok();
        return Err(PatchError::failed(format!("{}: written file does not match its signature", dest.display())));
    }
    io_at(fs::rename(&tmp, dest), dest)
}

/// Copy a file through a temp file, check it, then rename into place.
fn copy_checked(from: &Path, dest: &Path, want: Signature, suffix: &str) -> Result<()> {
    make_parent(dest)?;
    let tmp = with_suffix(dest, suffix);
    io_at(fs::copy(from, &tmp), &tmp)?;
    let got = Signature::of_file(&tmp)?;
    if got != want {
        fs::remove_file(&tmp).ok();
        return Err(PatchError::failed(format!(
            "{}: copy of {} does not match its signature",
            dest.display(),
            from.display()
        )));
    }
    if let Err(e) = fs::rename(&tmp, dest) {
        fs::remove_file(&tmp).ok();
        return Err(PatchError::failed(format!("{}: {e}", dest.display())));
    }
    Ok(())
}

/// Build the post-patch bytes of one file and check every step.
fn produce(
    session: &mut dyn Session,
    root: &Path,
    entry: &Entry,
    step: Step,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<Vec<u8>> {
    let current = entry.current();
    let limit = (current.sig.size as usize).min(FILE_LIMIT);
    let data = match step {
        Step::UpToDate => return Err(PatchError::failed("nothing to fetch for an up-to-date file")),
        Step::Image => {
            let raw = session.fetch(&current.image.path, current.image.len, on_bytes)?;
            codec::unpack_download(&raw, limit).map_err(|e| e.context(&current.image.path))?
        }
        Step::Deltas { first } => {
            let local = root.join(&entry.path);
            let mut data = io_at(fs::read(&local), &local)?;
            let mut base = 0u64;
            for r in &entry.revisions[first..] {
                let d = r.delta.as_ref().ok_or_else(|| PatchError::failed("delta chain has a gap"))?;
                let raw = session.fetch(&d.path, d.len, &mut |n| on_bytes(base + n))?;
                base += d.len;
                let step_limit = (r.sig.size as usize).min(FILE_LIMIT);
                data = codec::apply_olc(&raw, &data, step_limit).map_err(|e| e.context(&d.path))?;
                if Signature::of_bytes(&data) != r.sig {
                    return Err(PatchError::failed(format!("{}: delta {} gave the wrong result", entry.path, d.path)));
                }
            }
            data
        }
    };
    if Signature::of_bytes(&data) != current.sig {
        return Err(PatchError::failed(format!("{}: download does not match the catalog signature", entry.path)));
    }
    Ok(data)
}

/// Download and verify every file in `work` into `stage_dir`. The install is
/// only read. Files already staged with the right signature are kept.
#[allow(clippy::too_many_arguments)]
pub fn stage_all(
    root: &Path,
    catalog: &Catalog,
    work: &[FileWork],
    stage_dir: &Path,
    source: &dyn Source,
    connections: usize,
    progress: &(dyn Fn(Progress) + Sync),
    cancel: &Cancel,
) -> Result<u64> {
    let total_bytes: u64 = work.iter().map(|w| w.bytes).sum();
    let next = AtomicUsize::new(0);
    let staged = AtomicUsize::new(0);
    let received = AtomicU64::new(0);
    let failure: Mutex<Option<PatchError>> = Mutex::new(None);
    let failed = || failure.lock().map(|f| f.is_some()).unwrap_or(true);
    let record = |e: PatchError| {
        failure.lock().unwrap().get_or_insert(e);
    };

    std::thread::scope(|scope| {
        for _ in 0..connections.clamp(1, 8).min(work.len().max(1)) {
            scope.spawn(|| {
                let mut session: Option<Box<dyn Session>> = None;
                loop {
                    if failed() {
                        return;
                    }
                    if cancel.is_cancelled() {
                        return record(PatchError::Cancelled);
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(w) = work.get(i) else { return };
                    let entry = &catalog.entries[w.entry];
                    let want = entry.current().sig;
                    let dest = stage_dir.join(&w.path);
                    if dest.is_file() && Signature::of_file(&dest).ok() == Some(want) {
                        received.fetch_add(w.bytes, Ordering::Relaxed);
                    } else {
                        let mut result = Err(PatchError::failed("not attempted"));
                        for _ in 0..ATTEMPTS {
                            if session.is_none() {
                                match source.open() {
                                    Ok(s) => session = Some(s),
                                    Err(e) => {
                                        result = Err(e);
                                        continue;
                                    }
                                }
                            }
                            let s = session.as_mut().expect("opened above");
                            let mut counted = 0u64;
                            result = produce(s.as_mut(), root, entry, w.step, &mut |n| {
                                let add = n.saturating_sub(counted);
                                counted = n;
                                let done = received.fetch_add(add, Ordering::Relaxed) + add;
                                progress(Progress::Downloading { done_bytes: done, total_bytes });
                            });
                            match &result {
                                Ok(_) => break,
                                Err(_) => session = None,
                            }
                        }
                        let data = match result {
                            Ok(d) => d,
                            Err(e) => return record(e.context(&w.path)),
                        };
                        if let Err(e) = write_checked(&dest, &data, want) {
                            return record(e);
                        }
                    }
                    let n = staged.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(Progress::Staged { done: n, total: work.len(), path: w.path.clone() });
                }
            });
        }
    });

    match failure.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(received.load(Ordering::Relaxed)),
    }
}

/// UTC timestamp `YYYYMMDD-HHMMSS` for backup folder names.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

struct Journal {
    file: File,
}

impl Journal {
    fn create(dir: &Path, install: &Path, version: &str) -> Result<Journal> {
        io_at(fs::create_dir_all(dir), dir)?;
        let path = dir.join(JOURNAL);
        let mut file = io_at(OpenOptions::new().write(true).create_new(true).open(&path), &path)?;
        let head = format!("{JOURNAL_HEAD}\ninstall\t{}\nversion\t{version}\n", install.display());
        io_at(file.write_all(head.as_bytes()), &path)?;
        io_at(file.sync_all(), &path)?;
        Ok(Journal { file })
    }
    fn line(&mut self, text: &str) -> Result<()> {
        self.file.write_all(text.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.sync_all()?;
        Ok(())
    }
}

/// One file to put into the install.
pub struct Placement {
    pub rel: String,
    pub staged: PathBuf,
    pub want: Signature,
}

/// Replace files in the install one by one, backing each original up first.
/// Returns how many were placed.
pub fn install_all(
    install: &Path,
    backup_dir: &Path,
    version: &str,
    items: &[Placement],
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<usize> {
    let mut journal = Journal::create(backup_dir, install, version)?;
    let saved = backup_dir.join("files");
    for (i, item) in items.iter().enumerate() {
        let rel = catalog::check_relative(&item.rel)?;
        let dest = install.join(&rel);
        let hint = |e: PatchError| {
            e.context(format!("installing {} (undo with: signet-patch restore \"{}\")", item.rel, backup_dir.display()))
        };
        if dest.is_file() {
            let before = Signature::of_file(&dest).map_err(hint)?;
            copy_checked(&dest, &saved.join(&rel), before, PART_SUFFIX).map_err(hint)?;
            journal
                .line(&format!("R\t{}\t{}\t{}\t{}", before.size, before.byte_sum, before.md5_head, item.rel))
                .map_err(hint)?;
        } else {
            journal.line(&format!("A\t{}", item.rel)).map_err(hint)?;
        }
        copy_checked(&item.staged, &dest, item.want, NEW_SUFFIX).map_err(hint)?;
        progress(Progress::Installing { done: i + 1, total: items.len(), path: item.rel.clone() });
    }
    journal.line("done")?;
    Ok(items.len())
}

/// What a restore did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub install: PathBuf,
    pub restored: usize,
    pub removed: usize,
    /// The journal had no closing line: the apply stopped part way.
    pub was_partial: bool,
}

enum Undo {
    Replaced { rel: String, sig: Signature },
    Added { rel: String },
}

/// Put back every file a backup holds and remove files the apply added.
pub fn restore(
    backup_dir: &Path,
    install_override: Option<&Path>,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<RestoreReport> {
    let path = backup_dir.join(JOURNAL);
    let reader = BufReader::new(io_at(File::open(&path), &path)?);
    let mut lines = reader.lines();
    if lines.next().transpose()?.as_deref() != Some(JOURNAL_HEAD) {
        return Err(PatchError::refused(format!("{} is not a signet-patch backup", backup_dir.display())));
    }
    let mut install: Option<PathBuf> = None;
    let mut undo = Vec::new();
    let mut complete = false;
    for line in lines {
        let line = line?;
        let parts: Vec<&str> = line.split('\t').collect();
        let bad = || PatchError::failed(format!("backup journal line is damaged: {line:?}"));
        match parts.as_slice() {
            ["install", p] => install = Some(PathBuf::from(p)),
            ["version", _] => {}
            ["R", size, sum, md5, rel] => {
                catalog::check_relative(rel)?;
                let sig = Signature {
                    size: size.parse().map_err(|_| bad())?,
                    byte_sum: sum.parse().map_err(|_| bad())?,
                    md5_head: md5.parse().map_err(|_| bad())?,
                };
                undo.push(Undo::Replaced { rel: rel.to_string(), sig });
            }
            ["A", rel] => {
                catalog::check_relative(rel)?;
                undo.push(Undo::Added { rel: rel.to_string() });
            }
            ["done"] => complete = true,
            _ => return Err(bad()),
        }
    }
    let install = match install_override {
        Some(p) => p.to_path_buf(),
        None => install.ok_or_else(|| PatchError::failed("backup journal does not name the install folder"))?,
    };
    crate::guard::refuse_if_running(&install)?;
    let mut report = RestoreReport { install: install.clone(), was_partial: !complete, ..Default::default() };
    let total = undo.len();
    for (i, u) in undo.iter().rev().enumerate() {
        match u {
            Undo::Replaced { rel, sig } => {
                let dest = install.join(rel);
                copy_checked(&backup_dir.join("files").join(rel), &dest, *sig, NEW_SUFFIX)?;
                report.restored += 1;
                progress(Progress::Restoring { done: i + 1, total, path: rel.clone() });
            }
            Undo::Added { rel } => {
                let dest = install.join(rel);
                if dest.is_file() {
                    io_at(fs::remove_file(&dest), &dest)?;
                    report.removed += 1;
                }
                fs::remove_file(with_suffix(&dest, NEW_SUFFIX)).ok();
                progress(Progress::Restoring { done: i + 1, total, path: rel.clone() });
            }
        }
    }
    Ok(report)
}

/// Read the install's own `patch.cfg` bytes, if any.
pub fn local_catalog(install: &Path) -> Option<Vec<u8>> {
    fs::read(install.join(LOCAL_CATALOG)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_shape() {
        let t = timestamp();
        assert_eq!(t.len(), 15);
        assert_eq!(&t[8..9], "-");
        assert!(t.starts_with("20"));
    }
}

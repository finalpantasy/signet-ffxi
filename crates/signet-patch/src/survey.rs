//! Compare an install with the catalog and decide, per file, what to fetch.
//!
//! Rule per catalog entry (the viewer's choice):
//! - local file missing, or matching no version line: fetch the current image;
//! - matching the last line: nothing to do;
//! - matching an older line: if every later line has a delta and the deltas
//!   add up to fewer bytes than the current image, apply them in order;
//!   otherwise fetch the current image.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::catalog::{Catalog, Entry, Signature};
use crate::error::{PatchError, Result};
use crate::{Cancel, Progress};

/// What to do for one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    UpToDate,
    /// Download the current whole-file image.
    Image,
    /// Apply the deltas of `revisions[first..]` in order to the local file.
    Deltas { first: usize },
}

/// How the local copy stood before patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalState {
    Missing,
    /// Present but matches no version the catalog knows.
    Unrecognized,
    /// Matches revision `index`.
    Known { index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWork {
    /// Index into `Catalog::entries`.
    pub entry: usize,
    pub path: String,
    pub step: Step,
    pub local: LocalState,
    /// Signature of the local file before patching, if present.
    pub local_sig: Option<Signature>,
    /// Bytes to download for this file.
    pub bytes: u64,
}

/// Counts of how much of the install the catalog recognises, split into code
/// (.exe/.dll) and data. A retail install is consistent: its code and data
/// come from the same point in the version history. A private-server client
/// usually is not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lineage {
    pub code_present: usize,
    pub code_known: usize,
    pub data_present: usize,
    pub data_known: usize,
}

/// Largest gap, in percentage points, between the code and data recognition
/// rates that still counts as one retail lineage.
pub const LINEAGE_GAP_LIMIT: f64 = 8.0;

fn pct(known: usize, present: usize) -> f64 {
    if present == 0 { 0.0 } else { 100.0 * known as f64 / present as f64 }
}

impl Lineage {
    pub fn code_rate(&self) -> f64 {
        pct(self.code_known, self.code_present)
    }
    pub fn data_rate(&self) -> f64 {
        pct(self.data_known, self.data_present)
    }
    pub fn gap(&self) -> f64 {
        (self.code_rate() - self.data_rate()).abs()
    }
    fn count(&mut self, code: bool, known: bool) {
        if code {
            self.code_present += 1;
            self.code_known += known as usize;
        } else {
            self.data_present += 1;
            self.data_known += known as usize;
        }
    }
    fn merge(&mut self, o: &Lineage) {
        self.code_present += o.code_present;
        self.code_known += o.code_known;
        self.data_present += o.data_present;
        self.data_known += o.data_known;
    }
}

/// Result of the lineage check.
#[derive(Debug, Clone, PartialEq)]
pub enum LineageVerdict {
    Retail,
    /// No local catalog: an unpatched base install, which matches little of
    /// the history by nature. Allowed.
    UnpatchedBase,
    Foreign(String),
}

impl LineageVerdict {
    pub fn allowed(&self) -> bool {
        !matches!(self, LineageVerdict::Foreign(_))
    }
}

pub fn judge_lineage(lineage: &Lineage, has_local_catalog: bool) -> LineageVerdict {
    if !has_local_catalog {
        return LineageVerdict::UnpatchedBase;
    }
    if lineage.gap() <= LINEAGE_GAP_LIMIT {
        return LineageVerdict::Retail;
    }
    LineageVerdict::Foreign(format!(
        "this does not look like a Square Enix install: {}/{} programs and {}/{} data files match a known version \
         ({:.1}% against {:.1}%). Private-server clients are refused because patching would overwrite the files \
         they depend on.",
        lineage.code_known,
        lineage.code_present,
        lineage.data_known,
        lineage.data_present,
        lineage.code_rate(),
        lineage.data_rate()
    ))
}

/// The install's own `patch.cfg`, used as a second witness for files whose
/// version the server no longer lists. The server prunes old version lines
/// (a 2019 retail install's executables match none of the server's lines),
/// so without this an old but genuine install reads as foreign.
#[derive(Debug, Clone, Default)]
pub struct LocalHistory {
    newest: HashMap<String, (String, Signature)>,
    /// Version lines present in both catalogs.
    pub overlap: usize,
    /// Of those, lines whose signatures differ.
    pub mismatched: usize,
}

/// Fewest shared lines before the local catalog is trusted.
pub const LOCAL_MIN_OVERLAP: usize = 100;

impl LocalHistory {
    /// Cross-check the local catalog against the server's. Every version
    /// line both list must carry the same signature.
    pub fn new(local: &Catalog, server: &Catalog) -> LocalHistory {
        let by_path: HashMap<&str, &Entry> = server.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        let mut h = LocalHistory::default();
        for e in &local.entries {
            if let Some(srv) = by_path.get(e.path.as_str()) {
                for r in &e.revisions {
                    if let Some(sr) = srv.revisions.iter().find(|x| x.stamp == r.stamp) {
                        h.overlap += 1;
                        h.mismatched += usize::from(sr.sig != r.sig);
                    }
                }
            }
            let cur = e.current();
            h.newest.insert(e.path.clone(), (cur.stamp.clone(), cur.sig));
        }
        h
    }

    /// The local catalog agrees with the server everywhere they overlap.
    pub fn trusted(&self) -> bool {
        self.overlap >= LOCAL_MIN_OVERLAP && self.mismatched == 0
    }

    /// True when the file matches the local catalog's newest line for it and
    /// that version is one the server has pruned (absent from its lines).
    pub fn vouches(&self, entry: &Entry, sig: Signature) -> bool {
        match self.newest.get(&entry.path) {
            Some((stamp, s)) => *s == sig && !entry.revisions.iter().any(|r| &r.stamp == stamp),
            None => false,
        }
    }
}

fn is_code(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".exe") || p.ends_with(".dll")
}

/// The decision rule for one entry.
pub fn decide(entry: &Entry, local: Option<Signature>) -> (Step, LocalState, u64) {
    let image = entry.current().image.len;
    let Some(sig) = local else {
        return (Step::Image, LocalState::Missing, image);
    };
    let Some(index) = entry.revisions.iter().rposition(|r| r.sig == sig) else {
        return (Step::Image, LocalState::Unrecognized, image);
    };
    let state = LocalState::Known { index };
    let last = entry.revisions.len() - 1;
    if index == last {
        return (Step::UpToDate, state, 0);
    }
    let mut delta_bytes = 0u64;
    for r in &entry.revisions[index + 1..] {
        match &r.delta {
            Some(d) => delta_bytes += d.len,
            None => return (Step::Image, state, image),
        }
    }
    if delta_bytes < image { (Step::Deltas { first: index + 1 }, state, delta_bytes) } else { (Step::Image, state, image) }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Survey {
    pub total: usize,
    pub current: usize,
    pub outdated: usize,
    pub missing: usize,
    pub unrecognized: usize,
    pub bytes: u64,
    pub images: usize,
    pub delta_chains: usize,
    pub lineage: Lineage,
    /// Files the server does not recognise but the trusted local catalog does.
    pub known_from_local: usize,
    /// Files that need work, in catalog order.
    pub work: Vec<FileWork>,
}

const BATCH: usize = 32;

/// Hash the install (read-only) against the catalog.
pub fn survey(
    root: &Path,
    catalog: &Catalog,
    progress: &(dyn Fn(Progress) + Sync),
    cancel: &Cancel,
    threads: usize,
    local: Option<&LocalHistory>,
) -> Result<Survey> {
    let local = local.filter(|h| h.trusted());
    let from_local = AtomicUsize::new(0);
    let total = catalog.entries.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<FileWork>> = Mutex::new(Vec::with_capacity(total));
    let lineage = Mutex::new(Lineage::default());
    let failure: Mutex<Option<PatchError>> = Mutex::new(None);
    let stop = || failure.lock().map(|f| f.is_some()).unwrap_or(true) || cancel.is_cancelled();
    progress(Progress::Scanning { done: 0, total });

    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, 32).min(total.max(1)) {
            scope.spawn(|| {
                let mut mine = Vec::new();
                let mut seen = Lineage::default();
                'outer: loop {
                    let start = next.fetch_add(BATCH, Ordering::Relaxed);
                    if start >= total || stop() {
                        break;
                    }
                    for i in start..(start + BATCH).min(total) {
                        let entry = &catalog.entries[i];
                        let path = root.join(&entry.path);
                        let local_sig = if path.is_file() {
                            match Signature::of_file(&path) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    failure.lock().unwrap().get_or_insert(e);
                                    break 'outer;
                                }
                            }
                        } else {
                            None
                        };
                        if let Some(s) = local_sig {
                            let by_server = entry.revisions.iter().any(|r| r.sig == s);
                            let by_local = !by_server && local.is_some_and(|h| h.vouches(entry, s));
                            from_local.fetch_add(usize::from(by_local), Ordering::Relaxed);
                            seen.count(is_code(&entry.path), by_server || by_local);
                        }
                        let (step, state, bytes) = decide(entry, local_sig);
                        mine.push(FileWork { entry: i, path: entry.path.clone(), step, local: state, local_sig, bytes });
                    }
                    let n = done.fetch_add(BATCH, Ordering::Relaxed) + BATCH;
                    if n % 2048 < BATCH {
                        progress(Progress::Scanning { done: n.min(total), total });
                    }
                }
                results.lock().unwrap().append(&mut mine);
                lineage.lock().unwrap().merge(&seen);
            });
        }
    });

    if let Some(e) = failure.into_inner().unwrap() {
        return Err(e);
    }
    if cancel.is_cancelled() {
        return Err(PatchError::Cancelled);
    }
    progress(Progress::Scanning { done: total, total });
    let mut all = results.into_inner().unwrap();
    all.sort_unstable_by_key(|w| w.entry);
    let mut s = Survey {
        total,
        lineage: lineage.into_inner().unwrap(),
        known_from_local: from_local.into_inner(),
        ..Default::default()
    };
    for w in all {
        match (w.step, w.local) {
            (Step::UpToDate, _) => {
                s.current += 1;
                continue;
            }
            (_, LocalState::Missing) => s.missing += 1,
            (_, LocalState::Unrecognized) => s.unrecognized += 1,
            (_, LocalState::Known { .. }) => s.outdated += 1,
        }
        match w.step {
            Step::Image => s.images += 1,
            Step::Deltas { .. } => s.delta_chains += 1,
            Step::UpToDate => {}
        }
        s.bytes += w.bytes;
        s.work.push(w);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Blob, Revision};

    fn sig(n: u64) -> Signature {
        Signature { size: n, byte_sum: n as i32, md5_head: n as i32 }
    }
    fn rev(n: u64, image: u64, delta: Option<u64>) -> Revision {
        Revision {
            stamp: format!("3000000{n}_0"),
            sig: sig(n),
            image: Blob { path: format!("{n}/Direct/f.slc"), len: image },
            delta: delta.map(|len| Blob { path: format!("{n}/Indirect/f.olc"), len }),
        }
    }
    fn entry(revs: Vec<Revision>) -> Entry {
        Entry { path: "f".into(), revisions: revs }
    }

    #[test]
    fn decision_rule() {
        let e = entry(vec![rev(1, 100, None), rev(2, 100, Some(10)), rev(3, 100, Some(20))]);
        assert_eq!(decide(&e, None), (Step::Image, LocalState::Missing, 100));
        assert_eq!(decide(&e, Some(sig(9))), (Step::Image, LocalState::Unrecognized, 100));
        assert_eq!(decide(&e, Some(sig(3))), (Step::UpToDate, LocalState::Known { index: 2 }, 0));
        assert_eq!(decide(&e, Some(sig(2))), (Step::Deltas { first: 2 }, LocalState::Known { index: 1 }, 20));
        assert_eq!(decide(&e, Some(sig(1))), (Step::Deltas { first: 1 }, LocalState::Known { index: 0 }, 30));
        // A gap in the delta chain forces the image.
        let gap = entry(vec![rev(1, 100, None), rev(2, 100, None), rev(3, 100, Some(5))]);
        assert_eq!(decide(&gap, Some(sig(1))).0, Step::Image);
        // Deltas not cheaper than the image: take the image.
        let dear = entry(vec![rev(1, 100, None), rev(2, 50, Some(60))]);
        assert_eq!(decide(&dear, Some(sig(1))), (Step::Image, LocalState::Known { index: 0 }, 50));
        let even = entry(vec![rev(1, 100, None), rev(2, 50, Some(50))]);
        assert_eq!(decide(&even, Some(sig(1))).0, Step::Image);
    }

    #[test]
    fn lineage_rule() {
        let retail = Lineage { code_present: 19, code_known: 19, data_present: 40_000, data_known: 39_990 };
        assert_eq!(judge_lineage(&retail, true), LineageVerdict::Retail);
        let swapped_exes = Lineage { code_present: 19, code_known: 16, data_present: 40_000, data_known: 39_000 };
        assert!(!judge_lineage(&swapped_exes, true).allowed());
        let modded_data = Lineage { code_present: 19, code_known: 19, data_present: 40_000, data_known: 17_000 };
        assert!(!judge_lineage(&modded_data, true).allowed());
        assert_eq!(judge_lineage(&modded_data, false), LineageVerdict::UnpatchedBase);
    }

    #[test]
    fn local_catalog_vouches_only_for_pruned_versions_and_only_when_it_agrees() {
        // The server lists versions 1 and 3 of `f`; the install's catalog says it holds 2.
        let fillers: Vec<Entry> = (0..LOCAL_MIN_OVERLAP)
            .map(|i| Entry { path: format!("d{i}"), revisions: vec![rev(1, 10, None)] })
            .collect();
        let mut server = Catalog { entries: vec![entry(vec![rev(1, 10, None), rev(3, 10, None)])] };
        server.entries.extend(fillers.clone());
        let mut local = Catalog { entries: fillers };
        local.entries.push(entry(vec![rev(1, 10, None), rev(2, 10, None)]));
        let h = LocalHistory::new(&local, &server);
        assert_eq!((h.overlap, h.mismatched), (LOCAL_MIN_OVERLAP + 1, 0));
        assert!(h.trusted());
        let f = &server.entries[0];
        assert!(h.vouches(f, sig(2)));
        assert!(!h.vouches(f, sig(9)), "content must match the local line");
        // A version the server still lists is never vouched for by the local catalog.
        let mut listed = local.clone();
        listed.entries.last_mut().unwrap().revisions.push(rev(3, 10, None));
        assert!(!LocalHistory::new(&listed, &server).vouches(f, sig(3)));
        // One disagreeing shared line and the local catalog is not trusted.
        let mut forged = local.clone();
        forged.entries[0].revisions[0].sig.md5_head ^= 1;
        assert!(!LocalHistory::new(&forged, &server).trusted());
        // Too little overlap: not trusted.
        let small = Catalog { entries: vec![entry(vec![rev(1, 10, None), rev(2, 10, None)])] };
        assert!(!LocalHistory::new(&small, &server).trusted());
    }
}

//! End-to-end tests on a made-up install. No network beyond 127.0.0.1.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use super::*;
use crate::codec::{flag, testkit};
use crate::source::loopback;

/// One version of a file in the fake server.
struct Ver {
    stamp: &'static str,
    data: Vec<u8>,
    /// Image coding flag for the whole-file download.
    coding: u8,
    /// Ship a delta from the previous version.
    delta: bool,
}

struct FakeFile {
    path: &'static str,
    versions: Vec<Ver>,
}

fn image_bytes(v: &Ver) -> Vec<u8> {
    let mut out = vec![v.coding];
    match v.coding {
        flag::STORED => out.extend_from_slice(&v.data),
        flag::SLC => out.extend(testkit::slc(&v.data)),
        flag::ZLIB => out.extend(miniz_oxide::deflate::compress_to_vec_zlib(&v.data, 6)),
        _ => unreachable!(),
    }
    out
}

/// Catalog text and server files for a set of fake files.
fn build_server(files: &[FakeFile]) -> (String, HashMap<String, Vec<u8>>) {
    let mut text = String::new();
    let mut blobs = HashMap::new();
    for f in files {
        text.push_str(&format!("file {} {{\n", f.path));
        for (i, v) in f.versions.iter().enumerate() {
            let s = Signature::of_bytes(&v.data);
            let image_path = format!("{}/Direct/{}.slc", v.stamp, f.path);
            let image = image_bytes(v);
            text.push_str(&format!("{} {} {} {} {} {}", v.stamp, s.size, s.byte_sum, s.md5_head, image_path, image.len()));
            blobs.insert(image_path, image);
            if v.delta && i > 0 {
                let delta_path = format!("{}/Indirect/{}.olc", v.stamp, f.path);
                let delta = testkit::olc(&f.versions[i - 1].data, &v.data);
                text.push_str(&format!(" {} {}", delta_path, delta.len()));
                blobs.insert(delta_path, delta);
            }
            text.push('\n');
        }
        text.push_str("}\n\n");
    }
    text.push_str("end\n");
    (text, blobs)
}

fn bytes(seed: u32, n: usize) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_mul(seed).wrapping_add(seed) % 251) as u8).collect()
}

fn edited(base: &[u8], at: usize, text: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    v[at..at + text.len()].copy_from_slice(text);
    v
}

struct Fixture {
    root: std::path::PathBuf,
    install: std::path::PathBuf,
    server: MemoryServer,
    targets: BTreeMap<String, Vec<u8>>,
}

const OLD: &str = "30100101_0";
const MID: &str = "30150101_0";
const NEW: &str = "30200101_0";

/// The fake install:
/// - ROM/0/0.DAT at the oldest version, cheap deltas to current: delta chain
/// - ROM/0/1.DAT and VTABLE.DAT and FFXiMain.dll current
/// - ROM/1/5.DAT missing: .slc image
/// - FTABLE.DAT holds unknown bytes: zlib image
/// - Tools/Help File.txt old, deltas dearer than the image: stored image
/// - 20 current filler DATs so the data recognition rate looks retail
fn fixture(name: &str) -> Fixture {
    let root = testdir(name);
    let install = root.join("SquareEnix").join("FINAL FANTASY XI");
    let a1 = bytes(7, 30_000);
    let a2 = edited(&a1, 100, b"second version");
    let a3 = edited(&a2, 20_000, b"third version!");
    let help1 = bytes(3, 400);
    let help2 = bytes(5, 300);
    let mut files = vec![
        FakeFile {
            path: "ROM/0/0.DAT",
            versions: vec![
                Ver { stamp: OLD, data: a1.clone(), coding: flag::SLC, delta: false },
                Ver { stamp: MID, data: a2, coding: flag::SLC, delta: true },
                Ver { stamp: NEW, data: a3, coding: flag::SLC, delta: true },
            ],
        },
        FakeFile { path: "ROM/0/1.DAT", versions: vec![Ver { stamp: OLD, data: bytes(11, 5000), coding: flag::SLC, delta: false }] },
        FakeFile { path: "VTABLE.DAT", versions: vec![Ver { stamp: NEW, data: bytes(13, 900), coding: flag::STORED, delta: false }] },
        FakeFile { path: "FFXiMain.dll", versions: vec![Ver { stamp: NEW, data: bytes(17, 2000), coding: flag::ZLIB, delta: false }] },
        FakeFile { path: "ROM/1/5.DAT", versions: vec![Ver { stamp: NEW, data: bytes(19, 7000), coding: flag::SLC, delta: false }] },
        FakeFile { path: "FTABLE.DAT", versions: vec![Ver { stamp: NEW, data: bytes(23, 4000), coding: flag::ZLIB, delta: false }] },
        FakeFile {
            path: "Tools/Help File.txt",
            versions: vec![
                Ver { stamp: OLD, data: help1.clone(), coding: flag::STORED, delta: false },
                Ver { stamp: NEW, data: help2, coding: flag::STORED, delta: true },
            ],
        },
    ];
    let fillers: Vec<&'static str> =
        (0..20).map(|i| &*Box::leak(format!("ROM/9/{i}.DAT").into_boxed_str())).collect();
    for (i, p) in fillers.iter().enumerate() {
        files.push(FakeFile {
            path: p,
            versions: vec![Ver { stamp: OLD, data: bytes(29 + i as u32, 64), coding: flag::STORED, delta: false }],
        });
    }
    let (text, blobs) = build_server(&files);

    let put = |rel: &str, data: &[u8]| {
        let p = install.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    };
    put("ROM/0/0.DAT", &a1);
    put("Tools/Help File.txt", &help1);
    put("FTABLE.DAT", b"not a version the server knows");
    put("patch.cfg", format!("file ROM/0/0.DAT {{\n{OLD} 1 1 1 {OLD}/Direct/ROM/0/0.DAT.slc 1\n}}\nend\n").as_bytes());
    for f in &files {
        if ["ROM/0/1.DAT", "VTABLE.DAT", "FFXiMain.dll"].contains(&f.path) || f.path.starts_with("ROM/9/") {
            put(f.path, &f.versions.last().unwrap().data);
        }
    }
    fs::write(install.join("user.txt"), b"not in the catalog").unwrap();

    let mut targets: BTreeMap<String, Vec<u8>> =
        files.iter().map(|f| (f.path.to_string(), f.versions.last().unwrap().data.clone())).collect();
    targets.insert("patch.cfg".into(), text.clone().into_bytes());
    let server = MemoryServer { version: NEW.into(), catalog: text.into_bytes(), files: blobs, forbid_fetch: false };
    Fixture { root, install, server, targets }
}

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn quiet(_: Progress) {}

fn apply_opts(fx: &Fixture) -> ApplyOptions {
    let mut o = ApplyOptions::new(fx.install.clone());
    o.staging = fx.root.join("staging");
    o.backups = fx.root.join("backup");
    o.plan.save_catalog_in = Some(fx.root.join("staging"));
    o
}

#[test]
fn plan_counts_files_and_never_writes() {
    let fx = fixture("plan");
    let before = snapshot(&fx.install);
    let offline = MemoryServer { forbid_fetch: true, ..fx.server.clone() };
    let mut opts = PlanOptions::new(fx.install.clone());
    opts.save_catalog_in = Some(fx.root.join("saved"));
    let r = plan(&opts, &offline, &quiet, &Cancel::new()).unwrap();
    assert_eq!(snapshot(&fx.install), before);
    let s = &r.survey;
    assert_eq!(s.total, 27);
    assert_eq!(s.current, 23);
    assert_eq!((s.outdated, s.missing, s.unrecognized), (2, 1, 1));
    assert_eq!((s.images, s.delta_chains), (3, 1));
    let chain = s.work.iter().find(|w| w.path == "ROM/0/0.DAT").unwrap();
    assert_eq!(chain.step, survey::Step::Deltas { first: 1 });
    let help = s.work.iter().find(|w| w.path == "Tools/Help File.txt").unwrap();
    assert_eq!(help.step, survey::Step::Image, "dearer deltas must lose to the image");
    let entry_bytes = |p: &str| {
        r.catalog.entries.iter().find(|e| e.path == p).unwrap()
    };
    let a = entry_bytes("ROM/0/0.DAT");
    let want = a.revisions[1].delta.as_ref().unwrap().len
        + a.revisions[2].delta.as_ref().unwrap().len
        + entry_bytes("ROM/1/5.DAT").current().image.len
        + entry_bytes("FTABLE.DAT").current().image.len
        + entry_bytes("Tools/Help File.txt").current().image.len;
    assert_eq!(s.bytes, want);
    assert_eq!(r.lineage, LineageVerdict::Retail);
    assert_eq!(r.local_version.as_deref(), Some(OLD));
    assert_eq!(fs::read(r.saved_catalog.unwrap()).unwrap(), fx.server.catalog);
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn apply_installs_verified_files_and_restore_undoes_it() {
    let fx = fixture("apply");
    let before = snapshot(&fx.install);
    let events = std::sync::Mutex::new(Vec::new());
    let r = apply(&apply_opts(&fx), &fx.server, &|p| events.lock().unwrap().push(p), &Cancel::new()).unwrap();
    assert_eq!(r.version, NEW);
    assert_eq!(r.installed, 5, "four files plus patch.cfg");
    let after = snapshot(&fx.install);
    for (path, data) in &fx.targets {
        assert_eq!(after.get(path), Some(data), "{path}");
    }
    assert_eq!(after.get("user.txt").map(Vec::as_slice), Some(&b"not in the catalog"[..]));
    assert!(!after.keys().any(|k| k.ends_with(".signet-new") || k.ends_with(".part")));
    let ev = events.into_inner().unwrap();
    assert!(ev.iter().any(|e| matches!(e, Progress::Installing { done: 5, total: 5, .. })));

    // Second run: nothing to do, nothing written.
    let again = apply(&apply_opts(&fx), &fx.server, &quiet, &Cancel::new()).unwrap();
    assert_eq!((again.installed, again.backup.clone()), (0, None));

    let backup = r.backup.unwrap();
    let undo = restore(&backup, None, &quiet).unwrap();
    assert_eq!((undo.restored, undo.removed, undo.was_partial), (4, 1, false));
    assert_eq!(snapshot(&fx.install), before);
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn bad_download_is_caught_before_the_install_is_touched() {
    let mut fx = fixture("corrupt");
    let before = snapshot(&fx.install);
    // Valid coding, wrong content: the signature check must catch it.
    let key = format!("{NEW}/Direct/ROM/1/5.DAT.slc");
    let mut wrong = vec![flag::SLC];
    let mut data = fx.targets["ROM/1/5.DAT"].clone();
    data[10] ^= 1;
    wrong.extend(testkit::slc(&data));
    // Keep the length the catalog declares.
    wrong.resize(fx.server.files[&key].len(), 0);
    fx.server.files.insert(key, wrong);
    let err = apply(&apply_opts(&fx), &fx.server, &quiet, &Cancel::new()).unwrap_err();
    assert!(matches!(err, PatchError::Failed(_)), "{err}");
    assert_eq!(snapshot(&fx.install), before);
    assert!(!fx.root.join("backup").exists());
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn bad_delta_is_caught() {
    let mut fx = fixture("baddelta");
    let before = snapshot(&fx.install);
    let key = format!("{MID}/Indirect/ROM/0/0.DAT.olc");
    let mut delta = fx.server.files[&key].clone();
    let n = delta.len();
    delta[n / 2] ^= 0x10;
    fx.server.files.insert(key, delta);
    let err = apply(&apply_opts(&fx), &fx.server, &quiet, &Cancel::new()).unwrap_err().to_string();
    assert!(err.contains("ROM/0/0.DAT"), "{err}");
    assert_eq!(snapshot(&fx.install), before);
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn cancel_stops_before_the_install() {
    let fx = fixture("cancel");
    let before = snapshot(&fx.install);
    let cancel = Cancel::new();
    let c2 = cancel.clone();
    let err = apply(
        &apply_opts(&fx),
        &fx.server,
        &move |p| {
            if matches!(p, Progress::Planned { .. }) {
                c2.cancel()
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert_eq!(err, PatchError::Cancelled);
    assert_eq!(snapshot(&fx.install), before);
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn foreign_install_is_refused() {
    let fx = fixture("foreign");
    fs::write(fx.install.join("FFXiMain.dll"), b"a private server's exe").unwrap();
    let before = snapshot(&fx.install);
    let err = apply(&apply_opts(&fx), &fx.server, &quiet, &Cancel::new()).unwrap_err();
    assert!(matches!(&err, PatchError::Refused(m) if m.contains("Square Enix")), "{err}");
    assert_eq!(snapshot(&fx.install), before);
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn staging_inside_the_install_is_refused() {
    let fx = fixture("inside");
    let mut o = apply_opts(&fx);
    o.staging = fx.install.join("stage");
    assert!(matches!(apply(&o, &fx.server, &quiet, &Cancel::new()), Err(PatchError::Refused(_))));
    let mut p = PlanOptions::new(fx.install.clone());
    p.save_catalog_in = Some(fx.install.clone());
    assert!(matches!(plan(&p, &fx.server, &quiet, &Cancel::new()), Err(PatchError::Refused(_))));
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn apply_over_loopback_tcp() {
    let fx = fixture("tcp");
    let live = loopback::serve(NEW, &fx.server.catalog, fx.server.files.clone());
    let r = apply(&apply_opts(&fx), &live, &quiet, &Cancel::new()).unwrap();
    assert_eq!(r.installed, 5);
    let after = snapshot(&fx.install);
    for (path, data) in &fx.targets {
        assert_eq!(after.get(path), Some(data), "{path}");
    }
    fs::remove_dir_all(&fx.root).ok();
}

#[test]
fn restore_refuses_a_folder_without_a_journal() {
    let d = testdir("nojournal");
    assert!(restore(&d, None, &quiet).is_err());
    fs::write(d.join(install::JOURNAL), "something else\n").unwrap();
    assert!(matches!(restore(&d, None, &quiet), Err(PatchError::Refused(_))));
    fs::remove_dir_all(&d).ok();
}

#[test]
fn version_helpers() {
    use std::cmp::Ordering::*;
    assert_eq!(version_order("30261102_1", "30261102_1"), Equal);
    assert_eq!(version_order("30261102_0", "30261102_1"), Less);
    assert_eq!(version_order("30261102_10", "30261102_9"), Greater);
    assert_eq!(version_order("30190430_0", "30261102_1"), Less);
}

#[test]
fn latest_backup_skips_restored_and_orders_by_time() {
    let dir = testdir("latest-backup");
    let mk = |name: &str, journal: bool| {
        let d = dir.join(name);
        fs::create_dir_all(&d).unwrap();
        if journal {
            fs::write(d.join(install::JOURNAL), "x").unwrap();
        }
    };
    assert_eq!(latest_backup(&dir), None);
    mk("20261124-101010", true);
    mk("20261124-101010-2", true);
    mk("20261124-101010-10", true);
    mk("20261124-999999", false);
    assert_eq!(latest_backup(&dir).unwrap().file_name().unwrap(), "20261124-101010-10");
    fs::write(dir.join("20261124-101010-10").join(RESTORED_MARK), "x").unwrap();
    assert_eq!(latest_backup(&dir).unwrap().file_name().unwrap(), "20261124-101010-2");
    assert_eq!(latest_backup(&dir.join("missing")), None);
}

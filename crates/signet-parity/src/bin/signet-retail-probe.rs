//! signet-retail-probe: read-only measurement of the retail FFXI client at 60 Hz.
//!
//! Live:    signet-retail-probe --out retail.csv [--seconds 150] [--pid N] [--wait 60] [--hz 60]
//! Static:  signet-retail-probe --static <unpacked FFXiMain image> [--expect 2019|none]
//!
//! The live mode opens the client with PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ only.
//! It writes nothing to the game, injects nothing, installs no hooks, attaches no debugger, sends
//! no packets and synthesizes no input. The static mode runs the same signature resolver on an
//! unpacked image file, so the check can be redone offline whenever the DLL changes.

use signet_parity::layout::{self, CheckLine, Layout};
use signet_parity::pe;
use std::path::PathBuf;
use std::process::ExitCode;

struct Args {
    static_image: Option<PathBuf>,
    out: Option<PathBuf>,
    seconds: f64,
    hz: f64,
    pid: Option<u32>,
    wait: f64,
    expect_2019: bool,
    check_only: bool,
}

fn usage() -> &'static str {
    "usage:\n  signet-retail-probe --static <unpacked image> [--expect 2019|none]\n  signet-retail-probe --out <csv> [--seconds 150] [--hz 60] [--pid N] [--wait 60] [--check-only]"
}

fn parse() -> Result<Args, String> {
    let mut a = Args { static_image: None, out: None, seconds: 150.0, hz: 60.0, pid: None, wait: 60.0, expect_2019: true, check_only: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = |n: &str| it.next().ok_or(format!("{n} needs a value"));
        match k.as_str() {
            "--static" => a.static_image = Some(val("--static")?.into()),
            "--out" => a.out = Some(val("--out")?.into()),
            "--seconds" => a.seconds = val("--seconds")?.parse().map_err(|_| "bad --seconds")?,
            "--hz" => a.hz = val("--hz")?.parse().map_err(|_| "bad --hz")?,
            "--pid" => a.pid = Some(val("--pid")?.parse().map_err(|_| "bad --pid")?),
            "--wait" => a.wait = val("--wait")?.parse().map_err(|_| "bad --wait")?,
            "--expect" => {
                a.expect_2019 = match val("--expect")?.as_str() {
                    "2019" => true,
                    "none" => false,
                    o => return Err(format!("unknown --expect {o} (2019 or none)")),
                }
            }
            "--check-only" => a.check_only = true,
            "-h" | "--help" => return Err(usage().into()),
            o => return Err(format!("unknown argument {o}\n{}", usage())),
        }
    }
    if a.static_image.is_none() && a.out.is_none() && !a.check_only {
        return Err(usage().into());
    }
    if !(1.0..=240.0).contains(&a.hz) {
        return Err("--hz must be 1..240".into());
    }
    Ok(a)
}

fn print_lines(lines: &[CheckLine]) {
    for l in lines {
        println!("  [{}] {}: {}", if l.ok { " ok " } else { "FAIL" }, l.name, l.detail);
    }
}

fn print_layout(l: &Layout) {
    println!("resolved layout (RVAs and byte offsets):");
    println!("  camera: game-state global RVA {:#x}, camera pointer at +{:#x}, eye +{:#x}, look-at +{:#x}", l.cam_global_rva, l.cam_ptr_off, l.cam_eye_off, l.cam_look_off);
    println!("  entity: table RVA {:#x}, player-index word RVA {:#x}, position +{:#x}, heading +{:#x}", l.ent_table_rva, l.ent_selector_rva, l.ent_pos_off, l.ent_yaw_off);
    println!("          effective speed +{:#x}, server speed +{:#x}, aux speed +{:#x}, id {:x?}, name {:x?}", l.ent_speed_eff_off, l.ent_speed_server_off, l.ent_speed_aux_off, l.ent_id_off, l.ent_name_off);
    println!("  projection: fov numerator (double) at RVA {:#x}, denominator {}, D3D8 SetTransform vtable slot {:#x}", l.fov_num_rva, l.fov_den.describe(), l.set_transform_slot);
    println!("  camera distance constants: min at RVA {:#x}, max at RVA {:#x}", l.dist_min_rva, l.dist_max_rva);
}

fn run_static(a: &Args, path: &PathBuf) -> Result<bool, String> {
    let mut image = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let pe = pe::parse(&image)?;
    println!("static self-check of {} ({} bytes, image base {:#x}, size {:#x})", path.display(), image.len(), pe.image_base, pe.size_of_image);
    let code_end = pe.code_ranges().iter().map(|r| r.1).max().unwrap_or(0) as usize;
    if image.len() < code_end {
        return Err(format!("{} ends before the code section ({:#x}): pass an unpacked memory image of your own FFXiMain.dll, not the packed DLL", path.display(), code_end));
    }
    // A memory-mapped dump may omit the zero tail of the last section.
    image.resize(image.len().max(pe.size_of_image as usize), 0);
    let code = pe.code_ranges();
    println!("scanning code ranges: {}", code.iter().map(|(a, b)| format!("{a:#x}-{b:#x}")).collect::<Vec<_>>().join(", "));
    // A packed DLL has a zero-filled .text: refuse early with the real reason.
    if let Some(&(lo, hi)) = code.first() {
        let nonzero = image[lo as usize..(hi as usize).min(image.len())].iter().filter(|&&b| b != 0).count();
        if nonzero < 1000 {
            return Err("the .text section is empty: this is the packed DLL; pass an unpacked memory image instead".into());
        }
    }
    let res = layout::resolve_layout(&image, &pe, pe.image_base);
    print_lines(&res.lines);
    let mut ok = res.ok();
    if let Some(l) = &res.layout {
        print_layout(l);
        if a.expect_2019 {
            println!("comparison with the known 2019 build:");
            let d = layout::diff_layout(l, &layout::expected_2019());
            print_lines(&d);
            ok &= d.iter().all(|x| x.ok);
        }
        let fov = layout::f64_at(&image, l.fov_num_rva).zip(layout::fov_den_static(&image, l.fov_den)).map(|(n, d)| layout::fov_deg(n, d));
        let dmin = layout::f32_at(&image, l.dist_min_rva);
        let dmax = layout::f32_at(&image, l.dist_max_rva);
        println!("values in the image: fov {:.4} deg (NaN = denominator is a runtime field), numerator {:?}, distance constants min {:?} max {:?}", fov.unwrap_or(f64::NAN), layout::f64_at(&image, l.fov_num_rva), dmin, dmax);
    }
    println!("{}", if ok { "SELF-CHECK PASS: every signature resolved" } else { "SELF-CHECK FAIL" });
    Ok(ok)
}

#[cfg(windows)]
fn run_live(a: &Args) -> Result<bool, String> {
    use signet_parity::live;
    use signet_parity::rows::{Q_BAD_MASK, Q_LATE, Writer};
    let t_open = std::time::Instant::now();
    let proc = loop {
        match live::open(a.pid) {
            Ok(p) => break p,
            Err(e) if t_open.elapsed().as_secs_f64() < a.wait && e.contains("no process") => {
                println!("waiting for the retail client: {e}");
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            Err(e) => return Err(e),
        }
    };
    println!("opened {} pid {} read-only (PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ); FFXiMain.dll at {:#x}, {:#x} bytes", proc.exe, proc.pid, proc.base, proc.size);
    let (image, bad_pages) = proc.read_image();
    let pe = pe::parse(&image).map_err(|e| format!("module header in the live image: {e}"))?;
    println!("module image copied ({} unreadable pages); preferred base {:#x}, actual base {:#x}{}", bad_pages, pe.image_base, proc.base, if pe.image_base == proc.base { "" } else { " (relocated)" });
    let res = layout::resolve_layout(&image, &pe, proc.base);
    print_lines(&res.lines);
    let Some(l) = res.layout.clone().filter(|_| res.ok()) else {
        println!("SELF-CHECK FAIL: signatures did not all resolve; nothing logged. Rerun the static check on the unpacked patched DLL and update the signatures.");
        return Ok(false);
    };
    print_layout(&l);
    if a.expect_2019 {
        let d = layout::diff_layout(&l, &layout::expected_2019());
        let moved = d.iter().filter(|x| !x.ok).count();
        println!("difference from the 2019 build: {moved} of {} values moved{}", d.len(), if moved > 0 { " (expected after the patch)" } else { "" });
        for x in d.iter().filter(|x| !x.ok) {
            println!("    moved: {} {}", x.name, x.detail);
        }
    }

    // Wait for a player entity, then take a short burst of samples and judge them.
    let clock = live::Clock::new();
    let mut announced = false;
    let first = loop {
        let s = live::sample(&proc, &l, 0.0);
        if s.row.quality & Q_BAD_MASK == 0 && s.player_index > 0 {
            break s;
        }
        if t_open.elapsed().as_secs_f64() > a.wait {
            println!("SELF-CHECK FAIL: no readable player entity and camera after {:.0} s (quality bits {}, player index {}). Log in to a character and stand in the world, then run again.", a.wait, s.row.quality, s.player_index);
            return Ok(false);
        }
        if !announced {
            println!("waiting for a logged-in character (player index {}, quality bits {})...", s.player_index, s.row.quality);
            announced = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let r = &first.row;
    println!("first sample: position ({:.2}, {:.2}, {:.2}), heading {:.1} deg, effective speed {:.2}, server speed {:.2}", r.pos[0], r.pos[1], r.pos[2], r.heading_deg, r.speed_eff, r.speed_srv);
    println!("              camera eye ({:.2}, {:.2}, {:.2}), look-at ({:.2}, {:.2}, {:.2}), distance {:.2}, yaw {:.1}, pitch {:.1}, fov {:.3}", r.eye[0], r.eye[1], r.eye[2], r.look[0], r.look[1], r.look[2], r.dist, r.cam_yaw, r.cam_pitch, r.fov_deg);
    let near = ((r.pos[0] - r.look[0]).powi(2) + (r.pos[1] - r.look[1]).powi(2) + (r.pos[2] - r.look[2]).powi(2)).sqrt();
    println!("              player-to-look-at distance {near:.2} yalms; player id {:?}, name length {:?}", first.server_id, first.name_len);
    let mut checks: Vec<(&str, bool)> = vec![
        ("player is near the camera look-at point (position offsets and axes agree)", near < 6.0),
        ("camera distance is inside 0.5..30 yalms", (0.5..30.0).contains(&r.dist)),
        ("fov is 20..120 degrees", (20.0..120.0).contains(&r.fov_deg)),
        ("speeds are 0..30 yalms/s", (0.0..30.0).contains(&r.speed_eff) && (0.0..30.0).contains(&r.speed_srv)),
        ("player name field holds 1..16 characters", first.name_len.is_none_or(|n| (1..=16).contains(&n))),
        ("player server id is nonzero", first.server_id.is_none_or(|v| v != 0)),
    ];
    let mut ok = true;
    for (n, c) in checks.drain(..) {
        println!("  [{}] {n}", if c { " ok " } else { "FAIL" });
        ok &= c;
    }
    match live::start_raw_input() {
        Ok(()) => println!("raw input listener registered (wheel and mouse counts, passive)"),
        Err(e) => println!("WARNING: raw input listener failed ({e}); wheel and mouse columns will be 0 (the comparator falls back to distance plateaus)"),
    }
    if !ok {
        println!("SELF-CHECK FAIL: the values look wrong; nothing logged.");
        return Ok(false);
    }
    println!("SELF-CHECK PASS");
    let Some(out) = &a.out else { return Ok(true) };
    if a.check_only {
        return Ok(true);
    }
    live::install_ctrl_c();
    let mut w = Writer::create(out)?;
    println!("logging {:.0} Hz to {} for {:.0} s; press Ctrl+C to stop early", a.hz, out.display(), a.seconds);
    let period = clock.ticks(1.0 / a.hz);
    let start = clock.now();
    let mut next = start;
    let mut last_t = -1.0f64;
    let (mut n, mut bad, mut late) = (0u64, 0u64, 0u64);
    let mut fails_in_row = 0;
    while !live::stop_requested() {
        clock.wait_until(next);
        let now = clock.now();
        let t = clock.secs(now - start);
        if t > a.seconds {
            break;
        }
        let mut s = live::sample(&proc, &l, t);
        if last_t >= 0.0 && t - last_t > 1.5 / a.hz {
            s.row.quality |= Q_LATE;
            late += 1;
        }
        last_t = t;
        if s.row.quality & Q_BAD_MASK != 0 {
            bad += 1;
            fails_in_row += 1;
            if fails_in_row > (a.hz * 3.0) as i32 {
                w.flush()?;
                return Err("the client stopped answering reads for 3 s (closed?); log saved up to here".into());
            }
        } else {
            fails_in_row = 0;
        }
        w.push(&s.row)?;
        n += 1;
        if n % (a.hz as u64 * 5) == 0 {
            println!("t {:6.1} s  pos ({:8.2},{:8.2},{:8.2})  dist {:5.2}  fov {:.2}  bad {bad}  late {late}", t, s.row.pos[0], s.row.pos[1], s.row.pos[2], s.row.dist, s.row.fov_deg);
            w.flush()?;
        }
        next += period;
        if now > next + period {
            next = now + period; // fell behind: skip, do not burst
        }
    }
    w.flush()?;
    println!("wrote {n} rows ({bad} unreadable, {late} late) to {}", out.display());
    Ok(bad * 10 < n.max(1))
}

#[cfg(not(windows))]
fn run_live(_: &Args) -> Result<bool, String> {
    Err("the live probe is Windows only".into())
}

fn main() -> ExitCode {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let r = match &a.static_image {
        Some(p) => run_static(&a, p),
        None => run_live(&a),
    };
    match r {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("signet-retail-probe: {e}");
            ExitCode::from(2)
        }
    }
}

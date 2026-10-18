use super::*;

fn key(wide: bool, flags: u32) -> Key {
    Key {
        wide,
        device_type: 4,
        flags,
    }
}
fn device(bytes: usize, marker: u8) -> Descriptor {
    let mut record = Descriptor([0; 1100]);
    record.0[..4].copy_from_slice(&(bytes as u32).to_ne_bytes());
    record.0[4] = marker;
    record
}
fn snapshot(marker: u8) -> Snapshot {
    Snapshot {
        result: OK,
        devices: vec![device(580, marker)].into(),
    }
}
fn wait_generation(cache: &Cache, selected: Key, generation: u32) {
    let control = cache.control.lock().unwrap();
    let (_control, timeout) = cache
        .wake
        .wait_timeout_while(control, Duration::from_secs(4), |_| {
            cache.entries[selected.index()]
                .generation
                .load(Ordering::Acquire)
                < generation
        })
        .unwrap();
    assert!(!timeout.timed_out(), "selected query was not published");
}

#[test]
fn only_game_controller_class_with_native_flags_is_cached() {
    for kind in [0, 1, 2, 3, 0x12, 0x13, 0x14, 0x1c, u32::MAX] {
        assert!(
            !Key {
                device_type: kind,
                ..key(false, 1)
            }
            .cached()
        );
    }
    assert!(key(false, VALID_FLAGS).cached());
    assert!(!key(false, 2).cached());
    assert!(!key(false, 0x10000).cached());
}

#[test]
fn stalled_search_and_publication_contention_retain_connected_snapshot_without_waiting() {
    let cache = Arc::new(Cache::new());
    cache.prepare(|_| snapshot(9)).unwrap();
    assert_eq!(cache.lookup(key(false, 1)).devices[0].0[4], 9);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let worker_cache = Arc::clone(&cache);
    let worker = std::thread::spawn(move || {
        worker_cache.run(|selected| {
            if selected != key(false, 1) {
                return snapshot(9);
            }
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            snapshot(10)
        })
    });
    cache.activate();
    entered_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    let query_cache = Arc::clone(&cache);
    let (query_tx, query_rx) = mpsc::sync_channel(1);
    let caller = std::thread::spawn(move || {
        for _ in 0..1000 {
            assert_eq!(query_cache.lookup(key(false, 1)).devices[0].0[4], 9);
        }
        query_tx.send(()).unwrap();
    });
    let completed_while_search_blocked = query_rx.recv_timeout(Duration::from_millis(500));
    release_tx.send(()).unwrap();
    wait_generation(&cache, key(false, 1), 2);
    cache.stop();
    worker.join().unwrap();
    caller.join().unwrap();
    assert!(completed_while_search_blocked.is_ok());
    let _publication = cache.control.lock().unwrap();
    assert_eq!(cache.lookup(key(false, 1)).devices[0].0[4], 10);
}

#[test]
fn refresh_replaces_absent_connected_disconnected_reconnected_and_failed_snapshots() {
    let cache = Arc::new(Cache::new());
    cache.prepare(|_| Snapshot::empty()).unwrap();
    cache.lookup(key(false, 1));
    let connected = Arc::new(AtomicU32::new(7));
    let source = Arc::clone(&connected);
    let worker_cache = Arc::clone(&cache);
    let worker = std::thread::spawn(move || {
        worker_cache.run(|selected| {
            if selected != key(false, 1) {
                return Snapshot::empty();
            }
            match source.load(Ordering::Relaxed) {
                0 => Snapshot::empty(),
                99 => Snapshot {
                    result: FAIL,
                    devices: Arc::from([]),
                },
                marker => snapshot(marker as u8),
            }
        })
    });
    cache.activate();
    for (generation, marker) in [(2, 7), (3, 0), (4, 8), (5, 99), (6, 9)] {
        connected.store(marker, Ordering::Relaxed);
        cache.wake.notify_all();
        wait_generation(&cache, key(false, 1), generation);
        let result = cache.lookup(key(false, 1));
        if marker == 0 {
            assert!(result.devices.is_empty());
        } else if marker == 99 {
            assert_eq!(result.result, FAIL);
            assert!(result.devices.is_empty());
        } else {
            assert_eq!(result.devices[0].0[4], marker as u8);
        }
    }
    cache.stop();
    worker.join().unwrap();
    assert_eq!(cache.errors.load(Ordering::Relaxed), 1);
}

#[test]
fn every_supported_query_is_prewarmed_and_encoding_and_flags_are_isolated() {
    let cache = Cache::new();
    cache
        .prepare(|selected| snapshot(selected.index() as u8))
        .unwrap();
    assert_eq!(cache.entries.len(), 8);
    for (index, entry) in cache.entries.iter().enumerate() {
        assert!(entry.key.cached());
        assert_eq!(entry.key.index(), index);
        assert_eq!(entry.generation.load(Ordering::Acquire), 1);
        assert_eq!(cache.lookup(entry.key).devices[0].0[4], index as u8);
    }
    assert_eq!(cache.scans.load(Ordering::Relaxed), 8);
    assert_eq!(cache.lookup(key(false, 1)).devices[0].0[4], 1);
    assert_eq!(cache.lookup(key(true, 1)).devices[0].0[4], 5);
}

#[test]
fn concurrent_publication_keeps_snapshots_alive_and_never_fabricates_disconnection() {
    let cache = Arc::new(Cache::new());
    cache.prepare(|_| snapshot(1)).unwrap();
    let publisher = Arc::clone(&cache);
    let writer = std::thread::spawn(move || {
        for i in 0..10000 {
            publisher.entries[key(false, 1).index()]
                .snapshot
                .store(Arc::new(snapshot((i % 250) as u8)));
        }
    });
    for _ in 0..10000 {
        let observed = cache.lookup(key(false, 1));
        assert_eq!(observed.result, OK);
        assert_eq!(observed.devices.len(), 1);
        assert_eq!(
            u32::from_ne_bytes(observed.devices[0].0[..4].try_into().unwrap()),
            580
        );
    }
    writer.join().unwrap();
}

#[test]
fn dormant_queries_refresh_before_their_first_game_call() {
    let cache = Arc::new(Cache::new());
    cache.prepare(|_| Snapshot::empty()).unwrap();
    let worker_cache = Arc::clone(&cache);
    let worker = std::thread::spawn(move || worker_cache.run(|_| snapshot(42)));
    cache.activate();
    // No lookup marks this encoding/flag variant requested beforehand.
    wait_generation(&cache, key(true, 0x101), 2);
    assert_eq!(cache.lookup(key(true, 0x101)).devices[0].0[4], 42);
    cache.stop();
    worker.join().unwrap();
}

#[test]
fn records_keep_ansi_unicode_sizes_and_callback_stop_context_and_reentrancy() {
    assert_eq!(std::mem::align_of::<Descriptor>(), 4);
    for bytes in INSTANCE_BYTES {
        let mut collector = Collector {
            bytes,
            devices: Vec::with_capacity(MAX_DEVICES),
            error: OK,
        };
        let first = device(bytes, 11);
        let second = device(bytes, 12);
        unsafe {
            assert_eq!(
                collect(&first, (&mut collector as *mut Collector).cast()),
                1
            );
            assert_eq!(
                collect(&second, (&mut collector as *mut Collector).cast()),
                1
            );
        }
        let cached = Snapshot {
            result: OK,
            devices: collector.devices.into(),
        };
        struct Context {
            count: u32,
            thread: std::thread::ThreadId,
            cache: Cache,
        }
        unsafe extern "system" fn callback(record: *const Descriptor, context: *mut c_void) -> i32 {
            let context = unsafe { &mut *context.cast::<Context>() };
            assert_eq!(context.thread, std::thread::current().id());
            assert_eq!(unsafe { (*record).0[4] }, 11);
            context.count += 1;
            context.cache.lookup(key(false, 1));
            0
        }
        let mut context = Context {
            count: 0,
            thread: std::thread::current().id(),
            cache: Cache::new(),
        };
        assert_eq!(
            unsafe { cached.replay(callback, (&mut context as *mut Context).cast()) },
            OK
        );
        assert_eq!(context.count, 1);
        assert_eq!(
            u32::from_ne_bytes(cached.devices[0].0[..4].try_into().unwrap()) as usize,
            bytes
        );
    }
}

#[test]
fn native_collector_rejects_wrong_sizes_and_overflow_without_partial_results() {
    let mut collector = Collector {
        bytes: 580,
        devices: Vec::with_capacity(MAX_DEVICES),
        error: OK,
    };
    let bad = device(1100, 1);
    assert_eq!(
        unsafe { collect(&bad, (&mut collector as *mut Collector).cast()) },
        0
    );
    assert_eq!(collector.error, FAIL);
    collector.error = OK;
    let good = device(580, 1);
    for _ in 0..MAX_DEVICES {
        assert_eq!(
            unsafe { collect(&good, (&mut collector as *mut Collector).cast()) },
            1
        );
    }
    assert_eq!(
        unsafe { collect(&good, (&mut collector as *mut Collector).cast()) },
        0
    );
    assert_eq!(collector.error, OUT_OF_MEMORY);
}

// Real system-DLL interception in this standalone Rust test process. No
// injection, game, launcher, window, audio or network is involved.
#[test]
fn real_system_directinput_hooks_preserve_initial_warm_and_noncontroller_results() {
    unsafe { install() }.expect("install system DirectInput interception");
    let native = unsafe { Native::new() }.unwrap();
    let hooks = HOOKS.get().unwrap();
    for wide in [false, true] {
        let index = usize::from(wide);
        let method: Enumerate = unsafe { std::mem::transmute(native.targets()[index]) };
        let mut collector = Collector {
            bytes: INSTANCE_BYTES[index],
            devices: Vec::with_capacity(MAX_DEVICES),
            error: OK,
        };
        let before = counter(1);
        let result = unsafe {
            method(
                native.objects[index],
                2,
                Some(collect),
                (&mut collector as *mut Collector).cast(),
                1,
            )
        };
        assert!(result >= 0, "native keyboard enumeration {result:#x}");
        assert_eq!(counter(1), before);
        for refresh in [false, true] {
            let selected = key(wide, 1);
            if refresh {
                let generation = hooks.cache.entries[selected.index()]
                    .generation
                    .load(Ordering::Acquire);
                hooks.cache.wake.notify_all();
                wait_generation(&hooks.cache, selected, generation + 1);
            }
            assert!(
                hooks.cache.entries[selected.index()]
                    .generation
                    .load(Ordering::Acquire)
                    >= 1
            );
            let expected = unsafe { native.scan(selected) };
            collector.devices.clear();
            let before = counter(1);
            let result = unsafe {
                method(
                    native.objects[index],
                    4,
                    Some(collect),
                    (&mut collector as *mut Collector).cast(),
                    1,
                )
            };
            assert_eq!(counter(1), before + 1);
            assert_eq!(result, expected.result);
            assert_eq!(collector.devices.len(), expected.devices.len());
            for (a, b) in collector.devices.iter().zip(expected.devices.iter()) {
                assert_eq!(a.0, b.0);
            }
        }
        let original: Enumerate = unsafe { std::mem::transmute(hooks.detours[index].trampoline()) };
        assert_eq!(
            unsafe { method(native.objects[index], 4, None, ptr::null_mut(), 2) },
            unsafe { original(native.objects[index], 4, None, ptr::null_mut(), 2) }
        );
    }
    println!("controller-discovery-system-evidence={}", observe());
}

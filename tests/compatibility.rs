use windower_rs::{
    profile::{Region, Settings, initialization_command},
    protocol::*,
};

#[cfg(all(windows, target_pointer_width = "32"))]
#[test]
fn windows_child_arguments_quote_spaces_backslashes_and_utf16() {
    use windower_rs::launch::quote_units;
    let quote =
        |s: &str| String::from_utf16(&quote_units(&s.encode_utf16().collect::<Vec<_>>())).unwrap();
    assert_eq!(quote(""), "\"\"");
    assert_eq!(quote("plain"), "plain");
    assert_eq!(quote("C:\\My Game\\"), "\"C:\\My Game\\\\\"");
    assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    assert_eq!(
        quote_units(&[0xd800, 32, 0xdc00]),
        vec![34, 0xd800, 32, 0xdc00, 34]
    );
}

fn context() -> LaunchContext {
    LaunchContext {
        branch: "live".into(),
        launcher_version: "4.7.9.3".into(),
        hook_version: "4.7.9.3".into(),
        play_online_path: "E:/Games/POL".into(),
        ffxi_path: "E:/Games/FFXI".into(),
        windower_path: "E:/Windower".into(),
        region: Region::US,
        display_width: 1920,
        display_height: 1080,
        plugins: vec!["LuaCore".into()],
        addons: vec![],
    }
}

#[test]
fn constructor_and_xml_defaults_remain_distinct() {
    assert_eq!(
        Settings::parse("<settings/>").unwrap().select("").ui_scale,
        0.0
    );
    assert_eq!(
        Settings::parse("<settings><profile/></settings>")
            .unwrap()
            .select("")
            .ui_scale,
        1.0
    );
}

#[test]
fn autoload_deduplicates_exact_names_without_reordering_or_trimming() {
    let s = Settings::parse("<settings><autoload><addon>GearSwap</addon><addon>GearSwap</addon><addon>gearswap</addon><addon> XIPivot </addon><addon> </addon><plugin>Config</plugin></autoload></settings>").unwrap();
    assert_eq!(s.addons, ["GearSwap", "gearswap", " XIPivot "]);
    assert_eq!(s.plugins, ["Config"]);
}

#[test]
fn names_and_invalid_fields_follow_reference_fallbacks() {
    let s = Settings::parse("<settings><profile name=''><width>1600</width></profile><profile name=' Raid '><width>oops</width><region>US</region></profile></settings>").unwrap();
    assert_eq!(s.select("RAID").width, 0);
    assert_eq!(s.select("RAID").region, Region::US);
    assert_eq!(s.select("missing").width, 1600);
    assert!(
        Settings::parse("<settings><profile name='a'/><profile name='a'/></settings>").is_err()
    );
}

#[test]
fn initialization_filter_is_case_sensitive_and_preserves_spacing() {
    assert_eq!(
        initialization_command(Some(" load x ;Load y;exec z;echo ok")),
        "echo Command not supported in initialization:  load x ;Load y;echo Command not supported in initialization: exec z;echo ok"
    );
}

#[test]
fn payload_has_reference_layout_and_alignment() {
    let p = Settings::parse("<settings><profile/></settings>")
        .unwrap()
        .select("");
    let bytes = encode_settings(&p, &context()).unwrap();
    assert_eq!(bytes.len(), 42180);
    assert_eq!(&bytes[25248..25252], &1_i32.to_le_bytes());
    assert_eq!(&bytes[25252..25256], &1920_i32.to_le_bytes());
    assert_eq!(&bytes[25276..25284], &[0, 1, 1, 0, 0, 0, 0, 0]);
    assert_eq!(
        &bytes[33476..33490],
        &[76, 0, 117, 0, 97, 0, 67, 0, 111, 0, 114, 0, 101, 0]
    );
}

#[test]
fn unsafe_payload_values_fail_without_truncation() {
    let mut p = Settings::parse("<settings><profile/></settings>")
        .unwrap()
        .select("");
    p.name = "a".repeat(256);
    assert!(encode_settings(&p, &context()).is_err());
    p.name = "safe".into();
    let mut c = context();
    c.ffxi_path = "E:\\bad\0path".into();
    assert!(encode_settings(&p, &c).is_err());
    c = context();
    c.region = Region::Auto;
    assert!(encode_settings(&p, &c).is_err());
}

#[test]
fn malformed_xml_and_external_entities_are_rejected() {
    assert!(Settings::parse("<settings>").is_err());
    assert!(Settings::parse("<!DOCTYPE settings [<!ENTITY data SYSTEM 'file:///private'>]><settings><profile>&data;</profile></settings>").is_err());
}

#[test]
fn runtime_decodes_payload_and_rejects_corruption() {
    let p = Settings::parse("<settings><profile name='日本語 😀'/></settings>")
        .unwrap()
        .select("日本語 😀");
    let bytes = encode_settings(&p, &context()).unwrap();
    let decoded = decode_settings(&bytes).unwrap();
    assert_eq!(decoded.profile_name, "日本語 😀");
    assert_eq!(decoded.init_plugins, "LuaCore");
    assert_eq!(decoded.width, 1920);
    assert_eq!(decoded.ui_width, 1920);
    assert!(decode_settings(&bytes[..100]).is_err());
    let mut invalid = bytes.clone();
    invalid[25248..25252].copy_from_slice(&(-1_i32).to_le_bytes());
    assert!(decode_settings(&invalid).is_err());
    let mut invalid = bytes.clone();
    invalid[25276] = 2;
    assert!(decode_settings(&invalid).is_err());
    let mut invalid = bytes;
    invalid[..512].fill(65);
    assert!(decode_settings(&invalid).is_err());
}
#[cfg(windows)]
#[test]
fn legacy_process_paths_do_not_expose_extended_prefixes_to_pol() {
    let normalized = |path: &str| {
        String::from_utf16(
            &windower_rs::launch::normal_process_path(&path.encode_utf16().collect::<Vec<_>>())
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(
        normalized(r"\\?\E:\PlayOnline\pol.exe"),
        r"E:\PlayOnline\pol.exe"
    );
    assert_eq!(
        normalized(r"\\?\UNC\server\share\pol.exe"),
        r"\\server\share\pol.exe"
    );
    assert_eq!(
        normalized(r"E:\Play Online\pol.exe"),
        r"E:\Play Online\pol.exe"
    );
    assert!(
        windower_rs::launch::normal_process_path(
            &r"\\?\Volume{unknown}\pol.exe"
                .encode_utf16()
                .collect::<Vec<_>>()
        )
        .is_err()
    );
}

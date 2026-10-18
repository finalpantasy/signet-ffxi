use serde_json::json;
use windower_rs::player::{Player, Request};
use windower_rs::{addon_profile::AddonProfile, launcher_document::Document};
fn fixture() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "signet-player-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("addons/example")).unwrap();
    std::fs::write(root.join("addons/example/example.lua"), b"return 1").unwrap();
    let doc = Document {
        schema_version: 1,
        name: "Classic".into(),
        width: 1280,
        height: 720,
        ui_scale: 1.0,
        profile: AddonProfile {
            schema_version: 1,
            root: root.clone(),
            addons: vec![
                serde_json::from_value(
                    json!({"name":"example","ecosystem":"native","engine":"lua51"}),
                )
                .unwrap(),
            ],
            luajit_provider: None,
        },
    };
    doc.save_new(&root).unwrap();
    root
}
#[test]
fn profile_saves_are_native_versions_and_stale_edits_preserve_neighbors() {
    let root = fixture();
    let player = Player::open(&root).unwrap();
    let before = player.snapshot().unwrap();
    let id = before["profiles"][0]["id"].as_str().unwrap();
    let save = json!({"schema_version":1,"action":"save_profile","id":id,"name":"Classic","width":1920,"height":1080,"ui_scale":1.25,"enabled":[]});
    let after = player
        .execute(serde_json::from_value(save.clone()).unwrap())
        .unwrap();
    assert_eq!(after["profiles"][0]["document"]["width"], 1920);
    assert_eq!(
        after["profiles"][0]["document"]["profile"]["addons"],
        json!([])
    );
    assert_eq!(
        after["profiles"][0]["available_addons"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        Document::read(&root.join("profiles").join(format!("{id}.json")))
            .unwrap()
            .width,
        1280
    );
    assert!(
        player
            .execute(serde_json::from_value(save).unwrap())
            .unwrap_err()
            .contains("changed")
    );
    assert_eq!(player.snapshot().unwrap(), after);
    assert!(Player::open(&root).is_err(), "one writer per workspace");
    drop(player);
    assert!(Player::open(&root).is_ok());
}
#[test]
fn foreign_profiles_invalid_settings_and_unknown_fields_are_refused() {
    let root = fixture();
    let player = Player::open(&root).unwrap();
    let s = player.snapshot().unwrap();
    let id = s["profiles"][0]["id"].as_str().unwrap();
    let base = json!({"schema_version":1,"action":"save_profile","id":id,"name":"Classic","width":1280,"height":720,"ui_scale":1.0,"enabled":["example"]});
    for (key, value) in [
        ("id", json!("../escape")),
        ("name", json!("../escape")),
        ("width", json!(0)),
        ("enabled", json!(["unknown"])),
        ("enabled", json!(["example", "example"])),
    ] {
        let mut bad = base.clone();
        bad[key] = value;
        assert!(
            player
                .execute(serde_json::from_value(bad).unwrap())
                .is_err()
        );
        assert_eq!(player.snapshot().unwrap(), s);
    }
    let mut bad = base;
    bad["unexpected"] = json!(true);
    assert!(serde_json::from_value::<Request>(bad).is_err());
}
#[test]
fn copies_keep_available_addons_and_restore_creates_a_new_native_version() {
    let root = fixture();
    let player = Player::open(&root).unwrap();
    let first = player.snapshot().unwrap();
    let id = first["profiles"][0]["id"].as_str().unwrap();
    let saved=player.execute(serde_json::from_value(json!({"schema_version":1,"action":"save_profile","id":id,"name":"Classic","width":1920,"height":1080,"ui_scale":1.0,"enabled":[]})).unwrap()).unwrap();
    let changed = saved["profiles"][0]["id"].as_str().unwrap();
    let copied = player
        .execute(
            serde_json::from_value(
                json!({"schema_version":1,"action":"copy_profile","id":changed,"name":"Evening"}),
            )
            .unwrap(),
        )
        .unwrap();
    let copy = copied["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["document"]["name"] == "Evening")
        .unwrap();
    assert_eq!(
        copy["available_addons"].as_array().unwrap().len(),
        1,
        "disabled addon remains available in copied profile"
    );
    assert_eq!(copy["document"]["profile"]["addons"], json!([]));
    let restored = player
        .execute(
            serde_json::from_value(
                json!({"schema_version":1,"action":"restore_profile","id":changed,"version":id}),
            )
            .unwrap(),
        )
        .unwrap();
    let classic = restored["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["document"]["name"] == "Classic")
        .unwrap();
    assert_ne!(classic["id"], id);
    assert_ne!(classic["id"], changed);
    assert_eq!(classic["document"]["width"], 1280);
    assert_eq!(
        classic["document"]["profile"]["addons"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(player.execute(serde_json::from_value(json!({"schema_version":1,"action":"restore_profile","id":copy["id"],"version":id})).unwrap()).is_err());
}
#[test]
fn shared_catalog_does_not_replace_a_profiles_ecosystem() {
    let root = fixture();
    let mut neighbor = Document::read(&root.join("profiles/Classic-0001.json")).unwrap();
    neighbor.name = "Windower".into();
    neighbor.profile.addons[0].ecosystem = windower_rs::platform::Ecosystem::Windower4;
    neighbor.save_new(&root).unwrap();
    let player = Player::open(&root).unwrap();
    let snapshot = player.snapshot().unwrap();
    let classic = snapshot["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["document"]["name"] == "Classic")
        .unwrap();
    assert_eq!(classic["available_addons"][0]["ecosystem"], "native");
}

#[test]
fn reaching_profile_capacity_refuses_save_without_poisoning_workspace() {
    let root = fixture();
    let original = std::fs::read(root.join("profiles/Classic-0001.json")).unwrap();
    for sequence in 2..=4096 {
        std::fs::write(
            root.join(format!("profiles/Classic-{sequence:04}.json")),
            &original,
        )
        .unwrap();
    }
    let player = Player::open(&root).unwrap();
    let before = player.snapshot().unwrap();
    let id = before["profiles"][0]["id"].as_str().unwrap();
    let request = serde_json::from_value(
        json!({"schema_version":1,"action":"copy_profile","id":id,"name":"CapacityCopy"}),
    )
    .unwrap();
    assert!(player.execute(request).is_err());
    assert_eq!(
        std::fs::read_dir(root.join("profiles")).unwrap().count(),
        4096,
        "refused append must not create an unreadable over-capacity workspace"
    );
    assert_eq!(player.snapshot().unwrap(), before);
    drop(player);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn startup_refuses_a_historical_profile_revision() {
    let root = fixture();
    let player = Player::open(&root).unwrap();
    let before = player.snapshot().unwrap();
    let id = before["profiles"][0]["id"].as_str().unwrap();
    let after=player.execute(serde_json::from_value(json!({"schema_version":1,"action":"save_profile","id":id,"name":"Classic","width":1280,"height":720,"ui_scale":1.0,"enabled":[]})).unwrap()).unwrap();
    assert!(player.current_profile(id).is_err());
    assert!(
        player
            .current_profile(after["profiles"][0]["id"].as_str().unwrap())
            .unwrap()
            .profile
            .addons
            .is_empty()
    );
    assert!(
        player.profile(id).is_ok(),
        "history remains available for restore"
    );
}
#[test]
fn damaged_and_interrupted_versions_preserve_recoverable_profiles_with_warnings() {
    let root = fixture();
    std::fs::write(root.join("profiles/Classic-0002.json"), b"{partial").unwrap();
    std::fs::write(root.join("profiles/.interrupted.pending"), b"{partial").unwrap();
    let player = Player::open(&root).unwrap();
    let state = player.snapshot().unwrap();
    assert_eq!(state["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(state["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(state["profiles"][0]["requires_restore"], true);
    assert!(
        player.current_profile("Classic-0001").is_err(),
        "damaged newer revision must not silently launch old settings"
    );
    let recovered=player.execute(serde_json::from_value(json!({"schema_version":1,"action":"restore_profile","id":"Classic-0001","version":"Classic-0001"})).unwrap()).unwrap();
    assert_eq!(recovered["profiles"][0]["requires_restore"], false);
    assert!(
        player
            .current_profile(recovered["profiles"][0]["id"].as_str().unwrap())
            .is_ok()
    );
    assert_eq!(
        std::fs::read(root.join("profiles/Classic-0002.json")).unwrap(),
        b"{partial"
    );
}

#[test]
fn profile_revision_checks_keep_similarly_named_profiles_independent() {
    let root = fixture();
    let mut neighbor = Document::read(&root.join("profiles/Classic-0001.json")).unwrap();
    neighbor.name = "Classic-extra".into();
    neighbor.save_new(&root).unwrap();
    let player = Player::open(&root).unwrap();
    assert!(player.current_profile("Classic-0001").is_ok());
    assert!(player.current_profile("Classic-extra-0001").is_ok());
}

#[test]
fn revision_order_is_numeric_at_the_native_version_boundary() {
    let root = fixture();
    let old = std::fs::read(root.join("profiles/Classic-0001.json")).unwrap();
    std::fs::write(root.join("profiles/Classic-9999.json"), &old).unwrap();
    let mut newest: Document = serde_json::from_slice(&old).unwrap();
    newest.width = 1920;
    std::fs::write(
        root.join("profiles/Classic-10000.json"),
        serde_json::to_vec(&newest).unwrap(),
    )
    .unwrap();
    let player = Player::open(&root).unwrap();
    assert_eq!(
        player.snapshot().unwrap()["profiles"][0]["id"],
        "Classic-10000"
    );
    assert!(player.current_profile("Classic-9999").is_err());
    assert_eq!(player.current_profile("Classic-10000").unwrap().width, 1920);
}

#[test]
fn addon_case_aliases_share_one_catalog_entry_and_keep_own_declarations() {
    let root = fixture();
    let mut own = Document::read(&root.join("profiles/Classic-0001.json")).unwrap();
    let mut neighbor = own.clone();
    neighbor.name = "Evening".into();
    neighbor.profile.addons[0].name = "Example".into();
    neighbor.profile.addons[0].ecosystem = windower_rs::platform::Ecosystem::Windower4;
    neighbor.save_new(&root).unwrap();
    own.profile.addons.clear();
    own.save_new(&root).unwrap();
    let player = Player::open(&root).unwrap();
    let snapshot = player.snapshot().unwrap();
    for (name, spelling, ecosystem) in [
        ("Classic", "example", "native"),
        ("Evening", "Example", "windower4"),
    ] {
        let view = snapshot["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["document"]["name"] == name)
            .unwrap();
        let catalog = view["available_addons"].as_array().unwrap();
        assert_eq!(
            catalog.len(),
            1,
            "case aliases must not become conflicting toggles"
        );
        assert_eq!(catalog[0]["name"], spelling);
        assert_eq!(catalog[0]["ecosystem"], ecosystem);
        let after = player
            .execute(
                serde_json::from_value(json!({
                    "schema_version":1,"action":"save_profile","id":view["id"],
                    "name":name,"width":1280,"height":720,"ui_scale":1.0,"enabled":[spelling]
                }))
                .unwrap(),
            )
            .unwrap();
        let saved = after["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["document"]["name"] == name)
            .unwrap();
        assert_eq!(
            saved["document"]["profile"]["addons"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(saved["document"]["profile"]["addons"][0]["name"], spelling);
        assert_eq!(
            saved["document"]["profile"]["addons"][0]["ecosystem"],
            ecosystem
        );
    }
}

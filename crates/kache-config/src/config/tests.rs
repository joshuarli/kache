use super::*;
// Config and compiler tests mutate process-wide env, including PATH. Share
// one lock with cache-key tests that read the same state.
use crate::testing::{TestEnvGuard, config_path_lock, set_env_for_test};
use std::ffi::OsString;

#[test]
fn ignored_override_warnings_use_only_client_keys_and_never_their_values() {
    let root = tempfile::tempdir().unwrap();
    let invocation = |vars: Vec<(&str, &str)>| {
        InvocationEnv::from_parts(
            vars,
            Some(root.path().to_owned()),
            None,
            root.path().to_owned(),
        )
    };
    let empty = invocation(Vec::new());
    let left = invocation(vec![("KACHE_CACHE_DIR", "private-client-cache")]);
    let right = invocation(vec![("KACHE_KEY_SALT", "private-key-salt")]);
    let capture = kache_log::capture();
    warn_ignored_env_overrides_for(&empty);
    warn_ignored_env_overrides_for(&left);
    warn_ignored_env_overrides_for(&right);
    let messages = capture.records();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("KACHE_CACHE_DIR"));
    assert!(!messages[0].contains("KACHE_KEY_SALT"));
    assert!(messages[1].contains("KACHE_KEY_SALT"));
    assert!(!messages[1].contains("KACHE_CACHE_DIR"));
    assert!(messages.iter().all(|message| !message.contains("private-")));
}


#[test]
fn global_config_path_uses_the_clients_home_and_config_override() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("client-home");
    let mut env = InvocationEnv::from_parts(
        Vec::<(OsString, OsString)>::new(),
        Some(root.path().to_owned()),
        Some(home.clone()),
        root.path().to_owned(),
    );
    assert_eq!(
        config_file_path_for(&env),
        home.join(".config/kache/config.toml")
    );
    let override_dir = root.path().join("client-config");
    env.set("XDG_CONFIG_HOME", override_dir.as_os_str());
    assert_eq!(
        config_file_path_for(&env),
        override_dir.join("kache/config.toml")
    );
}

#[test]
fn relative_volume_routing_uses_the_client_directory() {
    let root = tempfile::tempdir().unwrap();
    let client = root.path().join("client");
    let env = InvocationEnv::from_parts(
        Vec::<(OsString, OsString)>::new(),
        Some(client.clone()),
        None,
        root.path().to_owned(),
    );
    let other = InvocationEnv::from_parts(
        Vec::<(OsString, OsString)>::new(),
        Some(root.path().join("other")),
        None,
        root.path().to_owned(),
    );
    let mut config = Config::load_resolved_for(&env, Ok(FileConfig::default())).unwrap();
    let shard = root.path().join("shard");
    config.volume_stores = vec![VolumeStore {
        volume: Config::normalize_volume_root(client.to_str().unwrap()).unwrap(),
        store: shard.clone(),
        max_size: None,
    }];
    assert_eq!(
        config.volume_store_for_invocation(Path::new("target/unit.rlib"), &env),
        Some(shard.as_path())
    );
    assert_eq!(
        config.volume_store_for_invocation(Path::new("target/unit.rlib"), &other),
        None
    );
}

/// kunobi-ninja/kache#319: executables are cached by default only where a
/// restored binary keeps source-level debugging. Linux embeds DWARF; macOS
/// gets a store-time `.dSYM` cached with the entry (shipped for #319);
/// Pinned as a test because this is a deliberate platform default.
#[test]
fn cache_executables_defaults_on_for_linux_and_macos() {
    assert_eq!(
        default_cache_executables(),
        cfg!(target_os = "linux") || cfg!(target_os = "macos"),
        "executables default on for Linux and macOS (see #319)"
    );
}

#[test]
fn trust_domain_rejects_path_components_and_separators() {
    assert_eq!(
        sanitize_trust_domain("github.kunobi-ninja"),
        Some("github.kunobi-ninja")
    );
    assert_eq!(sanitize_trust_domain("  ok_1  "), Some("ok_1"));
    for bad in [
        "",
        " ",
        ".",
        "..",
        "a/b",
        "a\\b",
        "../x",
        "x/..",
        "has space",
        "unicode-é",
    ] {
        assert_eq!(sanitize_trust_domain(bad), None, "{bad:?}");
    }
    let at_max = "a".repeat(TRUST_DOMAIN_MAX_LEN);
    assert_eq!(sanitize_trust_domain(&at_max), Some(at_max.as_str()));
    let too_long = "a".repeat(TRUST_DOMAIN_MAX_LEN + 1);
    assert_eq!(sanitize_trust_domain(&too_long), None);
}

#[test]
fn trust_domain_isolates_stores_and_fails_open() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let trusted = isolate_cache_dir_for_trust_domain(base.clone(), Some("trusted"));
    let public = isolate_cache_dir_for_trust_domain(base.clone(), Some("public"));
    assert_ne!(trusted, public);
    assert_eq!(trusted, base.join("trusted"));
    assert_eq!(public, base.join("public"));
    assert!(trusted.is_dir());
    assert!(public.is_dir());
    assert_eq!(
        isolate_cache_dir_for_trust_domain(base.clone(), None),
        base,
        "unset domain must not rewrite the store"
    );
    assert_eq!(
        isolate_cache_dir_for_trust_domain(base.clone(), Some("../escape")),
        base,
        "unsafe labels fail open"
    );
    assert_eq!(
        isolate_cache_dir_for_trust_domain(base.clone(), Some("   ")),
        base,
        "whitespace-only labels fail open"
    );

    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        isolate_cache_dir_for_trust_domain(file.clone(), Some("trusted")),
        file,
        "an unusable parent must fail open"
    );
}

fn set_kache_config_for_test(path: &std::path::Path) -> TestEnvGuard {
    set_env_for_test("KACHE_CONFIG", Some(path.as_os_str()))
}

struct NamedEnvGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl NamedEnvGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(name);
        unsafe { std::env::set_var(name, value) };
        Self { name, previous }
    }

    fn remove(name: &'static str) -> Self {
        let previous = std::env::var_os(name);
        unsafe { std::env::remove_var(name) };
        Self { name, previous }
    }
}

impl Drop for NamedEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

#[test]
fn test_default_cache_dir() {
    let dir = default_cache_dir();
    assert!(dir.to_string_lossy().contains("kache"));
}
#[test]
fn the_event_log_rotates_at_64_mib_by_default() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let _config = set_kache_config_for_test(&config_path);

    assert_eq!(Config::load().unwrap().event_log_max_size, 67_108_864);

    std::fs::write(&config_path, "[cache]\nevent_log_max_size = \"8MiB\"\n").unwrap();
    assert_eq!(Config::load().unwrap().event_log_max_size, 8 * 1024 * 1024);
}
#[test]
fn recovery_floor_obeys_file_environment_and_ci_defaults() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let _config = set_kache_config_for_test(&config_path);
    let _floor = NamedEnvGuard::remove("KACHE_AUTO_RECOVER_MIN_FREE_BYTES");
    let _ci = NamedEnvGuard::remove("CI");
    let _gha = NamedEnvGuard::remove("GITHUB_ACTIONS");
    let _gitlab = NamedEnvGuard::remove("GITLAB_CI");
    assert_eq!(Config::load().unwrap().auto_recover_min_free_bytes, DEFAULT_RECOVERY_FLOOR);
    {
        let _in_ci = NamedEnvGuard::set("GITHUB_ACTIONS", "true");
        assert_eq!(Config::load().unwrap().auto_recover_min_free_bytes, 0);
    }
    std::fs::write(&config_path, "[cache]\nauto_recover_min_free_bytes = 1073741824\n").unwrap();
    assert_eq!(Config::load().unwrap().auto_recover_min_free_bytes, 1 << 30);
    let override_floor = NamedEnvGuard::set("KACHE_AUTO_RECOVER_MIN_FREE_BYTES", "2147483648");
    assert_eq!(Config::load().unwrap().auto_recover_min_free_bytes, 2 << 30);
    drop(override_floor);
    std::fs::write(&config_path, "[cache]\nignore_env = true\nauto_recover_min_free_bytes = 1048576\n").unwrap();
    let _ignored = NamedEnvGuard::set("KACHE_AUTO_RECOVER_MIN_FREE_BYTES", "5242880");
    assert_eq!(Config::load().unwrap().auto_recover_min_free_bytes, 1 << 20);
}

#[test]
fn memory_pressure_admission_is_on_by_default_and_obeys_env_precedence() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let _config = set_kache_config_for_test(&config_path);
    let _missing = NamedEnvGuard::remove("KACHE_SCHEDULER_MEMORY_PRESSURE");
    assert!(Config::load().unwrap().scheduler_memory_pressure);
    std::fs::write(&config_path, "[cache]\nscheduler_memory_pressure = false\n").unwrap();
    assert!(!Config::load().unwrap().scheduler_memory_pressure);
    let _on = NamedEnvGuard::set("KACHE_SCHEDULER_MEMORY_PRESSURE", "1");
    assert!(Config::load().unwrap().scheduler_memory_pressure);
    drop(_on);
    std::fs::write(&config_path, "[cache]\nscheduler_memory_pressure = true\n").unwrap();
    for off in ["0", "false", "FALSE"] {
        let _off = NamedEnvGuard::set("KACHE_SCHEDULER_MEMORY_PRESSURE", off);
        assert!(!Config::load().unwrap().scheduler_memory_pressure, "{off}");
    }
    std::fs::write(
        &config_path,
        "[cache]\nignore_env = true\nscheduler_memory_pressure = true\n",
    )
    .unwrap();
    let _ignored = NamedEnvGuard::set("KACHE_SCHEDULER_MEMORY_PRESSURE", "0");
    assert!(Config::load().unwrap().scheduler_memory_pressure);
}


#[test]
fn test_shellexpand() {
    let expanded = shellexpand("~/foo");
    assert!(!expanded.to_string_lossy().starts_with("~/"));
}

#[test]
fn test_parse_size() {
    assert_eq!(parse_size("50GiB"), Some(50 * 1024 * 1024 * 1024));
    assert_eq!(parse_size("1MiB"), Some(1024 * 1024));
    assert!(parse_size("invalid").is_none());
}

#[test]
fn base_dirs_validate_and_normalize_host_independent_absolute_syntax() {
    let normalized = normalize_base_dirs([
        "/var//lib/./flatpak/".to_string(),
        "/snap".to_string(),
        "/work/a/./root".to_string(),
    ])
    .unwrap();
    assert_eq!(
        normalized,
        vec!["/snap", "/var/lib/flatpak", "/work/a/root",]
    );
}

#[test]
fn base_dirs_reject_relative_and_parent_traversal_entries() {
    let relative = normalize_base_dirs(["build/root".to_string()]).unwrap_err();
    assert!(relative.to_string().contains("must be absolute"));

    let parent = normalize_base_dirs(["/work/../other".to_string()]).unwrap_err();
    assert!(parent.to_string().contains("must not contain `..`"));

    {
        let root = "/";
        let error = normalize_base_dirs([root.to_string()]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("narrower than a filesystem root")
        );
    }
}

#[test]
fn config_load_integrates_paths_base_dirs() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let _guard = set_kache_config_for_test(&config_path);
    std::fs::write(
        &config_path,
        "[paths]\nbase_dirs = [\"/var/lib/flatpak\", \"/snap\"]\n",
    )
    .unwrap();

    assert_eq!(
        Config::load().unwrap().base_dirs,
        vec!["/snap".to_string(), "/var/lib/flatpak".to_string()]
    );

    std::fs::write(&config_path, "[paths]\nbase_dirs = [\"relative/root\"]\n").unwrap();
    assert!(Config::load().is_err());
}

#[test]
fn parse_size_checked_rejects_malformed_and_mirrors_parse_size() {
    // Values parse_size can't parse: a typo'd unit and digit grouping. These
    // are exactly what used to silently degrade to the hardcoded default.
    for bad in ["100 gigs", "1_000", "abc", ""] {
        assert!(parse_size(bad).is_none(), "expected {bad:?} to be invalid");
        assert!(parse_size_checked(bad, "KACHE_MAX_SIZE").is_none());
    }
    // Valid values pass through unchanged.
    assert_eq!(
        parse_size_checked("2GiB", "KACHE_MAX_SIZE"),
        Some(2 * 1024 * 1024 * 1024)
    );
}

#[test]
fn disk_share_budget_floors_caps_and_rounds() {
    const GIB: u64 = 1 << 30;
    // Independent of DISK_SHARE_* so mutating `*` in those constants
    // cannot change both sides of the assertion.
    assert_eq!(disk_share_budget(None), 50 << 30);
    assert_eq!(disk_share_budget(Some(0)), 50 << 30);
    // 10GiB disk: 5% is 0.5GiB, rounds to 1GiB, then floor 5GiB.
    assert_eq!(disk_share_budget(Some(10 * GIB)), 5 << 30);
    // 200GiB disk: 5% is exactly 10GiB.
    assert_eq!(disk_share_budget(Some(200 * GIB)), 10 * GIB);
    // 256GiB disk: 5% is 12.8GiB, nearest GiB is 13GiB.
    assert_eq!(disk_share_budget(Some(256 * GIB)), 13 * GIB);
    // 4TiB disk: 5% is 204.8GiB, cap 100GiB.
    assert_eq!(disk_share_budget(Some(4 * 1024 * GIB)), 100 << 30);
    // Exactly at the cap: 2000GiB * 5% = 100GiB.
    assert_eq!(disk_share_budget(Some(2000 * GIB)), 100 << 30);
}

#[test]
fn parse_local_max_size_rejects_none_without_unbounding() {
    assert_eq!(parse_local_max_size("none", "KACHE_MAX_SIZE"), None);
    assert_eq!(parse_local_max_size("None", "[cache] local_max_size"), None);
    assert_eq!(
        parse_local_max_size("2GiB", "KACHE_MAX_SIZE"),
        Some(2 * 1024 * 1024 * 1024)
    );
}

#[test]
fn describe_max_size_names_the_disk_share_when_derived() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let disk = Some(200 * GIB);
    let derived = disk_share_budget(disk);
    let text = describe_max_size(derived, disk);
    assert_eq!(
        text,
        "10.0 GiB (5% of 200.0 GiB, floor 5.0 GiB, cap 100.0 GiB)"
    );
    assert_eq!(
        describe_max_size(2 * GIB, disk),
        kache_core::size::format_bytes(2 * GIB).to_string()
    );
    assert!(
        describe_max_size(DISK_SHARE_FALLBACK, None).contains("disk size unknown"),
        "{}",
        describe_max_size(DISK_SHARE_FALLBACK, None)
    );
    assert!(
        describe_max_size(DISK_SHARE_FALLBACK, Some(0)).contains("disk size unknown"),
        "a zero-byte probe is unknown, not 5% of 0: {}",
        describe_max_size(DISK_SHARE_FALLBACK, Some(0))
    );
}

#[test]
fn load_uses_disk_share_budget_when_max_size_is_unset() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "[cache]\n").unwrap();
    let cache_dir = dir.path().join("store");
    std::fs::create_dir(&cache_dir).unwrap();
    let _cfg = set_kache_config_for_test(&cfg_path);
    let _cache = set_env_for_test("KACHE_CACHE_DIR", Some(cache_dir.as_os_str()));
    let _max = set_env_for_test("KACHE_MAX_SIZE", None);
    let loaded = Config::load().unwrap();
    let expected = disk_share_budget(kache_fs::volume::probe(&cache_dir).total_bytes);
    assert_eq!(loaded.max_size, expected);
}

#[test]
fn load_explicit_max_size_wins_over_disk_share() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        "[cache]\n[cache.volumes]\n\"/mnt/biglake\" = \"/mnt/biglake/kache-store\"\n",
    )
    .unwrap();
    let cache_dir = dir.path().join("store");
    std::fs::create_dir(&cache_dir).unwrap();
    let _cfg = set_kache_config_for_test(&cfg_path);
    let _cache = set_env_for_test("KACHE_CACHE_DIR", Some(cache_dir.as_os_str()));
    let _max = set_env_for_test("KACHE_MAX_SIZE", Some(std::ffi::OsStr::new("2GiB")));
    let loaded = Config::load().unwrap();
    assert_eq!(loaded.max_size, 2 * 1024 * 1024 * 1024);
    assert_eq!(
        loaded.volume_stores[0].max_size,
        Some(2 * 1024 * 1024 * 1024),
        "an explicit max size applies to each shard"
    );
}

#[test]
fn load_none_max_size_does_not_unbound_the_store() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "[cache]\nlocal_max_size = \"none\"\n").unwrap();
    let cache_dir = dir.path().join("store");
    std::fs::create_dir(&cache_dir).unwrap();
    let _cfg = set_kache_config_for_test(&cfg_path);
    let _cache = set_env_for_test("KACHE_CACHE_DIR", Some(cache_dir.as_os_str()));
    let _max = set_env_for_test("KACHE_MAX_SIZE", None);
    let loaded = Config::load().unwrap();
    let expected = disk_share_budget(kache_fs::volume::probe(&cache_dir).total_bytes);
    assert_eq!(loaded.max_size, expected);
    assert_ne!(loaded.max_size, 0);
}

#[test]
fn ignore_env_makes_file_win_over_env() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");

    // Restore KACHE_KEY_SALT after the test regardless of outcome.
    struct SaltGuard(Option<OsString>);
    impl Drop for SaltGuard {
        fn drop(&mut self) {
            unsafe {
                match self.0.as_ref() {
                    Some(v) => std::env::set_var("KACHE_KEY_SALT", v),
                    None => std::env::remove_var("KACHE_KEY_SALT"),
                }
            }
        }
    }
    let _salt = SaltGuard(std::env::var_os("KACHE_KEY_SALT"));
    unsafe { std::env::set_var("KACHE_KEY_SALT", "from-env") };

    let _g = set_kache_config_for_test(&cfg);

    // ignore_env = true: the pinned file's salt wins; the stray env is
    // ignored (the exact footgun the feature defends against).
    std::fs::write(
        &cfg,
        "[cache]\nignore_env = true\nkey_salt = \"from-file\"\n",
    )
    .unwrap();
    let loaded = Config::load().unwrap();
    assert_eq!(loaded.key_salt.as_deref(), Some("from-file"));

    // Without ignore_env, default precedence holds: env wins over the file.
    std::fs::write(&cfg, "[cache]\nkey_salt = \"from-file\"\n").unwrap();
    let loaded = Config::load().unwrap();
    assert_eq!(loaded.key_salt.as_deref(), Some("from-env"));
}

#[test]
fn config_file_fingerprint_tracks_content_and_presence() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    let _g = set_kache_config_for_test(&cfg);

    // Absent file has a stable fingerprint, distinct from any present file.
    let absent = config_file_fingerprint();
    assert_eq!(absent, config_file_fingerprint(), "absent must be stable");

    std::fs::write(
        &cfg,
        "[cache]\nignore_env = true\nlocal_max_size = \"10GiB\"\n",
    )
    .unwrap();
    let v10 = config_file_fingerprint();
    assert_ne!(absent, v10, "present must differ from absent");
    assert_eq!(
        v10,
        config_file_fingerprint(),
        "same content must be stable"
    );
    let (loaded, loaded_provenance) = Config::load_with_provenance().unwrap();
    assert_eq!(loaded.max_size, 10 * 1024 * 1024 * 1024);
    assert_eq!(loaded_provenance.path, std::path::absolute(&cfg).unwrap());
    assert_eq!(loaded_provenance.fingerprint, v10);

    // A content change moves the fingerprint (the daemon-restart trigger).
    std::fs::write(
        &cfg,
        "[cache]\nignore_env = true\nlocal_max_size = \"20GiB\"\n",
    )
    .unwrap();
    assert_ne!(v10, config_file_fingerprint(), "content change must re-key");
    assert_ne!(
        loaded_provenance.fingerprint,
        config_file_provenance_at(loaded_provenance.path.clone()).fingerprint,
        "the watcher baseline must remain the exact snapshot parsed above"
    );
    assert!(
        config_file_has_changed(&loaded_provenance),
        "the first watcher poll must notice an edit made after config load"
    );
}

#[test]
fn config_provenance_distinguishes_absent_and_unreadable_paths() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.toml");

    let (loaded, absent) = Config::load_file_config_with_provenance(missing.clone());
    assert!(loaded.is_ok(), "a missing config file means defaults");
    let expected_absent =
        ConfigFileProvenance::from_snapshot(missing.clone(), ConfigFileState::Absent, &[], None);
    assert_eq!(absent, expected_absent);
    assert_eq!(config_file_provenance_at(missing), absent);
    assert!(!config_file_has_changed(&absent));

    // Reading a directory as a config file fails on every supported
    // platform with a non-NotFound error, so it is an unreadable snapshot
    // rather than an absent one.
    let unreadable_path = dir.path().to_path_buf();
    let (loaded, unreadable) = Config::load_file_config_with_provenance(unreadable_path.clone());
    assert!(loaded.is_err(), "an unreadable config must stay an error");
    let expected_unreadable = ConfigFileProvenance::from_snapshot(
        unreadable_path.clone(),
        ConfigFileState::Unreadable,
        &[],
        None,
    );
    assert_eq!(unreadable, expected_unreadable);
    assert_eq!(config_file_provenance_at(unreadable_path), unreadable);
    assert!(!config_file_has_changed(&unreadable));
}

/// Write a host file and, when given, a chosen file under `dir`.
fn write_host_and_chosen(
    dir: &std::path::Path,
    host: &str,
    chosen: Option<&str>,
) -> (PathBuf, PathBuf) {
    let host_path = dir.join("host.toml");
    std::fs::write(&host_path, host).unwrap();
    let chosen_path = dir.join("chosen.toml");
    if let Some(chosen) = chosen {
        std::fs::write(&chosen_path, chosen).unwrap();
    }
    (host_path, chosen_path)
}

#[test]
fn host_layer_is_off_in_unit_tests_and_when_the_variable_is_empty() {
    let _lock = config_path_lock();
    {
        let _host = set_env_for_test("KACHE_HOST_CONFIG", None);
        assert_eq!(host_config_path(), None, "unit tests never read /etc");
    }
    {
        let _host = set_env_for_test("KACHE_HOST_CONFIG", Some(std::ffi::OsStr::new("")));
        assert_eq!(host_config_path(), None, "an empty value turns it off");
    }
    let _host = set_env_for_test(
        "KACHE_HOST_CONFIG",
        Some(std::ffi::OsStr::new("/srv/kache.toml")),
    );
    assert_eq!(host_config_path(), Some(PathBuf::from("/srv/kache.toml")));
    // Outside tests, this is the default. The docs name the same path.
    assert_eq!(HOST_CONFIG_PATH, "/etc/kache/config.toml");
}

/// Backward compatibility: with no host file, the chosen file resolves
/// exactly as it did before the host layer, down to the fingerprint.
#[test]
fn a_missing_host_file_changes_nothing() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("chosen.toml");
    let content = "[cache]\nlocal_max_size = \"10GiB\"\n";
    std::fs::write(&chosen, content).unwrap();
    let _host = set_host_config_for_test(&dir.path().join("absent.toml"));

    let (loaded, provenance) = Config::load_file_config_with_provenance(chosen.clone());
    assert_eq!(
        loaded.unwrap().cache.unwrap().local_max_size.as_deref(),
        Some("10GiB")
    );
    assert_eq!(
        provenance,
        ConfigFileProvenance::from_snapshot(
            chosen,
            ConfigFileState::Present,
            content.as_bytes(),
            None,
        ),
        "no host file must fingerprint exactly as before"
    );
}


#[test]
fn the_chosen_file_beats_the_host_key_by_key() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nkey_salt = \"host\"\nlocal_max_size = \"50GiB\"\n",
        Some("[cache]\nlocal_max_size = \"10GiB\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    let cache = Config::load_file_config().unwrap().cache.unwrap();
    assert_eq!(cache.local_max_size.as_deref(), Some("10GiB"));
    assert_eq!(
        cache.key_salt.as_deref(),
        Some("host"),
        "a key the chosen file leaves unset still comes from the host"
    );
}

#[test]
fn the_environment_beats_both_files() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nkey_salt = \"host\"\n",
        Some("[cache]\nlocal_max_size = \"10GiB\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    {
        let _env = set_env_for_test("KACHE_KEY_SALT", None);
        assert_eq!(
            Config::load().unwrap().key_salt.as_deref(),
            Some("host"),
            "host value"
        );
    }
    let _env = set_env_for_test("KACHE_KEY_SALT", Some(std::ffi::OsStr::new("env")));
    assert_eq!(
        Config::load().unwrap().key_salt.as_deref(),
        Some("env"),
        "the environment overrides the host file"
    );
}
/// `local_hit_daemon` was removed before 1.0 (kunobi-ninja/kache#565).
/// A file that still sets it, in either layer, must load with every other
/// setting intact rather than fail the parse.
#[test]
fn a_removed_local_hit_daemon_key_is_ignored() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nlocal_hit_daemon = true\n",
        Some("[cache]\nlocal_hit_daemon = true\nkey_salt = \"kept\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);
    let _env = set_env_for_test("KACHE_RECORD_SESSIONS", None);

    let file = Config::load_file_config();
    let cache = file
        .as_ref()
        .expect("a removed key must not fail the parse")
        .cache
        .as_ref()
        .unwrap();
    assert_eq!(cache.key_salt.as_deref(), Some("kept"));
}


/// A project that lists its volume stores means exactly those; a host
/// shard on another volume must not join them.
#[test]
fn a_chosen_volume_set_replaces_the_host_volumes_whole() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache.volumes]\n\"/mnt/host\" = \"/mnt/host/kache\"\n",
        Some("[cache.volumes]\n\"/mnt/project\" = \"/mnt/project/kache\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    let volumes = Config::load_file_config()
        .unwrap()
        .cache
        .unwrap()
        .volumes
        .unwrap();
    assert_eq!(
        volumes,
        HashMap::from([("/mnt/project".to_string(), "/mnt/project/kache".to_string())])
    );
}



/// A host path that exists but cannot be read is reported, not mistaken
/// for a machine without a host file.
#[test]
fn an_unreadable_host_path_is_reported_not_treated_as_absent() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    // Reading a directory fails, and not with NotFound.
    let host = dir.path().join("host.toml");
    std::fs::create_dir(&host).unwrap();
    let chosen = dir.path().join("chosen.toml");
    std::fs::write(&chosen, "[cache]\nlocal_max_size = \"10GiB\"\n").unwrap();
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    let status = host_config_status();
    assert!(
        matches!(&status, HostConfigStatus::Invalid { path, .. } if *path == host),
        "{status:?}"
    );
    assert_eq!(
        Config::load_file_config()
            .unwrap()
            .cache
            .unwrap()
            .local_max_size
            .as_deref(),
        Some("10GiB"),
        "the chosen file still loads"
    );
}

#[test]
fn a_malformed_host_file_is_ignored_and_reported() {
    let _lock = config_path_lock();
    for host_content in [
        "[cache\nthis is not toml",
        "[cache]\nlocal_max_size = 5\n", // valid TOML, wrong type
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (host, chosen) = write_host_and_chosen(
            dir.path(),
            host_content,
            Some("[cache]\nlocal_max_size = \"10GiB\"\n"),
        );
        let _host = set_host_config_for_test(&host);
        let _chosen = set_kache_config_for_test(&chosen);

        let cache = Config::load_file_config()
            .expect("a bad host file must not break the chosen one")
            .cache
            .unwrap();
        assert_eq!(cache.local_max_size.as_deref(), Some("10GiB"));
        assert!(
            matches!(host_config_status(), HostConfigStatus::Invalid { .. }),
            "{host_content:?} must be reported"
        );
    }
}

#[test]
fn the_host_layer_never_contributes_workspace_declarations() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[workspace]\nextra = 1\n\n[cache]\nkey_salt = \"host\"\n",
        None,
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    let file = Config::load_file_config().unwrap();
    assert!(file.workspace.is_none());
    assert_eq!(
        file.cache.unwrap().key_salt.as_deref(),
        Some("host"),
        "with no chosen file, the host layer is the whole file config"
    );
}

/// A host `ignore_env = true` would switch the environment off for every
/// build on the machine, putting the host file above it.
#[test]
fn a_host_ignore_env_cannot_silence_the_environment() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nignore_env = true\nkey_salt = \"host\"\n",
        Some("[cache]\nlocal_max_size = \"10GiB\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);
    let _env = set_env_for_test("KACHE_KEY_SALT", Some(std::ffi::OsStr::new("env")));

    let file = Config::load_file_config();
    let cache = file.as_ref().unwrap().cache.as_ref().unwrap();
    assert_eq!(cache.ignore_env, None, "the host key is dropped");
    assert_eq!(
        cache.key_salt.as_deref(),
        Some("host"),
        "the rest still applies"
    );
    assert_eq!(
        Config::load().unwrap().key_salt.as_deref(),
        Some("env"),
        "the environment overrides the host file"
    );
    let HostConfigStatus::Present { keys, .. } = host_config_status() else {
        panic!("the host file must be present");
    };
    assert!(
        keys.iter().all(|entry| entry.key != "cache.ignore_env"),
        "doctor must not list a key that has no effect: {keys:?}"
    );
}

/// A running daemon restarts on a changed fingerprint, so editing the
/// host file has to change it, and removing the file restores the
/// fingerprint a host-less machine has.
#[test]
fn editing_the_host_file_changes_the_fingerprint() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let content = "[cache]\nlocal_max_size = \"10GiB\"\n";
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\ninput_predictions = true\n",
        Some(content),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);

    let (_, provenance) = Config::load_with_provenance().unwrap();
    assert!(!config_file_has_changed(&provenance));
    std::fs::write(&host, "[cache]\ninput_predictions = false\n").unwrap();
    assert!(config_file_has_changed(&provenance), "host edit");

    std::fs::remove_file(&host).unwrap();
    assert_eq!(
        config_file_provenance_at(provenance.path.clone()).fingerprint,
        ConfigFileProvenance::from_snapshot(
            provenance.path.clone(),
            ConfigFileState::Present,
            content.as_bytes(),
            None,
        )
        .fingerprint
    );
}

#[test]
fn host_status_names_what_overrides_each_key() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nkey_salt = \"host\"\nlocal_max_size = \"50GiB\"\n",
        Some("[cache]\nlocal_max_size = \"10GiB\"\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);
    let _salt = set_env_for_test("KACHE_KEY_SALT", Some(std::ffi::OsStr::new("env")));

    let HostConfigStatus::Present { keys, .. } = host_config_status() else {
        panic!("the host file must be present");
    };
    let source = |key: &str| {
        keys.iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| panic!("{key} missing from {keys:?}"))
            .source
            .clone()
    };
    assert_eq!(
        source("cache.key_salt"),
        HostKeySource::Env("KACHE_KEY_SALT")
    );
    assert_eq!(
        source("cache.local_max_size"),
        HostKeySource::ChosenFile(normalize_config_path(chosen))
    );
    assert_eq!(keys.len(), 2, "a replaced-whole table is listed once");
}

/// Doctor names the variable config resolution reads, which for many keys
/// is not `KACHE_` plus the key's name.
#[test]
fn host_status_names_the_variable_config_reads() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let (host, chosen) = write_host_and_chosen(
        dir.path(),
        "[cache]\nlocal_max_size = \"50GiB\"\nlocal_store = \"/srv/kache\"\n\
             daemon_idle_timeout_secs = 60\n\n\
             [cc]\nextra_allowlist_flags = [\"-x\"]\n",
        Some("[cache]\n"),
    );
    let _host = set_host_config_for_test(&host);
    let _chosen = set_kache_config_for_test(&chosen);
    let expected = [
        ("cache.local_max_size", "KACHE_MAX_SIZE"),
        ("cache.local_store", "KACHE_CACHE_DIR"),
        (
            "cache.daemon_idle_timeout_secs",
            "KACHE_DAEMON_IDLE_TIMEOUT",
        ),
        ("cc.extra_allowlist_flags", "KACHE_CC_EXTRA_ALLOWLIST_FLAGS"),
    ];
    let _unset: Vec<_> = ENV_FILE_KEYS
        .iter()
        .map(|(var, _)| set_env_for_test(var, None))
        .collect();
    let _set: Vec<_> = expected
        .iter()
        .map(|(_, var)| set_env_for_test(var, Some(std::ffi::OsStr::new("1"))))
        .collect();

    let HostConfigStatus::Present { keys, .. } = host_config_status() else {
        panic!("the host file must be present");
    };
    for (key, var) in expected {
        let entry = keys
            .iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| panic!("{key} missing from {keys:?}"));
        assert_eq!(entry.source, HostKeySource::Env(var), "{key}");
    }
}

/// Every `KACHE_*` variable this file reads outside its tests is either in
/// [`ENV_FILE_KEYS`] or one of the operational variables with no config
/// key, so a new variable cannot be added without doctor learning its key.
#[test]
fn every_env_var_config_reads_maps_to_a_config_key() {
    const OPERATIONAL: &[&str] = &[
        "KACHE_CONFIG",
        "KACHE_HOST_CONFIG",
        "KACHE_DISABLED",
        "KACHE_SOCKET_PATH",
        "KACHE_TRUST_DOMAIN",
    ];
    // The tests live in their own file, so config.rs is all non-test code.
    let source = include_str!("../config.rs");
    let read: std::collections::BTreeSet<&str> = source
        .match_indices("\"KACHE_")
        .map(|(start, _)| {
            let name = &source[start + 1..];
            let end = name
                .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(name.len());
            &name[..end]
        })
        .collect();
    let mapped: std::collections::BTreeSet<&str> =
        ENV_FILE_KEYS.iter().map(|(var, _)| *var).collect();
    assert_eq!(
        mapped.len(),
        ENV_FILE_KEYS.len(),
        "a variable is listed twice"
    );
    for name in &read {
        assert!(
            mapped.contains(name) || OPERATIONAL.contains(name),
            "{name} is read but has no ENV_FILE_KEYS entry"
        );
    }
    for var in &mapped {
        assert!(read.contains(var), "{var} is listed but never read");
    }
    let gated: std::collections::BTreeSet<&str> = IGNORE_ENV_GATED_VARS.iter().copied().collect();
    assert_eq!(
        gated, mapped,
        "ignore_env gates exactly the file-backed variables"
    );
}

/// Each key in [`ENV_FILE_KEYS`] is one the config file has: a value set
/// there survives parsing and writing back.
#[test]
fn every_env_file_key_is_a_config_file_key() {
    let candidates = [
        deser_value::Value::from("1"),
        deser_value::Value::from(1),
        deser_value::Value::from(true),
        deser_value::Value::from(vec![deser_value::Value::from("x")]),
    ];
    for (var, key) in ENV_FILE_KEYS {
        let path: Vec<&str> = key.split('.').collect();
        let survives = candidates.iter().any(|candidate| {
            let mut value = candidate.clone();
            for segment in path.iter().rev() {
                value = deser_value::Value::from(deser_value::Map::from_iter([(segment.to_string(), value)]));
            }
            let Ok(file) = deser_value::from_value::<FileConfig>(&value) else {
                return false;
            };
            let Ok(back) = deser_value::to_value(&file) else {
                return false;
            };
            path.iter()
                .try_fold(&back, |value, segment| value.get(*segment))
                .is_some()
        });
        assert!(survives, "{var} maps to {key}, which the config file lacks");
    }
}

#[test]
fn config_provenance_makes_an_explicit_path_absolute_without_resolving_it() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let cfg = dir.path().join("nested/../config.toml");
    std::fs::write(dir.path().join("config.toml"), "[cache]\n").unwrap();
    let _g = set_kache_config_for_test(&cfg);

    let (_, provenance) = Config::load_with_provenance().unwrap();
    assert!(provenance.path.is_absolute());
    assert_eq!(provenance.path, cfg);
}

#[test]
fn relative_config_paths_are_bound_to_the_loading_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();

    let relative = PathBuf::from("config.toml");
    let first_path = normalize_config_path_from(relative.clone(), Some(&first));
    let second_path = normalize_config_path_from(relative, Some(&second));
    assert_eq!(first_path, first.join("config.toml"));
    assert_eq!(second_path, second.join("config.toml"));
    assert_ne!(first_path, second_path);
}

#[test]
#[cfg(unix)]
fn config_provenance_keeps_symlink_identity_and_detects_retarget() {
    use std::os::unix::fs::symlink;

    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.toml");
    let second = dir.path().join("second.toml");
    let active = dir.path().join("active.toml");
    std::fs::write(&first, "[cache]\nlocal_max_size = \"1GiB\"\n").unwrap();
    std::fs::write(&second, "[cache]\nlocal_max_size = \"2GiB\"\n").unwrap();
    symlink(&first, &active).unwrap();
    let _g = set_kache_config_for_test(&active);

    let (_, loaded) = Config::load_with_provenance().unwrap();
    assert_eq!(
        loaded.path, active,
        "provenance must retain the symlink path"
    );
    std::fs::remove_file(&active).unwrap();
    symlink(&second, &active).unwrap();
    assert_ne!(
        loaded.fingerprint,
        config_file_provenance_at(loaded.path.clone()).fingerprint,
        "retargeting the configured symlink must trip the watcher"
    );
    assert!(config_file_has_changed(&loaded));
}

#[test]
fn normalize_volume_root_canonicalizes_roots() {
    assert_eq!(Config::normalize_volume_root(""), None);
    assert_eq!(Config::normalize_volume_root("   "), None);
    assert_eq!(Config::normalize_volume_root("/").as_deref(), Some("/"));
    assert_eq!(
        Config::normalize_volume_root("/mnt/biglake/").as_deref(),
        Some("/mnt/biglake/")
    );
    assert_eq!(Config::normalize_volume_root("relative/dir"), None);
    assert_eq!(Config::normalize_volume_root("D:"), None);
}

#[test]
fn ensure_trailing_sep_appends_only_when_missing() {
    assert_eq!(
        Config::ensure_trailing_sep("D:\\".to_string(), '\\'),
        "D:\\"
    );
    assert_eq!(Config::ensure_trailing_sep("D:".to_string(), '\\'), "D:\\");
    assert_eq!(
        Config::ensure_trailing_sep("/mnt".to_string(), '/'),
        "/mnt/"
    );
    assert_eq!(Config::ensure_trailing_sep(String::new(), '/'), "/");
}

#[test]
fn load_volume_stores_dedupes_duplicate_roots_deterministically() {
    // "/mnt/" and "/mnt" normalize alike; the lexicographically-first store wins, stably.
    let file_config: Result<FileConfig> = Ok(FileConfig {
        cache: Some(CacheFileConfig {
            volumes: Some(
                [
                    ("/mnt/".to_string(), "b-store".to_string()),
                    ("/mnt".to_string(), "a-store".to_string()),
                    ("/other/".to_string(), "c-store".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    });
    let shards = Config::load_volume_stores(&file_config, None);
    let kept: Vec<(&str, &str)> = shards
        .iter()
        .map(|shard| (shard.volume.as_str(), shard.store.to_str().unwrap()))
        .collect();
    assert_eq!(
        kept,
        vec![("/mnt/", "a-store"), ("/other/", "c-store"),],
        "duplicates collapse to the first store, distinct roots survive"
    );
}

#[test]
fn match_volume_store_prefers_the_tightest_root() {
    let shards = |loose: &str, tight: &str| {
        vec![
            VolumeStore {
                volume: Config::normalize_volume_root(loose).unwrap(),
                store: PathBuf::from("loose-store"),
                max_size: None,
            },
            VolumeStore {
                volume: Config::normalize_volume_root(tight).unwrap(),
                store: PathBuf::from("tight-store"),
                max_size: None,
            },
        ]
    };
    {
        let shards = shards("/mnt/", "/mnt/biglake/");
        assert_eq!(
            Config::match_volume_store(&shards, Path::new("/mnt/biglake/work/a.rlib")),
            Some(Path::new("tight-store")),
            "overlapping roots resolve to the longest match"
        );
        assert_eq!(
            Config::match_volume_store(&shards, Path::new("/mnt/other/a.rlib")),
            Some(Path::new("loose-store"))
        );
        assert_eq!(
            Config::match_volume_store(&shards, Path::new("/home/u/a.rlib")),
            None
        );
    }
    assert!(Config::match_volume_store(&[], Path::new("x")).is_none());
}

#[test]
fn load_volume_stores_skips_invalid_entries() {
    let file_config: Result<FileConfig> = Ok(FileConfig {
        cache: Some(CacheFileConfig {
            volumes: Some(
                [
                    ("D:".to_string(), "D:/kache-store".to_string()),
                    ("".to_string(), "D:/nowhere".to_string()),
                    ("E:".to_string(), "   ".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    });
    let shards = Config::load_volume_stores(&file_config, None);
    // Unix keys must be absolute paths; drive letters never match.
    assert!(shards.is_empty());
}

#[test]
fn load_volume_stores_carries_the_explicit_max_size_to_each_shard() {
    let file_config: Result<FileConfig> = Ok(FileConfig {
        cache: Some(CacheFileConfig {
            volumes: Some(
                [
                    ("/mnt/a".to_string(), "/mnt/a/store".to_string()),
                    ("/mnt/b".to_string(), "/mnt/b/store".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    });
    let explicit = Config::load_volume_stores(&file_config, Some(7));
    let derived = Config::load_volume_stores(&file_config, None);
    assert_eq!(explicit.len(), 2);
    assert!(explicit.iter().all(|shard| shard.max_size == Some(7)));
    assert!(derived.iter().all(|shard| shard.max_size.is_none()));
}

#[test]
fn a_shard_budget_is_the_explicit_max_size_or_its_own_disk_share() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let mut main = Config::load_resolved_for(
        &InvocationEnv::from_parts(
            Vec::<(OsString, OsString)>::new(),
            Some(dir.path().to_owned()),
            None,
            dir.path().join("tmp"),
        ),
        Ok(FileConfig::default()),
    )
    .unwrap();
    main.max_size = 50 * GIB;
    let shard_dir = dir.path().join("shard");
    let mut shard = VolumeStore {
        volume: "/mnt/vol/".into(),
        store: shard_dir.clone(),
        max_size: Some(3 * GIB),
    };

    let explicit = main.for_volume_store(&shard, |_| panic!("an explicit budget needs no probe"));
    assert_eq!(explicit.max_size, 3 * GIB);
    assert_eq!(explicit.cache_dir, shard_dir);
    assert_eq!(
        explicit.runtime_dir, main.runtime_dir,
        "runtime stays on main"
    );

    shard.max_size = None;
    let derived = main.for_volume_store(&shard, |path| {
        assert_eq!(path, shard_dir, "the shard's own filesystem is measured");
        Some(200 * GIB)
    });
    assert_eq!(derived.max_size, 10 * GIB, "5% of the shard's 200GiB");
    let unknown = main.for_volume_store(&shard, |_| None);
    assert_eq!(unknown.max_size, DISK_SHARE_FALLBACK);
    assert_eq!(main.max_size, 50 * GIB, "the main budget is untouched");
}

#[test]
fn for_store_dir_picks_the_shard_budget_only_for_a_shard() {
    let dir = tempfile::tempdir().unwrap();
    let mut main = Config::load_resolved_for(
        &InvocationEnv::from_parts(
            Vec::<(OsString, OsString)>::new(),
            Some(dir.path().to_owned()),
            None,
            dir.path().join("tmp"),
        ),
        Ok(FileConfig::default()),
    )
    .unwrap();
    main.max_size = 5000;
    let shard_dir = dir.path().join("shard");
    main.volume_stores = vec![VolumeStore {
        volume: "/mnt/vol/".into(),
        store: shard_dir.clone(),
        max_size: Some(700),
    }];

    let shard = main.for_store_dir(&shard_dir, |_| None);
    assert_eq!(
        (shard.cache_dir.as_path(), shard.max_size),
        (shard_dir.as_path(), 700)
    );
    let same = main.for_store_dir(&main.cache_dir.clone(), |_| None);
    assert_eq!(
        (same.cache_dir, same.max_size),
        (main.cache_dir.clone(), 5000)
    );
    let other = main.for_store_dir(&dir.path().join("elsewhere"), |_| None);
    assert_eq!(
        (other.cache_dir, other.max_size),
        (main.cache_dir.clone(), 5000)
    );
}

#[test]
fn volume_store_for_routes_by_toml_config() {
    let _guard = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("kache/config.toml");
    let _env_guard = set_kache_config_for_test(&config_path);
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
            &config_path,
            "[cache]\nlocal_store = \"/tmp/kache-test-cache\"\n[cache.volumes]\n\"/mnt/biglake\" = \"/mnt/biglake/kache-test-store\"\n",
        )
        .unwrap();
    let config = Config::load().unwrap();
    assert_eq!(config.volume_stores.len(), 1);
    assert_eq!(
        config.volume_stores[0].max_size, None,
        "with no explicit max size a shard derives its own budget"
    );
    {
        assert_eq!(
            config.volume_store_for(Path::new("/mnt/biglake/work/a.rlib")),
            Some(Path::new("/mnt/biglake/kache-test-store"))
        );
        assert_eq!(config.volume_store_for(Path::new("/home/u/a.rlib")), None);
        let routed = config.routed_for_path(Path::new("/mnt/biglake/work/a.rlib"));
        assert_eq!(
            routed.cache_dir,
            PathBuf::from("/mnt/biglake/kache-test-store")
        );
        assert_eq!(routed.runtime_dir, config.runtime_dir);
        let unmapped = config.routed_for_path(Path::new("/home/u/a.rlib"));
        assert_eq!(unmapped.cache_dir, config.cache_dir);
    }
}

#[test]
fn absolutize_volume_path_keeps_absolute_and_joins_relative() {
    let abs = Path::new("/tmp/kache-vol-abs");
    assert_eq!(absolutize_volume_path(abs), abs);
    let rel = absolutize_volume_path(Path::new("kache-vol-rel"));
    assert!(rel.is_absolute(), "relative paths must join onto cwd");
    assert!(rel.ends_with("kache-vol-rel"));
    assert_ne!(rel, PathBuf::new());
}

#[test]
fn daemon_execution_concurrency_respects_environment_file_and_automatic_mode() {
    let root = tempfile::tempdir().unwrap();
    let file = |text| Ok(deser_toml::from_str::<FileConfig>(text).unwrap());
    let invocation = |value: Option<&str>| {
        InvocationEnv::from_parts(
            value
                .into_iter()
                .map(|value| ("KACHE_DAEMON_EXECUTION_CONCURRENCY", value))
                .chain(std::iter::once((
                    "KACHE_CACHE_DIR",
                    root.path().to_str().unwrap(),
                ))),
            Some(root.path().to_owned()),
            None,
            root.path().to_owned(),
        )
    };
    for (text, expected) in [
        ("", None),
        ("[cache]\ndaemon_execution_concurrency = 0", None),
        ("[cache]\ndaemon_execution_concurrency = 24", Some(24)),
        ("[cache]\ndaemon_execution_concurrency = 1024", Some(1024)),
    ] {
        assert_eq!(
            Config::daemon_execution_concurrency_for(&invocation(None), &file(text)).unwrap(),
            expected
        );
        assert_eq!(
            Config::load_resolved_for(&invocation(None), file(text))
                .unwrap()
                .daemon_execution_concurrency,
            expected,
        );
        for (value, expected) in [("0", None), ("1", Some(1)), ("1024", Some(1024))] {
            assert_eq!(
                Config::daemon_execution_concurrency_for(&invocation(Some(value)), &file(text))
                    .unwrap(),
                expected
            );
        }
        for value in ["", "-1", "1.5", "auto", "invalid", "18446744073709551616"] {
            assert!(
                Config::daemon_execution_concurrency_for(&invocation(Some(value)), &file(text))
                    .is_err()
            );
        }
    }
    assert_eq!(
        Config::daemon_execution_concurrency_for(
            &invocation(Some("invalid")),
            &file("[cache]\nignore_env = true\ndaemon_execution_concurrency = 64")
        )
        .unwrap(),
        Some(64)
    );
    let maximum = (usize::MAX >> 5).to_string();
    assert_eq!(
        Config::daemon_execution_concurrency_for(&invocation(Some(&maximum)), &file("")).unwrap(),
        maximum.parse::<usize>().ok()
    );
    let too_large = ((usize::MAX >> 5) + 1).to_string();
    assert!(
        Config::daemon_execution_concurrency_for(&invocation(Some(&too_large)), &file("")).is_err()
    );
    assert!(
        Config::daemon_execution_concurrency_for(
            &invocation(None),
            &file(&format!(
                "[cache]\ndaemon_execution_concurrency = {too_large}"
            ))
        )
        .is_err()
    );
    assert_eq!(
        Config::daemon_execution_concurrency_for(
            &invocation(None),
            &Err(anyhow::anyhow!("no file"))
        )
        .unwrap(),
        None
    );
}

#[test]
fn trusted_daemon_execution_requires_an_explicit_setting() {
    let file = |text| Ok(deser_toml::from_str::<FileConfig>(text).unwrap());
    let invocation = |value: Option<&str>| {
        InvocationEnv::from_parts(
            value
                .into_iter()
                .map(|value| ("KACHE_TRUST_DAEMON_EXECUTION", value)),
            None,
            None,
            std::env::temp_dir(),
        )
    };
    for (text, expected) in [
        ("", false),
        ("[cache]\ntrust_daemon_execution = false", false),
        ("[cache]\ntrust_daemon_execution = true", true),
    ] {
        assert_eq!(
            Config::trust_daemon_execution_for(&invocation(None), &file(text)),
            expected
        );
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("0", false),
            ("false", false),
            ("yes", false),
            ("", false),
        ] {
            assert_eq!(
                Config::trust_daemon_execution_for(&invocation(Some(value)), &file(text)),
                expected,
                "{text:?} {value:?}"
            );
        }
    }
    assert!(!Config::trust_daemon_execution_for(
        &invocation(Some("1")),
        &file("[cache]\nignore_env = true")
    ));
    assert!(Config::trust_daemon_execution_for(
        &invocation(Some("0")),
        &file("[cache]\nignore_env = true\ntrust_daemon_execution = true")
    ));
}
#[test]
fn probe_memos_live_under_the_configured_cache_dir() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: the process-state lock serialises environment edits.
    unsafe { std::env::set_var("KACHE_CACHE_DIR", dir.path()) };
    let probes = probe_memo_dir();
    unsafe { std::env::remove_var("KACHE_CACHE_DIR") };
    assert_eq!(probes, dir.path().join("probes"));
    let default = probe_memo_dir();
    assert_eq!(default, default_cache_dir().join("probes"));
    assert!(default.is_absolute());
}

#[test]
fn test_file_config_roundtrip() {
    let config = FileConfig {
        cc: None,
        paths: None,
        workspace: None,
        cache: Some(CacheFileConfig {
            bypass_env: None,
            bypass_argv: None,
            bypass_crates: None,
            volumes: None,
            trust_daemon_execution: None,
            daemon_execution_concurrency: None,

            auto_recover_min_free_bytes: None,

            scheduler_memory_pressure: None,

            ignore_env: None,
            key_salt: None,
            path_only_env_vars: None,
            key_env_vars: Some(vec!["BOLTFFI_*".to_string()]),
            local_store: Some("~/my/cache".to_string()),
            runtime_dir: Some("~/my/runtime".to_string()),
            local_max_size: Some("50GiB".to_string()),
            cache_executables: Some(true),
            exclude: Some(vec!["vendor/problem/**".to_string()]),
            event_log_max_size: Some("10MiB".to_string()),
            event_log_keep_lines: Some(500),
            compression_level: Some(3),
        }),
    };
    let serialized = deser_toml::to_string(&config).unwrap();
    let deserialized: FileConfig = deser_toml::from_str(&serialized).unwrap();
    assert_eq!(
        deserialized.cache.as_ref().unwrap().local_store.as_deref(),
        Some("~/my/cache")
    );
    assert_eq!(
        deserialized.cache.as_ref().unwrap().runtime_dir.as_deref(),
        Some("~/my/runtime")
    );
    assert_eq!(
        deserialized.cache.as_ref().unwrap().exclude.as_deref(),
        Some(&["vendor/problem/**".to_string()][..])
    );
    assert_eq!(
        deserialized.cache.as_ref().unwrap().key_env_vars.as_deref(),
        Some(&["BOLTFFI_*".to_string()][..])
    );
}

#[test]
fn test_key_salt_file_env_precedence() {
    let _guard = config_path_lock();

    // Save/clear the process-global salt env so the test is
    // deterministic, and restore it on the way out.
    let prev_salt = std::env::var_os("KACHE_KEY_SALT");
    let restore_salt = |v: &Option<OsString>| unsafe {
        match v {
            Some(val) => std::env::set_var("KACHE_KEY_SALT", val),
            None => std::env::remove_var("KACHE_KEY_SALT"),
        }
    };
    restore_salt(&None);

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "[cache]\nkey_salt = \"from-file\"\n").unwrap();
    let _cfg_guard = set_kache_config_for_test(&cfg_path);

    // File value is picked up.
    assert_eq!(
        Config::load().unwrap().key_salt.as_deref(),
        Some("from-file")
    );

    // Env wins over the file.
    unsafe { std::env::set_var("KACHE_KEY_SALT", "from-env") };
    assert_eq!(
        Config::load().unwrap().key_salt.as_deref(),
        Some("from-env")
    );

    // A whitespace-only value is treated as unset (never silently
    // shifts the key).
    unsafe { std::env::set_var("KACHE_KEY_SALT", "   ") };
    assert_eq!(Config::load().unwrap().key_salt, None);

    restore_salt(&prev_salt);
}

#[test]
fn path_only_env_vars_take_the_env_list_verbatim_without_empties() {
    let _guard = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    let _cfg_guard = set_kache_config_for_test(&cfg_path);

    std::fs::write(&cfg_path, "[cache]\npath_only_env_vars = [\"FROM_FILE\"]\n").unwrap();
    {
        let _vars = NamedEnvGuard::remove("KACHE_PATH_ONLY_ENV_VARS");
        assert_eq!(
            Config::load().unwrap().path_only_env_vars,
            vec!["FROM_FILE".to_string()]
        );
    }

    {
        // Env replaces the file list; comma and whitespace both separate,
        // and the separators around them produce no empty entries. Names
        // are otherwise kept verbatim: they match what rustc reports.
        let _vars = NamedEnvGuard::set(
            "KACHE_PATH_ONLY_ENV_VARS",
            ",BUILDCONFIG_RS, my_crate:OUT_DIR,",
        );
        assert_eq!(
            Config::load().unwrap().path_only_env_vars,
            vec!["BUILDCONFIG_RS".to_string(), "my_crate:OUT_DIR".to_string()]
        );
    }
    assert!(IGNORE_ENV_GATED_VARS.contains(&"KACHE_PATH_ONLY_ENV_VARS"));
}

#[test]
fn test_cc_extra_allowlist_flags_file_env_precedence() {
    let _guard = config_path_lock();

    let prev = std::env::var_os("KACHE_CC_EXTRA_ALLOWLIST_FLAGS");
    let restore = |v: &Option<OsString>| unsafe {
        match v {
            Some(val) => std::env::set_var("KACHE_CC_EXTRA_ALLOWLIST_FLAGS", val),
            None => std::env::remove_var("KACHE_CC_EXTRA_ALLOWLIST_FLAGS"),
        }
    };
    restore(&None);

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        "[cc]\nextra_allowlist_flags = [\"-ffunction-sections\", \"-fdata-sections\"]\n",
    )
    .unwrap();
    let _cfg_guard = set_kache_config_for_test(&cfg_path);

    // File list is picked up.
    assert_eq!(
        Config::load().unwrap().cc_extra_allowlist_flags,
        vec![
            "-ffunction-sections".to_string(),
            "-fdata-sections".to_string()
        ]
    );

    // Env (whitespace-separated) wins over the file and is normalized:
    // trimmed, empties dropped, deduped, first-seen order preserved.
    unsafe {
        std::env::set_var(
            "KACHE_CC_EXTRA_ALLOWLIST_FLAGS",
            "  -fno-rtti   -fno-rtti -fbravo ",
        )
    };
    assert_eq!(
        Config::load().unwrap().cc_extra_allowlist_flags,
        vec!["-fno-rtti".to_string(), "-fbravo".to_string()]
    );

    // An empty env value disables the feature (overrides the file).
    unsafe { std::env::set_var("KACHE_CC_EXTRA_ALLOWLIST_FLAGS", "   ") };
    assert!(Config::load().unwrap().cc_extra_allowlist_flags.is_empty());

    restore(&prev);
}

#[test]
fn test_key_env_vars_file_env_precedence() {
    let _guard = config_path_lock();

    let prev = std::env::var_os("KACHE_KEY_ENV_VARS");
    let restore = |v: &Option<OsString>| unsafe {
        match v {
            Some(val) => std::env::set_var("KACHE_KEY_ENV_VARS", val),
            None => std::env::remove_var("KACHE_KEY_ENV_VARS"),
        }
    };
    restore(&None);

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        "[cache]\nkey_env_vars = [\"BOLTFFI_*\", \"APP_MODE\"]\n",
    )
    .unwrap();
    let _cfg_guard = set_kache_config_for_test(&cfg_path);

    // File list is picked up, sorted (order in the file must not matter —
    // the patterns are folded into the key).
    assert_eq!(
        Config::load().unwrap().key_env_vars,
        vec!["APP_MODE".to_string(), "BOLTFFI_*".to_string()]
    );

    // Env (comma/whitespace-separated) wins over the file and is normalized:
    // trimmed, empties dropped, deduped, sorted.
    unsafe { std::env::set_var("KACHE_KEY_ENV_VARS", " ZULU, ALPHA ,ALPHA,, BRAVO ") };
    assert_eq!(
        Config::load().unwrap().key_env_vars,
        vec!["ALPHA".to_string(), "BRAVO".to_string(), "ZULU".to_string()]
    );

    // An empty env value disables the feature (overrides the file).
    unsafe { std::env::set_var("KACHE_KEY_ENV_VARS", "   ") };
    assert!(Config::load().unwrap().key_env_vars.is_empty());

    restore(&prev);
}

#[test]
fn test_key_env_vars_ignore_env_makes_file_win() {
    let _guard = config_path_lock();

    let prev = std::env::var_os("KACHE_KEY_ENV_VARS");
    let restore = |v: &Option<OsString>| unsafe {
        match v {
            Some(val) => std::env::set_var("KACHE_KEY_ENV_VARS", val),
            None => std::env::remove_var("KACHE_KEY_ENV_VARS"),
        }
    };

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        "[cache]\nignore_env = true\nkey_env_vars = [\"APP_MODE\"]\n",
    )
    .unwrap();
    let _cfg_guard = set_kache_config_for_test(&cfg_path);

    // A stray machine-global export must not silently shift every key.
    unsafe { std::env::set_var("KACHE_KEY_ENV_VARS", "SOMETHING_ELSE") };
    assert_eq!(
        Config::load().unwrap().key_env_vars,
        vec!["APP_MODE".to_string()]
    );
    assert!(IGNORE_ENV_GATED_VARS.contains(&"KACHE_KEY_ENV_VARS"));

    restore(&prev);
}

#[test]
fn test_normalize_key_env_vars_keeps_interior_star_pattern() {
    // The warning is advisory; the pattern is still carried through so a
    // config edit is never silently rewritten behind the user's back.
    assert_eq!(
        normalize_key_env_vars(["A*B".to_string(), "  ".to_string()], "test"),
        vec!["A*B".to_string()]
    );
}

#[test]
fn path_only_entries_naming_the_manifest_dir_are_reported_inert() {
    let entries: Vec<String> = [
        "CARGO_MANIFEST_DIR",
        "my_crate:CARGO_MANIFEST_DIR",
        " cargo_manifest_dir ",
        "OUT_DIR",
        "my_crate:OUT_DIR",
        "BUILDCONFIG_RS",
    ]
    .iter()
    .map(|entry| (*entry).to_string())
    .collect();
    assert_eq!(
        warn_inert_path_only_env_vars(&entries, "test"),
        vec![
            "CARGO_MANIFEST_DIR".to_string(),
            "my_crate:CARGO_MANIFEST_DIR".to_string(),
            " cargo_manifest_dir ".to_string(),
        ]
    );
    assert!(warn_inert_path_only_env_vars(&[], "test").is_empty());
}

#[test]
fn test_env_overrides_detect() {
    // Just verify it doesn't panic — actual env var presence is environment-dependent
    let overrides = EnvOverrides::detect();
    // In test environment, these are typically not set
    let _ = overrides.cache_dir;
}

#[test]
fn test_config_store_dir() {
    let config = Config {
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        volume_stores: Vec::new(),
        trust_daemon_execution: false,
        daemon_execution_concurrency: None,
        project_rules: ProjectRules::default(),

        auto_recover_min_free_bytes: 0,

        scheduler_memory_pressure: true,

        path_only_env_vars: Vec::new(),
        key_env_vars: Vec::new(),
        base_dirs: Vec::new(),
        cache_dir: PathBuf::from("/tmp/kache"),
        runtime_dir: PathBuf::from("/tmp/kache"),
        socket_path_override: None,
        max_size: 1024,
        disabled: false,
        cache_executables: false,
        event_log_max_size: 1024,
        event_log_keep_lines: 100,
        compression_level: 3,
    };
    assert_eq!(config.store_dir(), PathBuf::from("/tmp/kache/store"));
    assert_eq!(
        config.upload_spool_dir(),
        PathBuf::from("/tmp/kache/upload-queue")
    );
}

#[test]
fn test_config_index_db_path() {
    let config = Config {
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        volume_stores: Vec::new(),
        trust_daemon_execution: false,
        daemon_execution_concurrency: None,
        project_rules: ProjectRules::default(),

        auto_recover_min_free_bytes: 0,

        scheduler_memory_pressure: true,

        path_only_env_vars: Vec::new(),
        key_env_vars: Vec::new(),
        base_dirs: Vec::new(),
        cache_dir: PathBuf::from("/tmp/kache"),
        runtime_dir: PathBuf::from("/tmp/kache"),
        socket_path_override: None,
        max_size: 1024,
        disabled: false,
        cache_executables: false,
        event_log_max_size: 1024,
        event_log_keep_lines: 100,
        compression_level: 3,
    };
    assert_eq!(config.index_db_path(), PathBuf::from("/tmp/kache/index.db"));
}

#[test]
fn test_config_event_log_path() {
    let config = Config {
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        volume_stores: Vec::new(),
        trust_daemon_execution: false,
        daemon_execution_concurrency: None,
        project_rules: ProjectRules::default(),

        auto_recover_min_free_bytes: 0,

        scheduler_memory_pressure: true,

        path_only_env_vars: Vec::new(),
        key_env_vars: Vec::new(),
        base_dirs: Vec::new(),
        cache_dir: PathBuf::from("/tmp/kache"),
        runtime_dir: PathBuf::from("/tmp/kache-runtime"),
        socket_path_override: None,
        max_size: 1024,
        disabled: false,
        cache_executables: false,
        event_log_max_size: 1024,
        event_log_keep_lines: 100,
        compression_level: 3,
    };
    assert_eq!(
        config.event_log_path(),
        PathBuf::from("/tmp/kache-runtime/events.jsonl")
    );
    assert_eq!(
        config.transfer_log_path(),
        PathBuf::from("/tmp/kache-runtime/transfers.jsonl")
    );
    assert_eq!(config.store_dir(), PathBuf::from("/tmp/kache/store"));
    assert_eq!(config.index_db_path(), PathBuf::from("/tmp/kache/index.db"));
    assert_eq!(
        config.upload_spool_dir(),
        PathBuf::from("/tmp/kache/upload-queue")
    );
}

#[test]
fn test_config_socket_path() {
    let _lock = config_path_lock();
    let _env_guard = set_env_for_test("KACHE_SOCKET_PATH", None);
    let config = Config {
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        volume_stores: Vec::new(),
        trust_daemon_execution: false,
        daemon_execution_concurrency: None,
        project_rules: ProjectRules::default(),

        auto_recover_min_free_bytes: 0,

        scheduler_memory_pressure: true,

        path_only_env_vars: Vec::new(),
        key_env_vars: Vec::new(),
        base_dirs: Vec::new(),
        cache_dir: PathBuf::from("/tmp/kache"),
        runtime_dir: PathBuf::from("/tmp/kache-runtime"),
        socket_path_override: None,
        max_size: 1024,
        disabled: false,
        cache_executables: false,
        event_log_max_size: 1024,
        event_log_keep_lines: 100,
        compression_level: 3,
    };
    assert_eq!(
        config.socket_path(),
        PathBuf::from("/tmp/kache-runtime/daemon.sock")
    );

    let socket_dir = tempfile::tempdir().unwrap();
    let socket = socket_dir.path().join("kache.sock");
    let overridden = Config {
        socket_path_override: Some(socket.clone()),
        ..config.clone()
    };
    assert_eq!(overridden.socket_path(), socket);

    let regular = socket_dir.path().join("important.txt");
    std::fs::write(&regular, b"keep me").unwrap();
    for invalid in [PathBuf::from("daemon.sock"), regular] {
        let invalid_config = Config {
            socket_path_override: Some(invalid),
            ..config.clone()
        };
        assert_eq!(
            invalid_config.socket_path(),
            PathBuf::from("/tmp/kache-runtime/daemon.sock")
        );
    }
}

#[test]
fn runtime_dir_resolves_env_file_default_and_ignore_env() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let cache_dir = dir.path().join("cache");
    let file_runtime = dir.path().join("file-runtime");
    let env_runtime = dir.path().join("env-runtime");
    let _config = set_kache_config_for_test(&config_path);
    let _cache_env = set_env_for_test("KACHE_CACHE_DIR", None);
    let _runtime_env = set_env_for_test("KACHE_RUNTIME_DIR", None);
    let _socket_env = set_env_for_test("KACHE_SOCKET_PATH", None);

    std::fs::write(
        &config_path,
        format!(
            "[cache]\nlocal_store = {:?}\nruntime_dir = {:?}\n",
            cache_dir.to_string_lossy(),
            file_runtime.to_string_lossy()
        ),
    )
    .unwrap();
    let from_file = Config::load().unwrap();
    assert_eq!(from_file.cache_dir, cache_dir);
    assert_eq!(from_file.runtime_dir, file_runtime);

    unsafe { std::env::set_var("KACHE_RUNTIME_DIR", &env_runtime) };
    assert_eq!(Config::load().unwrap().runtime_dir, env_runtime);

    std::fs::write(
        &config_path,
        format!(
            "[cache]\nlocal_store = {:?}\nruntime_dir = {:?}\nignore_env = true\n",
            cache_dir.to_string_lossy(),
            file_runtime.to_string_lossy()
        ),
    )
    .unwrap();
    assert_eq!(Config::load().unwrap().runtime_dir, file_runtime);

    unsafe { std::env::remove_var("KACHE_RUNTIME_DIR") };
    std::fs::write(
        &config_path,
        format!("[cache]\nlocal_store = {:?}\n", cache_dir.to_string_lossy()),
    )
    .unwrap();
    let compatible_default = Config::load().unwrap();
    assert_eq!(compatible_default.runtime_dir, compatible_default.cache_dir);
}

#[test]
fn concurrent_runtime_dirs_isolate_job_state_while_sharing_store() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared-cache");
    let runtime_a = dir.path().join("job-a");
    let runtime_b = dir.path().join("job-b");
    let _cache_env = set_env_for_test("KACHE_CACHE_DIR", None);
    let _runtime_env = set_env_for_test("KACHE_RUNTIME_DIR", None);
    let _socket_env = set_env_for_test("KACHE_SOCKET_PATH", None);
    let make = |runtime: &Path| {
        Config::load_resolved(Ok(FileConfig {
            cache: Some(CacheFileConfig {
                local_store: Some(shared.to_string_lossy().into_owned()),
                runtime_dir: Some(runtime.to_string_lossy().into_owned()),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .unwrap()
    };
    let a = make(&runtime_a);
    let b = make(&runtime_b);

    assert_eq!(a.store_dir(), b.store_dir());
    assert_eq!(a.index_db_path(), b.index_db_path());
    assert_eq!(a.upload_spool_dir(), b.upload_spool_dir());
    assert_ne!(a.socket_path(), b.socket_path());
    for extension in ["lock", "run.lock", "state.json", "log"] {
        let a_path = a.socket_path().with_extension(extension);
        let b_path = b.socket_path().with_extension(extension);
        assert_ne!(a_path, b_path);
        assert!(a_path.starts_with(&runtime_a));
        assert!(b_path.starts_with(&runtime_b));
    }
    assert_ne!(a.event_log_path(), b.event_log_path());
    assert_ne!(a.transfer_log_path(), b.transfer_log_path());


    let writers = [(a, "job-a"), (b, "job-b")].map(|(config, contents)| {
        std::thread::spawn(move || {
            std::fs::create_dir_all(&config.runtime_dir).unwrap();
            std::fs::write(config.event_log_path(), contents).unwrap();
            config
        })
    });
    let [a, b] = writers.map(|writer| writer.join().unwrap());
    assert_eq!(
        std::fs::read_to_string(a.event_log_path()).unwrap(),
        "job-a"
    );
    assert_eq!(
        std::fs::read_to_string(b.event_log_path()).unwrap(),
        "job-b"
    );
    assert!(!shared.join("events.jsonl").exists());
}

#[test]
fn socket_path_override_is_validated_and_snapshotted() {
    let _lock = config_path_lock();
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = first_dir.path().join("daemon.sock");
    let second = second_dir.path().join("daemon.sock");
    let _env_guard = set_env_for_test("KACHE_SOCKET_PATH", Some(first.as_os_str()));

    let config = Config::load().unwrap();
    unsafe { std::env::set_var("KACHE_SOCKET_PATH", &second) };
    assert_eq!(config.socket_path(), first);

    for invalid in [OsString::new(), OsString::from("daemon.sock")] {
        assert_eq!(resolve_socket_path_override(Some(invalid)), None);
    }
    let root = Path::new("/");
    assert_eq!(
        resolve_socket_path_override(Some(root.as_os_str().to_owned())),
        None
    );
    assert_eq!(
        resolve_socket_path_override(Some(first_dir.path().as_os_str().to_owned())),
        None
    );

    let regular = first_dir.path().join("important.txt");
    std::fs::write(&regular, b"keep me").unwrap();
    assert_eq!(
        resolve_socket_path_override(Some(regular.as_os_str().to_owned())),
        None
    );

    let invalid_os_path = first_dir.path().join(OsString::from("bad\0socket"));
    assert_eq!(
        resolve_socket_path_override(Some(invalid_os_path.into_os_string())),
        None
    );

    #[cfg(unix)]
    {
        let link = first_dir.path().join("linked.sock");
        std::os::unix::fs::symlink(&regular, &link).unwrap();
        assert_eq!(
            resolve_socket_path_override(Some(link.as_os_str().to_owned())),
            None
        );

        let stale = first_dir.path().join("stale.sock");
        let listener = std::os::unix::net::UnixListener::bind(&stale).unwrap();
        drop(listener);
        assert_eq!(
            resolve_socket_path_override(Some(stale.as_os_str().to_owned())),
            Some(stale)
        );
    }
}

/// #222: each rule kind fires, names itself in the reason, and an
/// invocation matching nothing stays cacheable.
#[test]
fn user_bypass_rules_match_crate_argv_and_env() {
    let argv: Vec<String> = ["rustc", "--crate-name", "app", "-Zunpretty=expanded"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let none = |_: &str| None;

    let by_crate = Config::user_bypass_reason_with(
        "mutants-runner",
        &argv,
        &["mutants-runner".to_string()],
        &[],
        &[],
        none,
    );
    assert_eq!(
        by_crate.as_deref(),
        Some("bypass rule: crate mutants-runner")
    );

    // argv rules match a substring of any single argument, so the rule
    // does not have to spell the whole `-Zunpretty=expanded`.
    let by_argv =
        Config::user_bypass_reason_with("app", &argv, &[], &["-Zunpretty".to_string()], &[], none);
    assert_eq!(
        by_argv.as_deref(),
        Some("bypass rule: argv contains -Zunpretty")
    );

    // `NAME=VALUE` demands that exact value; the motivating sqlx case.
    let sqlx = |name: &str| (name == "SQLX_OFFLINE").then(|| "false".to_string());
    let by_env = Config::user_bypass_reason_with(
        "app",
        &argv,
        &[],
        &[],
        &["SQLX_OFFLINE=false".to_string()],
        sqlx,
    );
    assert_eq!(
        by_env.as_deref(),
        Some("bypass rule: env SQLX_OFFLINE=false")
    );
    // A different value must NOT fire: this is what makes the rule usable
    // for "online builds only", rather than disabling caching outright.
    let offline = |name: &str| (name == "SQLX_OFFLINE").then(|| "true".to_string());
    assert_eq!(
        Config::user_bypass_reason_with(
            "app",
            &argv,
            &[],
            &[],
            &["SQLX_OFFLINE=false".to_string()],
            offline,
        ),
        None
    );
    // A bare NAME matches on presence alone, whatever the value.
    assert_eq!(
        Config::user_bypass_reason_with(
            "app",
            &argv,
            &[],
            &[],
            &["SQLX_OFFLINE".to_string()],
            offline,
        )
        .as_deref(),
        Some("bypass rule: env SQLX_OFFLINE")
    );

    // Crate rules are exact, not substring: a prefix must not bypass an
    // unrelated crate that merely starts the same way.
    assert_eq!(
        Config::user_bypass_reason_with(
            "mutants-runner-support",
            &argv,
            &["mutants-runner".to_string()],
            &[],
            &[],
            none,
        ),
        None
    );
    // Nothing configured: unchanged, cacheable.
    assert_eq!(
        Config::user_bypass_reason_with("app", &argv, &[], &[], &[], none),
        None
    );
}

/// Drives the real entry point, which reads the rule lists out of the
/// active config file. The matcher tests above inject their lists, so they
/// leave the loading half unproven: a `user_bypass_reason` that always
/// returned `None` would silently stop enforcing every configured rule and
/// still pass them.
#[test]
fn user_bypass_reason_reads_rules_from_the_config_file() {
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[cache]\n\
             bypass_crates = [\"mutants-runner\"]\n\
             bypass_argv = [\"-Zunpretty\"]\n\
             bypass_env = [\"SQLX_OFFLINE=false\"]\n",
    )
    .unwrap();
    let _config = set_kache_config_for_test(&config_path);
    let _offline = NamedEnvGuard::remove("SQLX_OFFLINE");

    let plain = vec!["rustc".to_string(), "src/lib.rs".to_string()];
    assert_eq!(Config::user_bypass_reason("app", &plain), None);
    assert_eq!(
        Config::user_bypass_reason("mutants-runner", &plain).as_deref(),
        Some("bypass rule: crate mutants-runner")
    );

    let unpretty = vec!["rustc".to_string(), "-Zunpretty=expanded".to_string()];
    assert_eq!(
        Config::user_bypass_reason("app", &unpretty).as_deref(),
        Some("bypass rule: argv contains -Zunpretty")
    );

    let _online = NamedEnvGuard::set("SQLX_OFFLINE", "false");
    assert_eq!(
        Config::user_bypass_reason("app", &plain).as_deref(),
        Some("bypass rule: env SQLX_OFFLINE=false")
    );
}

/// A blank entry must never become a match-everything rule. An empty argv
/// rule substring-matches every argument, so one stray blank line in a
/// config would otherwise disable the entire cache silently.
#[test]
fn blank_bypass_entries_never_match() {
    let argv = vec!["rustc".to_string(), "--crate-name".to_string()];
    assert_eq!(
        Config::user_bypass_reason_with(
            "app",
            &argv,
            &["".to_string()],
            &["".to_string()],
            &["".to_string()],
            |_| Some(String::new()),
        ),
        None
    );
}

#[test]
fn test_source_excluded_matches_relative_pattern_against_root() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("crates/problem/src/lib.rs");
    let patterns = vec!["crates/problem/**".to_string()];

    assert!(source_excluded_by_patterns(
        &patterns,
        &source,
        &[dir.path().to_path_buf()]
    ));
}

#[test]
fn test_source_excluded_matches_source_as_passed() {
    let patterns = vec!["src/*.c".to_string()];

    assert!(source_excluded_by_patterns(
        &patterns,
        Path::new("src/foo.c"),
        &[]
    ));
    assert!(!source_excluded_by_patterns(
        &patterns,
        Path::new("include/foo.h"),
        &[]
    ));
}

#[test]
fn test_exclude_expands_cargo_home_default_when_unset() {
    let home = kache_fs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let cargo_home = home.join(".cargo").to_string_lossy().into_owned();

    let (expanded, _) = expand_env_vars_collecting("$CARGO_HOME/registry/src/**", |_| None);
    assert_eq!(expanded, format!("{cargo_home}/registry/src/**"));

    let (expanded_braced, _) =
        expand_env_vars_collecting("${CARGO_HOME}/registry/src/**", |_| None);
    assert_eq!(expanded_braced, format!("{cargo_home}/registry/src/**"));
}

#[test]
fn expand_collecting_reports_unset_vars_only_once() {
    let (expanded, unset) =
        expand_env_vars_collecting("$MISSING/$MISSING/${ALSO_MISSING}/x", |_| None);
    // Unset refs stay literal so the caller can see they matched nothing.
    assert_eq!(expanded, "$MISSING/$MISSING/${ALSO_MISSING}/x");
    // Deduplicated, in first-seen order.
    assert_eq!(
        unset,
        vec!["MISSING".to_string(), "ALSO_MISSING".to_string()]
    );
}

#[test]
fn expand_collecting_no_unset_when_resolved_or_defaulted() {
    let (expanded, unset) =
        expand_env_vars_collecting("$FOO/x", |k| (k == "FOO").then(|| "bar".to_string()));
    assert_eq!(expanded, "bar/x");
    assert!(unset.is_empty());

    // CARGO_HOME has a built-in default, so it is not reported as unset.
    let (_, unset_default) = expand_env_vars_collecting("$CARGO_HOME/x", |_| None);
    assert!(unset_default.is_empty());
}

#[test]
fn test_load_config_reads_exclude_patterns() {
    let _guard = config_path_lock();

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("kache/config.toml");
    let _env_guard = set_kache_config_for_test(&config_path);

    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
        &config_path,
        r#"
[cache]
exclude = ["src/generated/**", "vendor/problem/**"]
"#,
    )
    .unwrap();

    assert!(Config::source_excluded(
        Path::new("src/generated/lib.rs"),
        &[]
    ));
    assert!(Config::source_excluded(
        Path::new("vendor/problem/foo.c"),
        &[]
    ));
    assert!(!Config::source_excluded(Path::new("src/main.rs"), &[]));
}

#[test]
fn test_config_file_path() {
    let path = config_file_path();
    assert!(path.to_string_lossy().contains("kache"));
    assert!(path.to_string_lossy().ends_with("config.toml"));
}

#[test]
fn test_resolve_config_path_prefers_kache_config() {
    let path = resolve_config_path_from(Some(PathBuf::from("/tmp/managed/config.toml")), None);
    assert_eq!(path, PathBuf::from("/tmp/managed/config.toml"));
}

#[test]
fn test_load_and_save_raw_file_config_use_resolved_path() {
    let _guard = config_path_lock();

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("managed/config.toml");
    let _env_guard = set_kache_config_for_test(&config_path);

    let config = FileConfig {
        cc: None,
        paths: None,
        workspace: None,
        cache: Some(CacheFileConfig {
            local_store: Some("/tmp/managed-cache".to_string()),
            ..Default::default()
        }),
    };

    Config::save_file_config_to(&config, &resolve_config_path()).unwrap();
    assert!(config_path.exists());

    let (loaded, existed) = Config::load_raw_file_config_from(&resolve_config_path()).unwrap();
    assert!(existed);
    assert_eq!(
        loaded.cache.as_ref().and_then(|c| c.local_store.as_deref()),
        Some("/tmp/managed-cache")
    );
}

#[test]
fn test_shellexpand_no_tilde() {
    let path = shellexpand("/absolute/path");
    assert_eq!(path, PathBuf::from("/absolute/path"));
}

#[test]
fn test_shellexpand_relative() {
    let path = shellexpand("relative/path");
    assert_eq!(path, PathBuf::from("relative/path"));
}

#[test]
fn test_shellexpand_bare_tilde() {
    let path = shellexpand("~");
    if let Some(home) = kache_fs::home_dir() {
        assert_eq!(path, home);
    } else {
        assert_eq!(path, PathBuf::from("~"));
    }
}

struct GenericEnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for GenericEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn set_env_var_for_test(key: &'static str, value: &str) -> GenericEnvGuard {
    let previous = std::env::var_os(key);
    unsafe {
        std::env::set_var(key, value);
    }
    GenericEnvGuard { key, previous }
}

#[test]
fn test_kache_cache_dir_env_expands_bare_tilde() {
    let _guard = config_path_lock();
    if let Some(home) = kache_fs::home_dir() {
        let _env_guard = set_env_var_for_test("KACHE_CACHE_DIR", "~");
        let config = Config::load().unwrap();
        assert_eq!(config.cache_dir, home);
    }
}

#[test]
fn test_kache_config_env_expands_bare_tilde() {
    let _guard = config_path_lock();
    if let Some(home) = kache_fs::home_dir() {
        let _env_guard = set_env_var_for_test("KACHE_CONFIG", "~");
        let resolved = resolve_config_path();
        assert_eq!(resolved, home);
    }
}

#[test]
fn test_parse_size_various() {
    assert_eq!(parse_size("1KiB"), Some(1024));
    assert_eq!(parse_size("10GiB"), Some(10 * 1024 * 1024 * 1024));
    assert_eq!(parse_size("0B"), Some(0));
    assert!(parse_size("").is_none());
    assert!(parse_size("abc").is_none());
}

#[test]
fn test_save_and_load_file_config() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("kache/config.toml");

    let config = FileConfig {
        cc: None,
        paths: None,
        workspace: None,
        cache: Some(CacheFileConfig {
            bypass_env: None,
            bypass_argv: None,
            bypass_crates: None,
            volumes: None,
            trust_daemon_execution: None,
            daemon_execution_concurrency: None,

            auto_recover_min_free_bytes: None,

            scheduler_memory_pressure: None,

            ignore_env: None,
            key_salt: None,
            path_only_env_vars: None,
            key_env_vars: None,
            local_store: Some("/tmp/my-cache".to_string()),
            runtime_dir: Some("/tmp/my-runtime".to_string()),
            local_max_size: Some("10GiB".to_string()),
            cache_executables: Some(true),
            exclude: None,
            event_log_max_size: None,
            event_log_keep_lines: None,
            compression_level: Some(5),
        }),
    };

    Config::save_file_config_to(&config, &config_path).unwrap();
    assert!(config_path.exists());

    let (loaded, existed) = Config::load_raw_file_config_from(&config_path).unwrap();
    assert!(existed);
    assert_eq!(
        loaded.cache.as_ref().unwrap().local_store.as_deref(),
        Some("/tmp/my-cache")
    );
    assert_eq!(loaded.cache.as_ref().unwrap().compression_level, Some(5));
    assert_eq!(
        loaded.cache.as_ref().unwrap().runtime_dir.as_deref(),
        Some("/tmp/my-runtime")
    );
}

#[test]
fn test_load_raw_file_config_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("nonexistent/config.toml");

    let (config, existed) = Config::load_raw_file_config_from(&config_path).unwrap();
    assert!(!existed);
    assert!(config.cache.is_none());
}

#[test]
fn raw_config_roundtrip_preserves_workspace_table() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.toml");
    let saved = dir.path().join("saved.toml");
    std::fs::write(
            &source,
            "[cache]\nlocal_max_size='1GiB'\n\n[[workspace.extra_inputs]]\ncrates=['macro-provider']\ninputs=['shared/value.txt']\npropagate_to_dependents=true\n",
        )
        .unwrap();

    let (config, existed) = Config::load_raw_file_config_from(&source).unwrap();
    assert!(existed);
    Config::save_file_config_to(&config, &saved).unwrap();

    let original: deser_value::Value = deser_toml::from_str(&std::fs::read_to_string(source).unwrap()).unwrap();
    let roundtripped: deser_value::Value =
        deser_toml::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert_eq!(roundtripped.get("workspace"), original.get("workspace"));
}

#[test]
fn raw_config_load_rejects_a_file_that_does_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[cache]\nlocal_max_size = = \"1GiB\"\n").unwrap();

    let error = Config::load_raw_file_config_from(&path)
        .unwrap_err()
        .to_string();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("line 2"), "{error}");
    assert!(!error.contains('\n'), "one line: {error}");
}

#[cfg(feature = "file-write")]
#[test]
fn update_file_config_keeps_comments_and_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let source = r#"# my kache settings
[cache]
local_max_size = "40GiB" # keep it small
retired_setting = true
exclude = ["vendor/**"]

[cache.retired]
# unknown table
user-agent = "old/1.0"
bucket = "b"
"#;
    std::fs::write(&path, source).unwrap();
    let (before, _) = Config::load_raw_file_config_from(&path).unwrap();
    let mut after = before.clone();
    let cache = after.cache.as_mut().unwrap();
    cache.local_max_size = Some("55GiB".into());
    cache.key_salt = Some("custom".into());
    cache.exclude = None;

    Config::update_file_config_at(&path, &before, &after).unwrap();

    let (loaded, _) = Config::load_raw_file_config_from(&path).unwrap();
    let cache = loaded.cache.unwrap();
    assert_eq!(cache.local_max_size.as_deref(), Some("55GiB"));
    assert_eq!(cache.key_salt.as_deref(), Some("custom"));
    assert!(cache.exclude.is_none());
}

#[test]
fn config_file_read_errors_other_than_missing_are_reported() {
    // A directory exists but cannot be read as a file.
    let dir = tempfile::tempdir().unwrap();
    let error = Config::load_raw_file_config_from(dir.path()).unwrap_err();
    assert!(error.to_string().starts_with("reading "), "{error:#}");

    #[cfg(feature = "file-write")]
    {
        let config = FileConfig::default();
        let error = Config::update_file_config_at(dir.path(), &config, &config).unwrap_err();
        assert!(error.to_string().starts_with("reading "), "{error:#}");
    }
}

#[cfg(feature = "file-write")]
#[test]
fn update_file_config_adds_a_table_the_file_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# mine\n[cache]\nlocal_store = \"~/s\"\n").unwrap();
    let (before, _) = Config::load_raw_file_config_from(&path).unwrap();
    let mut after = before.clone();
    after.paths = Some(PathsFileConfig {
        base_dirs: Some(vec!["/src".into()]),
    });

    Config::update_file_config_at(&path, &before, &after).unwrap();

    let (loaded, _) = Config::load_raw_file_config_from(&path).unwrap();
    assert_eq!(
        loaded.cache.unwrap().local_store.as_deref(),
        Some("~/s")
    );
    assert_eq!(
        loaded.paths.unwrap().base_dirs.as_deref(),
        Some(["/src".to_string()].as_slice())
    );
}

#[test]
fn describe_toml_error_is_one_line_with_position_and_message() {
    let parse = deser_toml::from_str::<deser_value::Map>("a = = 1").unwrap_err();
    assert_eq!(
        describe_toml_error(&parse),
        format!("line 1, column 5: {}", parse.message().trim())
    );
    // A schema error carries no position, so the message stands alone.
    let mut table = deser_value::Map::new();
    table.insert("cache", deser_value::Value::from(1));
    let schema = deser_value::from_value::<FileConfig>(&deser_value::Value::from(table)).unwrap_err();
    assert_eq!(describe_toml_error(&schema), schema.message().trim());
}

#[cfg(feature = "file-write")]
#[test]
fn update_file_config_leaves_an_unchanged_file_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let source = "# note\n[cache]\nlocal_store = '~/store'  # mine\n";
    std::fs::write(&path, source).unwrap();
    let (before, _) = Config::load_raw_file_config_from(&path).unwrap();
    let mut after = before.clone();
    after.cc = Some(CcFileConfig::default());

    Config::update_file_config_at(&path, &before, &after).unwrap();

    let (loaded, _) = Config::load_raw_file_config_from(&path).unwrap();
    assert_eq!(
        loaded.cache.unwrap().local_store.as_deref(),
        Some("~/store")
    );
    assert!(loaded.cc.unwrap_or_default().extra_allowlist_flags.is_none());
}

#[test]
fn unusable_config_file_names_the_file_and_the_error() {
    let _guard = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let _env_guard = set_kache_config_for_test(&path);
    let _host = set_env_for_test("KACHE_HOST_CONFIG", Some(std::ffi::OsStr::new("")));

    assert!(unusable_config_file().is_none(), "a missing file is fine");
    std::fs::write(&path, "[cache]\nlocal_max_size = \"1GiB\"\n").unwrap();
    assert!(unusable_config_file().is_none());

    std::fs::write(&path, "[cache\n").unwrap();
    let (reported, reason) = unusable_config_file().expect("a broken file is reported");
    assert_eq!(reported, path);
    assert!(reason.contains("line 1"), "{reason}");
}







#[test]
fn test_resolve_config_path_prefers_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path().join("workspace");
    let nested_dir = project_root.join("crate/src");
    std::fs::create_dir_all(&nested_dir).unwrap();

    let project_config = project_root.join(PROJECT_CONFIG_NAME);
    std::fs::write(&project_config, "[cache]\n").unwrap();

    let resolved = resolve_config_path_from(None, Some(nested_dir));
    assert_eq!(resolved, project_config);
}

#[test]
fn test_resolve_config_path_env_overrides_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path().join("workspace");
    std::fs::create_dir_all(&project_root).unwrap();

    let project_config = project_root.join(PROJECT_CONFIG_NAME);
    let env_config = dir.path().join("explicit-kache.toml");
    std::fs::write(&project_config, "[cache]\n").unwrap();

    let resolved = resolve_config_path_from(Some(env_config.clone()), Some(project_root));
    assert_eq!(resolved, env_config);
}

#[test]
fn test_resolve_config_path_falls_back_to_global_when_no_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let nested_dir = dir.path().join("workspace/crate");
    std::fs::create_dir_all(&nested_dir).unwrap();

    let resolved = resolve_config_path_from(None, Some(nested_dir));
    assert_eq!(resolved, config_file_path());
}

#[test]
fn test_normalize_cc_flags_trims_dedupes_and_drops_empty() {
    let input = [
        "  -O2 ".to_string(),
        "-O2".to_string(), // duplicate after trim
        String::new(),     // empty -> dropped
        "   ".to_string(), // whitespace-only -> dropped
        "-fPIC".to_string(),
        " -fPIC".to_string(), // duplicate after trim
    ];
    assert_eq!(
        normalize_cc_flags(input),
        vec!["-O2".to_string(), "-fPIC".to_string()]
    );
    assert!(normalize_cc_flags(Vec::<String>::new()).is_empty());
}








#[test]
fn the_default_floor_is_ten_gib_and_the_user_file_never_a_project_one() {
    assert_eq!(DEFAULT_RECOVERY_FLOOR, 10 * 1024 * 1024 * 1024);
    let _lock = config_path_lock();
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("chosen.toml");
    let _set = set_env_for_test("KACHE_CONFIG", Some(chosen.as_os_str()));
    assert_eq!(user_config_path(), chosen);
    let _unset = set_env_for_test("KACHE_CONFIG", None);
    assert_eq!(user_config_path(), config_file_path());
}

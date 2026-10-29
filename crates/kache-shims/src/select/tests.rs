use super::*;
use crate::fs::fake::FakeFs;

fn env(exe: &str, path: &[&str]) -> Env {
    Env {
        exe: exe.into(),
        path: path.iter().map(PathBuf::from).collect(),
        home: Some("/home/me".into()),
        user: Some("me".into()),
        ..Env::default()
    }
}

struct Row {
    name: &'static str,
    fs: fn() -> FakeFs,
    exe: &'static str,
    path: &'static [&'static str],
    tweak: fn(&mut Env),
    want: &'static str,
    kind: Kind,
    stability: Stability,
    reason: &'static str,
}

fn plain_env(_: &mut Env) {}

/// One row per installer layout and PATH shape.
#[test]
fn each_layout_selects_the_expected_path() {
    use Kind::*;
    use Stability::*;
    const KEG: &str = "/opt/homebrew/Cellar/kache/0.20.0/bin/kache";
    const STORE: &str = "/nix/store/abc-kache-0.26.0/bin/kache";
    const MISE_EXE: &str = "/home/me/.local/share/mise/installs/kache/0.26.3/bin/kache";
    const MISE_BIN: &str = "/home/me/.local/share/mise/installs/kache/0.26.3/bin";
    let rows = [
        Row {
            name: "cargo install on PATH",
            fs: || FakeFs::new().exe("/home/me/.cargo/bin/kache"),
            exe: "/home/me/.cargo/bin/kache",
            path: &["/usr/bin", "/home/me/.cargo/bin"],
            tweak: plain_env,
            want: "/home/me/.cargo/bin/kache",
            kind: Other,
            stability: UserManaged,
            reason: "the first kache on PATH",
        },
        Row {
            name: "hardlink on PATH",
            fs: || {
                FakeFs::new()
                    .exe("/opt/tools/kache")
                    .same_file("/usr/local/bin/kache", "/opt/tools/kache")
            },
            exe: "/opt/tools/kache",
            path: &["/usr/local/bin"],
            tweak: plain_env,
            want: "/usr/local/bin/kache",
            kind: Other,
            stability: UserManaged,
            reason: "",
        },
        Row {
            name: "bind mount on PATH",
            fs: || {
                FakeFs::new()
                    .exe("/srv/tools/bin/kache")
                    .same_file("/mnt/tools/bin/kache", "/srv/tools/bin/kache")
            },
            exe: "/srv/tools/bin/kache",
            path: &["/mnt/tools/bin"],
            tweak: plain_env,
            want: "/mnt/tools/bin/kache",
            kind: Other,
            stability: UserManaged,
            reason: "",
        },
        Row {
            name: "a same file without the execute bit",
            fs: || FakeFs::new().plain("/opt/tools/kache"),
            exe: "/opt/tools/kache",
            path: &["/opt/tools"],
            tweak: plain_env,
            want: "/opt/tools/kache",
            kind: Other,
            stability: Versioned,
            reason: "no installer alias or PATH entry",
        },
        Row {
            name: "relative PATH entries",
            fs: || FakeFs::new().exe("/bin/kache"),
            exe: "/bin/kache",
            path: &["bin", "", ".", "./bin"],
            tweak: plain_env,
            want: "/bin/kache",
            kind: Other,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "Homebrew opt link",
            fs: || {
                FakeFs::new()
                    .exe(KEG)
                    .link("/opt/homebrew/opt/kache", "../Cellar/kache/0.20.0")
                    .link(
                        "/opt/homebrew/bin/kache",
                        "../Cellar/kache/0.20.0/bin/kache",
                    )
            },
            exe: KEG,
            path: &["/opt/homebrew/bin"],
            tweak: plain_env,
            want: "/opt/homebrew/opt/kache/bin/kache",
            kind: Homebrew,
            stability: InstallerManaged,
            reason: "Homebrew's opt link for kache",
        },
        Row {
            name: "Homebrew run through its bin link (macOS current_exe)",
            fs: || {
                FakeFs::new()
                    .exe(KEG)
                    .link("/opt/homebrew/opt/kache", "../Cellar/kache/0.20.0")
                    .link(
                        "/opt/homebrew/bin/kache",
                        "../Cellar/kache/0.20.0/bin/kache",
                    )
            },
            exe: "/opt/homebrew/bin/kache",
            path: &[],
            tweak: plain_env,
            want: "/opt/homebrew/opt/kache/bin/kache",
            kind: Homebrew,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "Homebrew formula named differently from the binary",
            fs: || {
                FakeFs::new()
                    .exe("/opt/homebrew/Cellar/kache-unstable/0.28.0-rc.1/bin/kache")
                    .link(
                        "/opt/homebrew/opt/kache-unstable",
                        "../Cellar/kache-unstable/0.28.0-rc.1",
                    )
            },
            exe: "/opt/homebrew/Cellar/kache-unstable/0.28.0-rc.1/bin/kache",
            path: &[],
            tweak: plain_env,
            want: "/opt/homebrew/opt/kache-unstable/bin/kache",
            kind: Homebrew,
            stability: InstallerManaged,
            reason: "opt link for kache-unstable",
        },
        Row {
            name: "Homebrew old keg after opt moved on",
            fs: || {
                FakeFs::new()
                    .exe("/opt/homebrew/Cellar/kache/0.19.0/bin/kache")
                    .exe(KEG)
                    .link("/opt/homebrew/opt/kache", "../Cellar/kache/0.20.0")
            },
            exe: "/opt/homebrew/Cellar/kache/0.19.0/bin/kache",
            path: &["/opt/homebrew/Cellar/kache/0.19.0/bin"],
            tweak: plain_env,
            want: "/opt/homebrew/Cellar/kache/0.19.0/bin/kache",
            kind: Homebrew,
            stability: Versioned,
            reason: "Homebrew removes this version",
        },
        Row {
            name: "a PATH link into a keg is as pinned as the keg",
            fs: || {
                FakeFs::new()
                    .exe("/home/linuxbrew/.linuxbrew/Cellar/kache/0.20.0/bin/kache")
                    .link(
                        "/home/linuxbrew/.linuxbrew/bin/kache",
                        "../Cellar/kache/0.20.0/bin/kache",
                    )
            },
            exe: "/home/linuxbrew/.linuxbrew/Cellar/kache/0.20.0/bin/kache",
            path: &["/home/linuxbrew/.linuxbrew/bin"],
            tweak: plain_env,
            want: "/home/linuxbrew/.linuxbrew/Cellar/kache/0.20.0/bin/kache",
            kind: Homebrew,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "a symlinked PATH dir into a mise version",
            fs: || FakeFs::new().exe(MISE_EXE).link("/usr/local/bin", MISE_BIN),
            exe: MISE_EXE,
            path: &["/usr/local/bin"],
            tweak: plain_env,
            want: MISE_EXE,
            kind: Mise,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "a Homebrew opt copy is not an installer alias",
            fs: || {
                FakeFs::new()
                    .exe(KEG)
                    .same_file("/opt/homebrew/opt/kache/bin/kache", KEG)
            },
            exe: KEG,
            path: &[],
            tweak: plain_env,
            want: KEG,
            kind: Homebrew,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "MISE_INSTALLS_DIR",
            fs: || {
                FakeFs::new()
                    .exe("/mise-installs/kache/0.26.3/bin/kache")
                    .link("/mise-installs/kache/latest", "0.26.3")
            },
            exe: "/mise-installs/kache/0.26.3/bin/kache",
            path: &[],
            tweak: |env| env.mise_installs_dir = Some("/mise-installs".into()),
            want: "/mise-installs/kache/latest/bin/kache",
            kind: Mise,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "ASDF_DIR when ~/.asdf is absent",
            fs: || FakeFs::new().exe("/opt/asdf/installs/kache/1.0/bin/kache"),
            exe: "/opt/asdf/installs/kache/1.0/bin/kache",
            path: &["/opt/asdf/installs/kache/1.0/bin"],
            tweak: |env| env.asdf_dir = Some("/opt/asdf".into()),
            want: "/opt/asdf/installs/kache/1.0/bin/kache",
            kind: Asdf,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "a pinned Nix generation on PATH",
            fs: || {
                FakeFs::new().exe("/opt/tools/kache").same_file(
                    "/nix/var/nix/profiles/profile-7-link/bin/kache",
                    "/opt/tools/kache",
                )
            },
            exe: "/opt/tools/kache",
            path: &["/nix/var/nix/profiles/profile-7-link/bin"],
            tweak: plain_env,
            want: "/opt/tools/kache",
            kind: Other,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "root's Nix profile",
            fs: || {
                FakeFs::new().exe(STORE).link(
                    "/nix/var/nix/profiles/per-user/root/profile/bin/kache",
                    STORE,
                )
            },
            exe: STORE,
            path: &[],
            tweak: plain_env,
            want: "/nix/var/nix/profiles/per-user/root/profile/bin/kache",
            kind: Nix,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "Nix store binary with a matching user profile",
            fs: || {
                FakeFs::new()
                    .exe(STORE)
                    .link("/home/me/.nix-profile", "/nix/store/prof-user-environment")
                    .link("/nix/store/prof-user-environment/bin/kache", STORE)
            },
            exe: STORE,
            path: &["/nix/store/abc-kache-0.26.0/bin"],
            tweak: plain_env,
            want: "/home/me/.nix-profile/bin/kache",
            kind: Nix,
            stability: InstallerManaged,
            reason: "the Nix profile in /home/me/.nix-profile/bin",
        },
        Row {
            name: "Nix profile under XDG_STATE_HOME",
            fs: || {
                FakeFs::new()
                    .exe(STORE)
                    .link("/xdg/nix/profile/bin/kache", STORE)
            },
            exe: STORE,
            path: &[],
            tweak: |env| env.xdg_state_home = Some("/xdg".into()),
            want: "/xdg/nix/profile/bin/kache",
            kind: Nix,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "NixOS per-user profile",
            fs: || {
                FakeFs::new()
                    .exe(STORE)
                    .link("/etc/profiles/per-user/me/bin/kache", STORE)
            },
            exe: STORE,
            path: &[],
            tweak: plain_env,
            want: "/etc/profiles/per-user/me/bin/kache",
            kind: Nix,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "Nix store binary without a matching profile",
            fs: || {
                FakeFs::new()
                    .exe(STORE)
                    .exe("/nix/store/def-kache-0.27.0/bin/kache")
                    .link(
                        "/home/me/.nix-profile/bin/kache",
                        "/nix/store/def-kache-0.27.0/bin/kache",
                    )
            },
            exe: STORE,
            path: &[
                "/nix/store/abc-kache-0.26.0/bin",
                "/home/me/.nix-profile/bin",
            ],
            tweak: plain_env,
            want: STORE,
            kind: Nix,
            stability: Versioned,
            reason: "garbage collected",
        },
        Row {
            name: "mise activate mode puts the version dir on PATH",
            fs: || {
                FakeFs::new().exe(MISE_EXE).link(
                    "/home/me/.local/share/mise/installs/kache/latest",
                    "./0.26.3",
                )
            },
            exe: MISE_EXE,
            path: &[MISE_BIN],
            tweak: plain_env,
            want: "/home/me/.local/share/mise/installs/kache/latest/bin/kache",
            kind: Mise,
            stability: InstallerManaged,
            reason: "mise's latest alias for kache",
        },
        Row {
            name: "mise latest points at another version",
            fs: || {
                FakeFs::new()
                    .exe(MISE_EXE)
                    .exe("/home/me/.local/share/mise/installs/kache/0.27.0/bin/kache")
                    .link("/home/me/.local/share/mise/installs/kache/latest", "0.27.0")
            },
            exe: MISE_EXE,
            path: &[MISE_BIN],
            tweak: plain_env,
            want: MISE_EXE,
            kind: Mise,
            stability: Versioned,
            reason: "mise removes it",
        },
        Row {
            name: "mise dispatcher is skipped even when it is this file",
            fs: || {
                FakeFs::new().exe("/home/me/.cargo/bin/kache").same_file(
                    "/home/me/.local/share/mise/shims/kache",
                    "/home/me/.cargo/bin/kache",
                )
            },
            exe: "/home/me/.cargo/bin/kache",
            path: &["/home/me/.local/share/mise/shims", "/home/me/.cargo/bin"],
            tweak: plain_env,
            want: "/home/me/.cargo/bin/kache",
            kind: Other,
            stability: UserManaged,
            reason: "",
        },
        Row {
            name: "MISE_DATA_DIR",
            fs: || {
                FakeFs::new()
                    .exe("/tools/mise/installs/cargo-kache/0.26.3/bin/kache")
                    .link("/tools/mise/installs/cargo-kache/latest", "0.26.3")
            },
            exe: "/tools/mise/installs/cargo-kache/0.26.3/bin/kache",
            path: &[],
            tweak: |env| env.mise_data_dir = Some("/tools/mise".into()),
            want: "/tools/mise/installs/cargo-kache/latest/bin/kache",
            kind: Mise,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "XDG_DATA_HOME mise",
            fs: || {
                FakeFs::new()
                    .exe("/xdg/data/mise/installs/kache/1.0.0/kache")
                    .link("/xdg/data/mise/installs/kache/latest", "1.0.0")
            },
            exe: "/xdg/data/mise/installs/kache/1.0.0/kache",
            path: &[],
            tweak: |env| env.xdg_data_home = Some("/xdg/data".into()),
            want: "/xdg/data/mise/installs/kache/latest/kache",
            kind: Mise,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "mise data dir that is itself a symlink",
            fs: || {
                FakeFs::new()
                    .exe("/data/mise/installs/kache/0.26.3/bin/kache")
                    .link("/home/me/.local/share/mise", "/data/mise")
                    .link("/data/mise/installs/kache/latest", "0.26.3")
            },
            exe: "/data/mise/installs/kache/0.26.3/bin/kache",
            path: &[],
            tweak: plain_env,
            want: "/data/mise/installs/kache/latest/bin/kache",
            kind: Mise,
            stability: InstallerManaged,
            reason: "",
        },
        Row {
            name: "asdf keeps no alias",
            fs: || {
                let exe = "/home/me/.asdf/installs/kache/0.26.0/bin/kache";
                FakeFs::new()
                    .exe(exe)
                    .same_file("/home/me/.asdf/shims/kache", exe)
            },
            exe: "/home/me/.asdf/installs/kache/0.26.0/bin/kache",
            path: &[
                "/home/me/.asdf/shims",
                "/home/me/.asdf/installs/kache/0.26.0/bin",
            ],
            tweak: plain_env,
            want: "/home/me/.asdf/installs/kache/0.26.0/bin/kache",
            kind: Asdf,
            stability: Versioned,
            reason: "asdf keeps no alias",
        },
        Row {
            name: "ASDF_DATA_DIR",
            fs: || FakeFs::new().exe("/opt/asdf/installs/kache/1.0/bin/kache"),
            exe: "/opt/asdf/installs/kache/1.0/bin/kache",
            path: &[],
            tweak: |env| env.asdf_data_dir = Some("/opt/asdf".into()),
            want: "/opt/asdf/installs/kache/1.0/bin/kache",
            kind: Asdf,
            stability: Versioned,
            reason: "",
        },
        Row {
            name: "sudo is reported",
            fs: || {
                FakeFs::new()
                    .exe(KEG)
                    .link("/opt/homebrew/opt/kache", "../Cellar/kache/0.20.0")
            },
            exe: KEG,
            path: &[],
            tweak: |env| env.elevation = Some(Elevation::Sudo { user: "me".into() }),
            want: "/opt/homebrew/opt/kache/bin/kache",
            kind: Homebrew,
            stability: Unverified,
            reason: "running as root through sudo for me, so HOME and PATH may be root's; Homebrew",
        },
        Row {
            name: "a HOME owned by someone else is reported",
            fs: || FakeFs::new().exe("/usr/bin/kache"),
            exe: "/usr/bin/kache",
            path: &["/usr/bin"],
            tweak: |env| {
                env.elevation = Some(Elevation::ForeignHome {
                    home: "/home/other".into(),
                })
            },
            want: "/usr/bin/kache",
            kind: Other,
            stability: Unverified,
            reason: "HOME (/home/other) belongs to another user",
        },
        Row {
            name: "sudo keeps a versioned path versioned",
            fs: || FakeFs::new().exe("/opt/tools/kache"),
            exe: "/opt/tools/kache",
            path: &[],
            tweak: |env| env.elevation = Some(Elevation::Sudo { user: "me".into() }),
            want: "/opt/tools/kache",
            kind: Other,
            stability: Versioned,
            reason: "sudo",
        },
    ];
    for row in rows {
        let mut env = env(row.exe, row.path);
        (row.tweak)(&mut env);
        let got = select(&env, &(row.fs)()).expect(row.name);
        assert_eq!(got.path, Path::new(row.want), "{}: {got:?}", row.name);
        assert_eq!(
            (got.kind, got.stability),
            (row.kind, row.stability),
            "{}",
            row.name
        );
        assert!(got.reason.contains(row.reason), "{}: {got:?}", row.name);
    }
}


// Helpers and wiring the table and the property do not reach.

#[test]
fn keg_paths_split_into_opt_and_formula() {
    assert_eq!(
        homebrew_keg(Path::new(
            "/opt/homebrew/Cellar/kache/0.20.0/libexec/bin/kache"
        )),
        Some((
            PathBuf::from("/opt/homebrew/opt/kache"),
            PathBuf::from("libexec/bin/kache"),
            "kache".to_string()
        ))
    );
    for incomplete in [
        "/opt/homebrew/Cellar/kache/0.20.0",
        "/opt/homebrew/Cellar/kache",
        "/opt/homebrew/bin/kache",
    ] {
        assert_eq!(homebrew_keg(Path::new(incomplete)), None, "{incomplete}");
    }
}

#[test]
fn without_home_only_system_locations_are_known() {
    let mut env = env("/unused", &[]);
    env.home = None;
    env.user = None;
    assert_eq!(
        nix_profile_bins(&env),
        [
            PathBuf::from("/run/current-system/sw/bin"),
            PathBuf::from("/nix/var/nix/profiles/default/bin"),
            PathBuf::from("/nix/var/nix/profiles/per-user/root/profile/bin"),
        ]
    );
    let layout = Layout::new(&env, &FakeFs::new());
    assert_eq!(
        layout.versioned(Path::new("/nix/store/x/bin")),
        Some(Kind::Nix)
    );
    assert_eq!(
        layout.versioned(Path::new(
            "/home/me/.local/share/mise/installs/kache/1.0/bin"
        )),
        None
    );
    assert!(!layout.is_dispatcher(Path::new("/home/me/.asdf/shims")));
}

#[test]
fn elevation_is_sudo_as_root_or_a_home_owned_by_someone_else() {
    let home = Some(Path::new("/home/me"));
    let sudo = Some(Elevation::Sudo { user: "me".into() });
    let foreign = Some(Elevation::ForeignHome {
        home: "/home/me".into(),
    });
    let me = Some("me");
    let cases = [
        (0, Some("me"), Some("root"), home, Some(0), sudo.clone()),
        // `sudo -u bob` as me: not root, but someone else's environment.
        (1000, Some("me"), Some("bob"), home, Some(1000), sudo),
        // SUDO_USER equal to the current user is a leftover variable.
        (1000, Some("me"), me, home, Some(1000), None),
        (0, Some(""), me, home, Some(0), None),
        (0, None, me, home, Some(0), None),
        (0, None, me, home, Some(1000), foreign),
        (1000, None, me, home, None, None),
        (1000, None, me, None, Some(0), None),
    ];
    for (euid, sudo_user, user, home, owner, want) in cases {
        let got = elevation(euid, sudo_user.map(String::from), user, home, owner);
        assert_eq!(
            got, want,
            "euid={euid} sudo={sudo_user:?} user={user:?} owner={owner:?}"
        );
    }
}

#[test]
fn a_replaced_or_missing_binary_is_an_error() {
    let fs = FakeFs::new().exe("/usr/bin/kache").exe("/usr/bin/other");
    // The path now names a different file from the running image.
    let mut swapped = env("/usr/bin/kache", &[]);
    swapped.exe_identity = fs.identity(Path::new("/usr/bin/other"));
    let error = select(&swapped, &fs).unwrap_err();
    assert!(matches!(&error, Error::Replaced(p) if p == Path::new("/usr/bin/kache")));
    swapped.exe_identity = fs.identity(Path::new("/usr/bin/kache"));
    assert!(select(&swapped, &fs).is_ok());

    let replaced = select(&env("/usr/bin/kache (deleted)", &[]), &fs).unwrap_err();
    assert!(matches!(&replaced, Error::Replaced(p) if p == Path::new("/usr/bin/kache")));
    assert!(
        replaced
            .to_string()
            .contains("was replaced while this process ran")
    );

    let missing = select(&env("/usr/bin/kache", &[]), &FakeFs::new()).unwrap_err();
    assert_eq!(missing.to_string(), "/usr/bin/kache no longer exists");
    assert!(std::error::Error::source(&missing).is_none());

    let current = Error::CurrentExe(std::io::Error::other("no procfs"));
    assert!(current.to_string().contains("no procfs"));
    assert!(std::error::Error::source(&current).is_some());
}

#[test]
fn labels_name_each_kind_and_stability() {
    let kinds = [
        (Kind::Homebrew, "Homebrew"),
        (Kind::Nix, "Nix"),
        (Kind::Mise, "mise"),
        (Kind::Asdf, "asdf"),
        (Kind::Other, "standalone"),
    ];
    for (kind, label) in kinds {
        assert_eq!(kind.to_string(), label);
    }
    let stabilities = [
        (Stability::InstallerManaged, "installer-managed", true),
        (Stability::UserManaged, "user-managed", true),
        (Stability::Versioned, "versioned", false),
        (Stability::Unverified, "unverified", false),
    ];
    for (stability, label, survives) in stabilities {
        assert_eq!(stability.to_string(), label);
        assert_eq!(stability.survives_upgrade(), survives, "{label}");
    }
}

#[test]
fn live_wiring_reads_this_process() {
    let exe = std::env::current_exe().unwrap();
    let selection = detect().expect("the test binary exists");
    assert_eq!(
        kache_fs::file_identity(&selection.path).unwrap(),
        kache_fs::file_identity(&exe).unwrap()
    );

    let env = Env::from_process().unwrap();
    assert_eq!(env.exe, exe);
    assert_eq!(env.exe_identity, kache_fs::file_identity(&exe).ok());
    assert_eq!(running_identity(), env.exe_identity);
    let path = std::env::var_os("PATH").unwrap_or_default();
    assert_eq!(env.path, std::env::split_paths(&path).collect::<Vec<_>>());
    assert_eq!(env.home, std::env::var_os("HOME").map(PathBuf::from));

    if let Some(home) = &env.home {
        let var = |name| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let installs = var("MISE_INSTALLS_DIR").unwrap_or_else(|| {
            var("MISE_DATA_DIR")
                .or_else(|| var("XDG_DATA_HOME").map(|data| data.join("mise")))
                .unwrap_or_else(|| home.join(".local/share/mise"))
                .join("installs")
        });
        let version_dir = installs.join("kache/1.0/bin");
        assert_eq!(
            Layout::from_process().versioned(&version_dir),
            Some(Kind::Mise)
        );
    }

    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        assert_eq!(
            process_elevation(Some(Path::new("/")), env.user.as_deref()),
            Some(Elevation::ForeignHome { home: "/".into() })
        );
    }
}

#[test]
fn nix_generations_are_numbered_profile_links() {
    let generation = |name: &str| is_nix_generation(Component::Normal(name.as_ref()));
    for name in ["profile-12-link", "system-3-link"] {
        assert!(generation(name), "{name}");
    }
    for name in [
        "-12-link",
        "profile--link",
        "profile-ab-link",
        "profile-12",
        "link",
    ] {
        assert!(!generation(name), "{name}");
    }
}

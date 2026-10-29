//! File-backed configuration commands. Environment and host overrides stay separate.

use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct Scope {
    /// Use the nearest project config, or create .kache.toml in this directory
    pub(crate) project: bool,
    /// Use the user config, ignoring KACHE_CONFIG and project files
    pub(crate) global: bool,
    /// Use this configuration file
    pub(crate) file: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum Command {
    /// Read a value explicitly set in the selected file
    Get {
        scope: Scope,
        key: String,
    },
    /// List values explicitly set in the selected file
    List {
        scope: Scope,
    },
    /// Set a known configuration key; accepts TOML values or an unquoted string
    Set {
        scope: Scope,
        key: String,
        value: String,
    },
    /// Remove a value from the selected file
    Unset {
        scope: Scope,
        key: String,
    },
    /// Print the selected file's path, even if it does not exist
    Path {
        scope: Scope,
    },
}

impl Scope {
    fn path(&self) -> Result<PathBuf> {
        let cwd = std::env::current_dir().context("resolving configuration directory")?;
        let path = if let Some(path) = &self.file {
            path.clone()
        } else if self.global {
            crate::config::config_file_path()
        } else if self.project {
            cwd.ancestors()
                .map(|parent| parent.join(".kache.toml"))
                .find(|path| path.is_file())
                .unwrap_or_else(|| cwd.join(".kache.toml"))
        } else {
            crate::config::resolve_config_path()
        };
        Ok(cwd.join(path))
    }
}

pub(crate) fn run(command: &Command, json: bool) -> Result<()> {
    let scope = match command {
        Command::Get { scope, .. }
        | Command::List { scope }
        | Command::Set { scope, .. }
        | Command::Unset { scope, .. }
        | Command::Path { scope } => scope,
    };
    let path = scope.path()?;
    let body = match command {
        Command::Path { .. } => {
            if !json {
                println!("{}", path.display());
            }
            deser_value::value!({"path": path.display().to_string()})
        }
        Command::Get { key, .. } => {
            let keys = key_parts(key)?;
            let values = read_values(&path)?;
            let value = lookup(&values, &keys)
                .with_context(|| format!("{key} is not set in {}", path.display()))?;
            if !json {
                println!("{}", format_leaf(value));
            }
            deser_value::value!({"path": path.display().to_string(), "key": key, "value": value.clone()})
        }
        Command::List { .. } => {
            let values = read_values(&path)?;
            if !json {
                print_values("", &values);
            }
            deser_value::value!({"path": path.display().to_string(), "values": values})
        }
        Command::Set { key, value, .. } => {
            edit(&path, key, Some(value))?;
            if !json {
                println!("Set {key} in {}", path.display());
            }
            deser_value::value!({"path": path.display().to_string(), "key": key, "action": "set"})
        }
        Command::Unset { key, .. } => {
            edit(&path, key, None)?;
            if !json {
                println!("Unset {key} in {}", path.display());
            }
            deser_value::value!({"path": path.display().to_string(), "key": key, "action": "unset"})
        }
    };
    if json {
        crate::machine::emit("config", body, Vec::new())?;
    }
    Ok(())
}

fn key_parts(key: &str) -> Result<Vec<&str>> {
    let keys: Vec<_> = key.split('.').collect();
    if keys.iter().any(|part| {
        part.is_empty()
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }) {
        bail!("use a dotted key such as cache.explain_miss");
    }
    Ok(keys)
}

fn lookup<'a>(value: &'a deser_value::Value, keys: &[&str]) -> Option<&'a deser_value::Value> {
    keys.iter().try_fold(value, |value, key| value.get(*key))
}

fn read_values(path: &Path) -> Result<deser_value::Value> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(deser_value::Value::from(deser_value::Map::new()));
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(deser_value::Value::from(deser_value::Map::new()));
    }
    deser_toml::from_str(&text).with_context(|| format!("{} is not valid TOML", path.display()))
}

fn format_leaf(value: &deser_value::Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(flag) = value.as_bool() {
        return flag.to_string();
    }
    if let Some(number) = value.as_i64() {
        return number.to_string();
    }
    if let Some(number) = value.as_u64() {
        return number.to_string();
    }
    deser_json::to_string(value).unwrap_or_else(|_| value.name().to_string())
}

fn print_values(prefix: &str, value: &deser_value::Value) {
    if let Some(table) = value.as_map() {
        for (key, value) in table {
            let name = key.as_str().unwrap_or("");
            let key = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}.{name}")
            };
            print_values(&key, value);
        }
    } else {
        println!("{prefix} = {}", format_leaf(value));
    }
}

fn edit(path: &Path, key: &str, value: Option<&str>) -> Result<()> {
    let keys = key_parts(key)?;
    // Resolve existing symlinks before choosing a stable lock or replacing the file.
    let path = if fs::symlink_metadata(path).is_ok() {
        path.canonicalize()
            .context("resolving configuration file")?
    } else {
        path.to_owned()
    };
    let parent = path
        .parent()
        .context("configuration file needs a parent directory")?;
    fs::create_dir_all(parent).context("creating configuration directory")?;
    let mut lock_name = OsString::from(".");
    lock_name.push(
        path.file_name()
            .context("configuration file needs a name")?,
    );
    lock_name.push(".lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(parent.join(lock_name))
        .context("opening configuration lock")?;
    lock.lock().context("locking configuration file")?;
    let mut document = read_values(&path)?;
    change(&mut document, &keys, value)?;
    let contents = deser_toml::to_string(&document).context("serializing config")?;
    crate::atomic::atomic_write_and_replace(&path, false, |temp| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(temp)?.write_all(contents.as_bytes())?;
        Ok(())
    })?;
    Ok(())
}

fn parse_scalar(raw: &str) -> Result<deser_value::Value> {
    let wrapped = format!("v = {raw}\n");
    if let Ok(parsed) = deser_toml::from_str::<deser_value::Value>(&wrapped)
        && let Some(value) = parsed.get("v")
    {
        if value.is_map() {
            bail!("set a scalar or array key; edit configuration tables in TOML");
        }
        return Ok(value.clone());
    }
    Ok(deser_value::Value::from(raw))
}

fn change(document: &mut deser_value::Value, keys: &[&str], raw: Option<&str>) -> Result<()> {
    if raw.is_some() && keys.first() == Some(&"workspace") {
        bail!("edit workspace declarations directly in TOML");
    }
    let replacement = raw.map(parse_scalar).transpose()?;
    if !document.is_map() {
        *document = deser_value::Value::from(deser_value::Map::new());
    }
    assign(document, keys, keys, replacement)?;
    if raw.is_some() {
        let typed: crate::config::FileConfig =
            deser_value::from_value(document).context("invalid configuration value")?;
        let known = deser_value::to_value(&typed).context("serializing config")?;
        if lookup(&known, keys).is_none() {
            bail!("unknown configuration key {}", keys.join("."));
        }
    }
    Ok(())
}

fn assign(
    node: &mut deser_value::Value,
    keys: &[&str],
    full: &[&str],
    replacement: Option<deser_value::Value>,
) -> Result<()> {
    let Some((key, rest)) = keys.split_first() else {
        bail!("use a dotted key such as cache.explain_miss");
    };
    let table = node
        .as_map_mut()
        .context("a parent key is not a configuration table")?;
    if rest.is_empty() {
        if let Some(value) = replacement {
            table.insert(*key, value);
        } else if table.remove(*key).is_none() {
            bail!("{} is not set", full.join("."));
        }
        return Ok(());
    }
    if !table.contains_key(*key) {
        if replacement.is_none() {
            bail!("{} is not set", full.join("."));
        }
        table.insert(*key, deser_value::Map::new());
    }
    let child = table
        .get_mut(*key)
        .context("a parent key is not a configuration table")?;
    if !child.is_map() {
        bail!("a parent key is not a configuration table");
    }
    assign(child, rest, full, replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_preserve_comments_unknown_keys_and_value_types() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        fs::write(
            &path,
            "# Personal settings\n[cache]\nexplain_miss = false # Keep offline\nfuture_key = 9\n[workspace]\nextra_inputs = []\n",
        )
        .unwrap();
        edit(&path, "cache.explain_miss", Some("true")).unwrap();
        edit(&path, "cache.local_max_size", Some("50GiB")).unwrap();
        edit(&path, "paths.base_dirs", Some("['/a', '/b']")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let values: deser_value::Value = deser_toml::from_str(&text).unwrap();
        assert_eq!(values["cache"]["explain_miss"].as_bool(), Some(true));
        assert_eq!(values["cache"]["local_max_size"].as_str(), Some("50GiB"));
        assert_eq!(values["paths"]["base_dirs"][1].as_str(), Some("/b"));
        edit(&path, "cache.future_key", None).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("future_key"));
        edit(&path, "workspace.extra_inputs", None).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("extra_inputs"));
    }

    #[test]
    fn rejected_edits_leave_the_file_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let original = "[cache]\nexplain_miss = true\n";
        fs::write(&path, original).unwrap();
        for (key, value) in [
            ("cache.typo", Some("true")),
            ("workspace.typo", Some("true")),
            ("cache.explain_miss", Some("not-a-boolean")),
            ("cache.not_a_section.typo", Some("1")),
            ("cache", Some("{explain_miss = false}")),
            ("cache.explain_miss.child", Some("false")),
            ("cache..explain_miss", Some("false")),
            ("", Some("false")),
            ("cache.explain_miss\n", Some("false")),
            ("cache.absent", None),
            ("missing.value", None),
        ] {
            assert!(edit(&path, key, value).is_err(), "{key}");
            assert_eq!(fs::read_to_string(&path).unwrap(), original, "{key}");
        }
        fs::write(&path, "not valid TOML").unwrap();
        assert!(edit(&path, "cache.explain_miss", Some("true")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not valid TOML");
    }

    #[test]
    fn set_creates_a_config_and_unset_restores_absence() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/config.toml");
        assert!(edit(&path, "cache.explain_miss", None).is_err());
        assert!(!path.exists());
        edit(&path, "cache.explain_miss", Some("true")).unwrap();
        edit(&path, "cache.explain_miss", None).unwrap();
        let values: deser_value::Value = deser_toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(lookup(&values, &["cache", "explain_miss"]).is_none());
    }

    #[test]
    fn concurrent_edits_keep_both_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        for _ in 0..8 {
            fs::write(&path, "[cache]\ncache_executables = false\nexplain_miss = false\n").unwrap();
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                for key in ["cache.cache_executables", "cache.explain_miss"] {
                    let barrier = &barrier;
                    let path = &path;
                    scope.spawn(move || {
                        barrier.wait();
                        edit(path, key, Some("true")).unwrap();
                    });
                }
            });
            let values: deser_value::Value = deser_toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(values["cache"]["explain_miss"].as_bool(), Some(true));
            assert_eq!(values["cache"]["explain_miss"].as_bool(), Some(true));
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_private_permissions_survive_an_edit() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("dotfiles.toml");
        let link = root.path().join("config.toml");
        edit(&target, "cache.explain_miss", Some("false")).unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        edit(&link, "cache.explain_miss", Some("true")).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert!(fs::read_to_string(&target).unwrap().contains("true"));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_file(target).unwrap();
        assert!(edit(&link, "cache.explain_miss", Some("false")).is_err());
    }
}

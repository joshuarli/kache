use std::fs;
use std::path::Path;

fn repo_file(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

#[test]
fn pkg_url_extension_follows_pkg_fmt() {
    let manifest = repo_file("Cargo.toml");
    let pkg_url = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("pkg-url"))
        .expect("Cargo.toml should define [package.metadata.binstall] pkg-url");

    assert!(
        pkg_url.contains("{ archive-suffix }"),
        "pkg-url must end in {{ archive-suffix }} so the extension tracks each \
         target's pkg-fmt:\n  {pkg_url}"
    );
    for hardcoded in [".tar.gz", ".tgz", ".zip"] {
        assert!(
            !pkg_url.contains(hardcoded),
            "pkg-url must not hardcode `{hardcoded}`:\n  {pkg_url}"
        );
    }
}

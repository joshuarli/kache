/// Build version: CI sets KACHE_VERSION from the git tag, local builds use
/// Cargo.toml. Only the leading 'v' is stripped, so a prerelease suffix flows
/// through verbatim (release-candidates publish to crates.io, so the tag — and
/// thus --version — carries the full version, e.g. "v0.5.0-rc.4" -> "0.5.0-rc.4").
pub const VERSION: &str = strip_leading_v(match option_env!("KACHE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
});

const fn strip_leading_v(raw: &str) -> &str {
    if raw.len() > 1 && raw.as_bytes()[0] == b'v' {
        raw.split_at(1).1
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::strip_leading_v;

    #[test]
    fn only_one_leading_v_is_stripped_and_the_rest_flows_through() {
        for (raw, stripped) in [
            ("v0.5.0-rc.4", "0.5.0-rc.4"),
            ("0.5.0-rc.4", "0.5.0-rc.4"),
            ("vv1", "v1"),
            ("v", "v"),
            ("", ""),
        ] {
            assert_eq!(strip_leading_v(raw), stripped, "{raw:?}");
        }
    }
}

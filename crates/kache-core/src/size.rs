//! Human byte counts: parsing (`50GiB`, `512MiB`, `1000000`) and display
//! (`10.0 GiB`).
//!
//! Pure so config parsing and every human report share one reading of a
//! size. The accepted grammar mirrors the crate this replaces: a bare
//! integer is bytes, otherwise a decimal number with a case-insensitive
//! SI (`KB`, 1000-based) or binary (`KiB`, 1024-based) unit, optional
//! single-letter included. Anything else is `None`.

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * 1024 * 1024;
const TIB: u64 = 1024 * 1024 * 1024 * 1024;
const PIB: u64 = 1024 * 1024 * 1024 * 1024 * 1024;
const EIB: u64 = 1024 * 1024 * 1024 * 1024 * 1024 * 1024;

/// Parse a human size string to bytes. `None` for anything that is not a
/// bare integer or a number with a known unit.
pub fn parse_size(value: &str) -> Option<u64> {
    if let Ok(exact) = value.parse::<u64>() {
        return Some(exact);
    }
    let number: String = value.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let parsed: f64 = number.parse().ok()?;
    let suffix = value[number.len()..].trim_start();
    let factor = unit_factor(suffix)?;
    if number.contains('.') {
        Some((parsed * factor as f64) as u64)
    } else {
        match number.parse::<u64>() {
            Ok(whole) => Some(whole.saturating_mul(factor)),
            Err(_) => Some((parsed * factor as f64) as u64),
        }
    }
}

fn unit_factor(unit: &str) -> Option<u64> {
    Some(match () {
        _ if unit.eq_ignore_ascii_case("b") => 1,
        _ if unit.eq_ignore_ascii_case("k") || unit.eq_ignore_ascii_case("kb") => 1_000,
        _ if unit.eq_ignore_ascii_case("m") || unit.eq_ignore_ascii_case("mb") => 1_000_000,
        _ if unit.eq_ignore_ascii_case("g") || unit.eq_ignore_ascii_case("gb") => 1_000_000_000,
        _ if unit.eq_ignore_ascii_case("t") || unit.eq_ignore_ascii_case("tb") => 1_000_000_000_000,
        _ if unit.eq_ignore_ascii_case("p") || unit.eq_ignore_ascii_case("pb") => 1_000_000_000_000_000,
        _ if unit.eq_ignore_ascii_case("e") || unit.eq_ignore_ascii_case("eb") => {
            1_000_000_000_000_000_000
        }
        _ if unit.eq_ignore_ascii_case("ki") || unit.eq_ignore_ascii_case("kib") => KIB,
        _ if unit.eq_ignore_ascii_case("mi") || unit.eq_ignore_ascii_case("mib") => MIB,
        _ if unit.eq_ignore_ascii_case("gi") || unit.eq_ignore_ascii_case("gib") => GIB,
        _ if unit.eq_ignore_ascii_case("ti") || unit.eq_ignore_ascii_case("tib") => TIB,
        _ if unit.eq_ignore_ascii_case("pi") || unit.eq_ignore_ascii_case("pib") => PIB,
        _ if unit.eq_ignore_ascii_case("ei") || unit.eq_ignore_ascii_case("eib") => EIB,
        _ => return None,
    })
}

/// Format bytes the way reports print them: `512 B`, `1.0 KiB`,
/// `27.1 GiB`. One decimal, binary units, same rounding as the crate this
/// replaces (f64 division, `{:.1}`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < KIB {
        return format!("{bytes} B");
    }
    let size = bytes as f64;
    let mut exp: u32 = 1;
    while size / (KIB as f64).powi(exp as i32) >= KIB as f64 {
        exp += 1;
    }
    let prefix = b"KMGTPE"[exp as usize - 1] as char;
    format!("{:.1} {prefix}iB", size / (KIB as f64).powi(exp as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_integers_units_and_fractions() {
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("1000000"), Some(1_000_000));
        assert_eq!(parse_size("2GiB"), Some(2 << 30));
        assert_eq!(parse_size("50GiB"), Some(50 << 30));
        assert_eq!(parse_size("512MiB"), Some(512 << 20));
        assert_eq!(parse_size("3 MB"), Some(3_000_000));
        assert_eq!(parse_size("1K"), Some(1_000));
        assert_eq!(parse_size("1Ki"), Some(1 << 10));
        assert_eq!(parse_size("1.5KiB"), Some(1536));
        assert_eq!(parse_size("9007199254740993B"), Some(9_007_199_254_740_993));
        for bad in ["100 gigs", "1_000", "abc", "", "a124GB", "1 000 B"] {
            assert_eq!(parse_size(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn displays_one_decimal_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536 << 10), "1.5 MiB");
        assert_eq!(format_bytes(1 << 30), "1.0 GiB");
        assert_eq!(format_bytes(10 << 30), "10.0 GiB");
    }
}

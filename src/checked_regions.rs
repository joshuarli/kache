/// Return `(start, end)` after proving a file-offset range is representable,
/// does not overlap `minimum_offset` when non-empty, and lies within
/// `file_len`.
///
/// Empty regions carry no bytes, so their offset need not be at or after
/// `minimum_offset`; it must still be representable and within the file.
pub(crate) fn checked_file_region(
    file_len: usize,
    offset: u64,
    size: u64,
    minimum_offset: usize,
) -> Option<(usize, usize)> {
    let start = usize::try_from(offset).ok()?;
    let size = usize::try_from(size).ok()?;
    if size != 0 && start < minimum_offset {
        return None;
    }
    let end = start.checked_add(size)?;
    if end > file_len {
        return None;
    }
    Some((start, end))
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_file_region_enforces_bounds_and_empty_region_semantics() {
        assert_eq!(checked_file_region(32, 8, 8, 8), Some((8, 16)));
        assert_eq!(checked_file_region(32, 24, 8, 8), Some((24, 32)));
        assert_eq!(checked_file_region(32, 0, 0, 8), Some((0, 0)));
        assert_eq!(checked_file_region(32, 7, 1, 8), None);
        assert_eq!(checked_file_region(32, 25, 8, 8), None);
        assert_eq!(checked_file_region(32, u64::MAX, 1, 8), None);
    }
}

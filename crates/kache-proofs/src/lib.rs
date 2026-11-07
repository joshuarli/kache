//! Repository-only formal verification harnesses.

#[cfg(any(kani, test))]
mod checked_regions_under_proof {
    // Compile the production implementation itself while keeping the proof
    // harness out of both published crates and ordinary mutation discovery.
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/checked_regions.rs"
    ));
}

#[cfg(kani)]
mod kani_proofs {
    use super::checked_regions_under_proof::checked_file_region;

    #[kani::proof]
    fn checked_file_region_matches_its_mathematical_contract() {
        let file_len: usize = kani::any();
        let offset: u64 = kani::any();
        let size: u64 = kani::any();
        let minimum_offset: usize = kani::any();
        let exact_end = u128::from(offset) + u128::from(size);
        let valid = exact_end <= file_len as u128
            && (size == 0 || u128::from(offset) >= minimum_offset as u128);
        let region = checked_file_region(file_len, offset, size, minimum_offset);
        kani::cover!(valid && exact_end == file_len as u128);

        assert_eq!(region.is_some(), valid);
        if let Some((start, end)) = region {
            assert_eq!(start as u128, u128::from(offset));
            assert_eq!(end as u128, exact_end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::checked_regions_under_proof::checked_file_region;

    #[test]
    fn test_checked_file_region_bounds_and_empty() {
        assert_eq!(checked_file_region(32, 8, 8, 8), Some((8, 16)));
        assert_eq!(checked_file_region(32, 24, 8, 8), Some((24, 32)));
        assert_eq!(checked_file_region(32, 0, 0, 8), Some((0, 0)));
        assert_eq!(checked_file_region(32, 7, 1, 8), None);
        assert_eq!(checked_file_region(32, 25, 8, 8), None);
        assert_eq!(checked_file_region(32, u64::MAX, 1, 8), None);
    }
}

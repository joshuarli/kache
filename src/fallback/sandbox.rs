//! Policy decisions are independent of macOS process inspection and run in Linux mutation tests.

pub(super) fn permits_pipeline(peer: i32, daemon: i32, trusted: bool) -> bool {
    matches!(peer, 0 | 1) && matches!(daemon, 0 | 1) && (trusted || (peer == 0 && daemon == 0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn pipeline_trust_accepts_known_sandboxes_and_never_unknown_policy() {
        for peer in [-1, 0, 1, 2] {
            for daemon in [-1, 0, 1, 2] {
                assert_eq!(
                    super::permits_pipeline(peer, daemon, false),
                    peer == 0 && daemon == 0
                );
                assert_eq!(
                    super::permits_pipeline(peer, daemon, true),
                    [0, 1].contains(&peer) && [0, 1].contains(&daemon)
                );
            }
        }
    }
}

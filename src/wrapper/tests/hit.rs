use super::*;
use std::time::{Duration, Instant};

#[test]
fn report_preserves_phase_metrics() {
    let _lock = crate::test_support::process_state_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().join("cache"));
    let mut first = cached_file("first.o", "a");
    first.size = 13;
    let mut second = cached_file("second.o", "b");
    second.size = 29;
    let meta = entry_meta("stored-key", vec![first, second], &[]);
    HitCompletion {
        event_root: "/consumer",
        crate_name: "consumer.c",
        result: EventResult::PrefetchHit,
        cache_key: "requested-key",
        start: Instant::now() - Duration::from_millis(1500),
        key_ms: 41,
        key_hash_stats: FileHashStats {
            cache_hits: 3,
            cache_misses: 5,
            bytes_hashed: 73,
        },
        lookup_ms: 23,
        restore_ms: 19,
        key_record: KeyEventRecord::default(),
        object_output: "consumer.o".to_string(),
    }
    .report(&config, &meta);
    let events = events::read_events(&config.event_log_path()).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.root, "/consumer");
    assert_eq!(event.crate_name, "consumer.c");
    assert_eq!(event.result, EventResult::PrefetchHit);
    assert_eq!(event.cache_key, "requested-key");
    assert_eq!(event.size, 42);
    assert_eq!(event.compile_time_ms, 7);
    assert!(event.elapsed_ms >= 1500);
    assert_eq!(event.key_ms, 41);
    assert_eq!(event.key_hash_hits, 3);
    assert_eq!(event.key_hash_misses, 5);
    assert_eq!(event.key_hash_bytes, 73);
    assert_eq!(event.lookup_ms, 23);
    assert_eq!(event.restore_ms, 19);
    assert_eq!(event.store_ms, 0);
    assert_eq!(event.object_output, "consumer.o");
    assert_eq!(event.miss_reason, crate::events::MissReason::None);
}

//! Per-key demand observations: which cache keys one wrapper process looked
//! up, in order. Recorded in memory per invocation and persisted on the
//! wrapper's build event.

use deser::{Deserialize, Serialize};

/// One key requested by a wrapper, including unsuccessful predictions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KeyDemand {
    pub cache_key: String,
    pub first_demand_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_demand_round_trips_with_defaults() {
        let demand = KeyDemand {
            cache_key: "k1".into(),
            first_demand_at_ms: 1_050,
        };
        let json = deser_json::to_string(&demand).unwrap();
        assert_eq!(
            deser_json::from_str::<KeyDemand>(&json).unwrap(),
            demand
        );
        let legacy: KeyDemand = deser_json::from_str(
            r#"{"cache_key":"k","first_demand_at_ms":100,"remote_wait_ms":5}"#,
        )
        .unwrap();
        assert_eq!(legacy.first_demand_at_ms, 100);
    }
}

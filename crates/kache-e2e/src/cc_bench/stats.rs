//! The locked estimator: paired ratios summarised by their geometric mean,
//! with a two-sided 95% t-interval computed on the log ratios (df = n - 1)
//! and transformed back, so the interval is about the same quantity as the
//! point estimate.

use deser::Serialize;

/// Two-sided 95% Student t quantiles for df = 1..=9, the locked 5..=10 pairs.
const T_975: [f64; 9] = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RatioSummary {
    pub pairs: usize,
    pub geomean: f64,
    /// `None` below two pairs or above the table.
    pub ci95: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[deser(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}

/// Summarise `numerator[i] / denominator[i]` over the pairs.
pub fn paired_ratio(numerator: &[f64], denominator: &[f64]) -> Option<RatioSummary> {
    if numerator.len() != denominator.len() || numerator.is_empty() {
        return None;
    }
    let logs: Vec<f64> = numerator
        .iter()
        .zip(denominator)
        .map(|(a, b)| (a / b).ln())
        .collect::<Vec<_>>();
    if logs.iter().any(|l| !l.is_finite()) {
        return None;
    }
    let n = logs.len();
    let mean = logs.iter().sum::<f64>() / n as f64;
    let ci95 = (n >= 2).then(|| T_975.get(n - 2)).flatten().map(|t| {
        let var = logs.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        let half = t * (var / n as f64).sqrt();
        [(mean - half).exp(), (mean + half).exp()]
    });
    Some(RatioSummary { pairs: n, geomean: mean.exp(), ci95 })
}

/// The locked verdict rule against a "ratio <= target" objective.
pub fn verdict(summary: &RatioSummary, target: f64) -> Verdict {
    match summary.ci95 {
        Some([_, high]) if high <= target => Verdict::Pass,
        Some([low, _]) if low > target => Verdict::Fail,
        _ => Verdict::Inconclusive,
    }
}

/// Median of the samples, for absolute reporting.
pub fn median(samples: &[f64]) -> Option<f64> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    (n > 0).then(|| if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 })
}

/// splitmix64: the deterministic stream the locked seed drives.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Fisher-Yates.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = (self.next_u64() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_is_on_log_ratios_and_brackets_the_geomean() {
        let kache = [2.0, 4.0, 8.0];
        let ccache = [1.0, 1.0, 1.0];
        let s = paired_ratio(&kache, &ccache).unwrap();
        assert!((s.geomean - 4.0).abs() < 1e-9, "geomean of 2, 4, 8");
        let [low, high] = s.ci95.unwrap();
        // log ratios ln2, ln4, ln8: mean ln4, sd ln2, t(2) = 4.303.
        let half = 4.303 * 2f64.ln() / 3f64.sqrt();
        assert!((low - (4f64.ln() - half).exp()).abs() < 1e-9);
        assert!((high - (4f64.ln() + half).exp()).abs() < 1e-9);
        assert!((low * high - 16.0).abs() < 1e-6, "symmetric in log space around the geomean");
    }

    #[test]
    fn identical_ratios_have_a_degenerate_interval() {
        let s = paired_ratio(&[1.1; 5], &[1.0; 5]).unwrap();
        let [low, high] = s.ci95.unwrap();
        assert!((low - 1.1).abs() < 1e-9 && (high - 1.1).abs() < 1e-9);
        assert_eq!(verdict(&s, 1.15), Verdict::Pass);
        assert_eq!(verdict(&s, 1.05), Verdict::Fail);
    }

    #[test]
    fn an_interval_crossing_the_target_is_inconclusive() {
        let s = paired_ratio(&[1.0, 1.3, 1.1, 1.2, 1.05], &[1.0; 5]).unwrap();
        assert_eq!(verdict(&s, 1.15), Verdict::Inconclusive);
        assert_eq!(paired_ratio(&[1.0], &[1.0]).unwrap().ci95, None);
        assert_eq!(verdict(&paired_ratio(&[1.0], &[1.0]).unwrap(), 1.15), Verdict::Inconclusive);
        assert!(paired_ratio(&[1.0, 0.0], &[1.0, 1.0]).is_none());
        assert!(paired_ratio(&[1.0], &[1.0, 2.0]).is_none());
    }

    #[test]
    fn the_seeded_order_is_reproducible() {
        let order = |seed| {
            let mut arms = ["raw", "ccache", "kache"];
            Rng::new(seed).shuffle(&mut arms);
            arms
        };
        assert_eq!(order(20261117), order(20261117));
        let orders: std::collections::BTreeSet<_> = (0..20).map(order).collect();
        assert!(orders.len() > 1);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }
}

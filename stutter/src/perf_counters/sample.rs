use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuPerfDelta {
    pub cycles: Option<u64>,
    pub instructions: Option<u64>,
    pub cache_references: Option<u64>,
    pub cache_misses: Option<u64>,

    pub ipc: Option<f64>,
    pub cache_miss_rate: Option<f64>,
    pub cache_mpki: Option<f64>,

    pub time_enabled_ns: Option<u64>,
    pub time_running_ns: Option<u64>,
    pub multiplexed: bool,
    pub scaled: bool,

    pub unavailable_reason: Option<String>,
}

pub(super) fn scale(raw: u64, time_enabled: u64, time_running: u64) -> Option<u64> {
    if time_running == 0 {
        return None;
    }
    if time_running == time_enabled {
        return Some(raw);
    }

    Some(((raw as u128 * time_enabled as u128) / time_running as u128) as u64)
}

pub(super) fn apply_derived_metrics(delta: &mut CpuPerfDelta) {
    delta.ipc = ratio(delta.instructions, delta.cycles);
    delta.cache_miss_rate = ratio(delta.cache_misses, delta.cache_references);
    delta.cache_mpki = match (delta.cache_misses, delta.instructions) {
        (Some(misses), Some(instructions)) if instructions > 0 => {
            Some(misses as f64 * 1000.0 / instructions as f64).filter(|v| v.is_finite())
        }
        _ => None,
    };
}

fn ratio(numerator: Option<u64>, denominator: Option<u64>) -> Option<f64> {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator > 0 => {
            Some(numerator as f64 / denominator as f64).filter(|v| v.is_finite())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_perf_values() {
        assert_eq!(scale(100, 10, 10), Some(100));
        assert_eq!(scale(100, 20, 10), Some(200));
        assert_eq!(scale(100, 10, 0), None);
        assert_eq!(scale(u64::MAX, u64::MAX, u64::MAX), Some(u64::MAX));
    }

    #[test]
    fn derives_perf_ratios() {
        let mut delta = CpuPerfDelta {
            cycles: Some(100),
            instructions: Some(200),
            cache_references: Some(100),
            cache_misses: Some(10),
            ..Default::default()
        };

        apply_derived_metrics(&mut delta);

        assert_eq!(delta.ipc, Some(2.0));
        assert_eq!(delta.cache_miss_rate, Some(0.1));
        assert_eq!(delta.cache_mpki, Some(50.0));

        let mut zero = CpuPerfDelta {
            cycles: Some(0),
            instructions: Some(0),
            cache_references: Some(0),
            cache_misses: Some(10),
            ..Default::default()
        };
        apply_derived_metrics(&mut zero);
        assert_eq!(zero.ipc, None);
        assert_eq!(zero.cache_miss_rate, None);
        assert_eq!(zero.cache_mpki, None);
    }
}

pub(super) fn frame_count_low_reason(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
    low_ratio: f64,
) -> Option<String> {
    let (baseline, candidate) = frame_count_pair(baseline_frame_count, candidate_frame_count)?;
    let min = baseline.min(candidate);
    let max = baseline.max(candidate);

    if min == 0 && max > 0 {
        return Some(format!(
            "frame data mismatch when frame-based scoring is required: one window has frames and the other has none (baseline_frame_count={} candidate_frame_count={})",
            baseline, candidate
        ));
    }

    if min > 0 {
        let ratio = max as f64 / min as f64;
        if ratio > low_ratio {
            return Some(format!(
                "frame data mismatch when frame-based scoring is required: frame count ratio {:.2} exceeds {:.2} (baseline_frame_count={} candidate_frame_count={})",
                ratio, low_ratio, baseline, candidate
            ));
        }
    }

    None
}

pub(super) fn frame_count_medium_reason(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
    medium_ratio: f64,
) -> Option<String> {
    let (baseline, candidate) = frame_count_pair(baseline_frame_count, candidate_frame_count)?;
    let min = baseline.min(candidate);
    let max = baseline.max(candidate);

    if min > 0 {
        let ratio = max as f64 / min as f64;
        if ratio > medium_ratio {
            return Some(format!(
                "frame count differs by more than {:.0}% but remains below low-quality threshold (baseline_frame_count={} candidate_frame_count={} ratio={:.2})",
                (medium_ratio - 1.0) * 100.0,
                baseline,
                candidate,
                ratio
            ));
        }
    }

    None
}

fn frame_count_pair(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
) -> Option<(usize, usize)> {
    match (baseline_frame_count, candidate_frame_count) {
        (Some(baseline), Some(candidate)) => Some((baseline, candidate)),
        _ => None,
    }
}

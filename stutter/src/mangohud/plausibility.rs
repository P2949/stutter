const MAX_ABSOLUTE_FRAMETIME_MS: f64 = 5.0 * 60.0 * 1000.0;
const FRAMETIME_DELTA_MULTIPLIER: f64 = 2.0;
const FRAMETIME_DELTA_TOLERANCE_MS: f64 = 100.0;

#[derive(Default)]
pub(super) struct MangoHudFramePlausibilityFilter {
    previous_elapsed_ms: Option<u64>,
}

impl MangoHudFramePlausibilityFilter {
    pub(super) fn accept(&mut self, elapsed_ms: Option<u64>, frametime_ms: f64) -> bool {
        if !frametime_is_plausible(frametime_ms, self.previous_elapsed_ms, elapsed_ms) {
            return false;
        }

        if let Some(elapsed_ms) = elapsed_ms {
            self.previous_elapsed_ms = Some(elapsed_ms);
        }

        true
    }
}

fn frametime_is_plausible(
    frametime_ms: f64,
    previous_elapsed_ms: Option<u64>,
    current_elapsed_ms: Option<u64>,
) -> bool {
    if !frametime_ms.is_finite() || frametime_ms <= 0.0 || frametime_ms > MAX_ABSOLUTE_FRAMETIME_MS
    {
        return false;
    }

    let Some(current_elapsed_ms) = current_elapsed_ms else {
        return true;
    };

    let Some(previous_elapsed_ms) = previous_elapsed_ms else {
        return frametime_ms <= current_elapsed_ms as f64 + FRAMETIME_DELTA_TOLERANCE_MS;
    };

    if current_elapsed_ms <= previous_elapsed_ms {
        return frametime_ms <= FRAMETIME_DELTA_TOLERANCE_MS;
    }

    let elapsed_delta_ms = (current_elapsed_ms - previous_elapsed_ms) as f64;
    let max_by_multiplier = elapsed_delta_ms * FRAMETIME_DELTA_MULTIPLIER;
    let max_by_tolerance = elapsed_delta_ms + FRAMETIME_DELTA_TOLERANCE_MS;
    frametime_ms <= max_by_multiplier.max(max_by_tolerance)
}

use std::collections::{BTreeSet, VecDeque};

use crate::{process_tree::TaskClass, recorder::IntervalRecord};

pub(crate) fn overlap_basis_label<'a>(bases: impl Iterator<Item = &'a str>) -> Option<String> {
    let unique = bases
        .filter(|basis| !basis.trim().is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    if unique.is_empty() {
        return None;
    }

    Some(unique.into_iter().collect::<Vec<_>>().join("+"))
}

pub(crate) fn drain_front_before_elapsed<T, F>(
    items: &mut VecDeque<T>,
    start_ms: u64,
    elapsed_ms: F,
) where
    F: Fn(&T) -> u64,
{
    while items
        .front()
        .is_some_and(|item| elapsed_ms(item) < start_ms)
    {
        items.pop_front();
    }
}

pub(crate) fn push_sorted_by_elapsed<T, F>(items: &mut VecDeque<T>, item: T, elapsed_ms: F)
where
    F: Fn(&T) -> u64,
{
    let item_elapsed_ms = elapsed_ms(&item);

    match items
        .iter()
        .rposition(|existing| elapsed_ms(existing) <= item_elapsed_ms)
    {
        Some(index) => items.insert(index + 1, item),
        None => items.push_front(item),
    }
}

pub(crate) fn sort_intervals_by_elapsed(intervals: &mut VecDeque<IntervalRecord>) {
    let mut records = intervals.drain(..).collect::<Vec<_>>();
    records.sort_by_key(|record| record.elapsed_ms);
    *intervals = records.into();
}

pub(crate) fn is_valid_frametime_ms(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub(crate) fn is_compile_progress_record(record: &IntervalRecord) -> bool {
    record.samples > 0
        && matches!(
            record.class,
            TaskClass::BuildJob | TaskClass::Compiler | TaskClass::Linker
        )
}

pub(crate) fn percentile_f64(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let rank = ((values.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    values[rank.min(values.len() - 1)]
}

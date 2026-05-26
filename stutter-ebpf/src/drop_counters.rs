use crate::maps::DROP_COUNTERS;

#[inline(always)]
pub(crate) fn increment_drop_counter(reason: u32) {
    let Some(counter) = DROP_COUNTERS.get_ptr_mut(reason) else {
        return;
    };

    unsafe {
        *counter = (*counter).saturating_add(1);
    }
}

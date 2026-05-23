use crate::{WAKEUP_DATA, WakeupData};

#[inline(always)]
pub(crate) fn take_wakeup_data(pid: u32, out: &mut WakeupData) -> bool {
    // Aya does not expose lookup-and-delete here, so centralize the remaining
    // non-atomic window: copy the tiny value, then delete it immediately.
    if let Some(data) = unsafe { WAKEUP_DATA.get(pid) } {
        *out = *data;
        let _ = WAKEUP_DATA.remove(pid);
        true
    } else {
        false
    }
}

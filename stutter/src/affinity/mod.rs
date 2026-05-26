mod cpu_mask;
mod restore_file;
mod restore_record;
mod syscall;

#[cfg(test)]
mod tests;

pub use cpu_mask::CpuMask;
pub use restore_file::{default_restore_path, read_restore_records, restore_saved};
#[cfg(test)]
pub use restore_file::{load_restore_state, save_merged_restore_state, save_restore_state};
#[cfg(test)]
pub use restore_record::restore_all;
pub(crate) use restore_record::restore_identity_status_at;
pub use restore_record::{AffinityRecord, RestoreRecordStatus, RestoreState, RestoreSummary};
pub use syscall::{read_allowed_mask, set_affinity};
#[cfg(test)]
pub use syscall::{read_allowed_mask_raw, set_affinity_raw};

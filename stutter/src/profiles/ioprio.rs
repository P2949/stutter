use crate::actions::ioprio::IoPrioPolicy;

pub(super) fn profile_ioprio_policy() -> IoPrioPolicy {
    IoPrioPolicy {
        allow_ioprio_changes: true,
        allow_realtime_class: true,
        allow_none_class: true,
        max_best_effort_level: 7,
        require_strong_block_io_evidence: false,
        strong_block_io_evidence: true,
    }
}

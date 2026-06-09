//! Block I/O tracepoint offset validation.

use std::path::Path;

use stutter_common::tracepoint_offsets::{
    BLOCK_RQ_NR_SECTOR_MIN_SIZE, BLOCK_RQ_REQUEST_POINTER_MIN_SIZE,
    BLOCK_RQ_REQUIRED_METADATA_FIELDS, BLOCK_RQ_RWBS_MIN_SIZE,
};

use crate::ebpf::tracepoint_format::{
    TracepointFormat, parse_tracepoint_format_at, tracepoint_field_has_offset_and_size,
    validated_tracepoint_field_offset,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BlockIoTracepointOffsets {
    pub(crate) block_rq: bool,
    pub(crate) block_rq_has_rwbs: bool,
    pub(crate) block_rq_key_offset: Option<u32>,
    pub(crate) block_rq_issue_nr_sector_offset: Option<u32>,
    pub(crate) block_rq_issue_rwbs_offset: Option<u32>,
    pub(crate) block_rq_complete_nr_sector_offset: Option<u32>,
    pub(crate) block_rq_complete_rwbs_offset: Option<u32>,
}

pub(crate) fn validate_block_io_tracepoint_offsets(events_root: &Path) -> BlockIoTracepointOffsets {
    let issue_path = events_root.join("block/block_rq_issue/format");
    let complete_path = events_root.join("block/block_rq_complete/format");

    let issue = match parse_tracepoint_format_at(&issue_path) {
        Ok(format) => format,
        Err(err) => {
            log::warn!(
                "block_io_tracepoint_format_unavailable tracepoint=block_rq_issue path={} err={err:#}",
                issue_path.display()
            );
            return BlockIoTracepointOffsets::default();
        }
    };

    let complete = match parse_tracepoint_format_at(&complete_path) {
        Ok(format) => format,
        Err(err) => {
            log::warn!(
                "block_io_tracepoint_format_unavailable tracepoint=block_rq_complete path={} err={err:#}",
                complete_path.display()
            );
            return BlockIoTracepointOffsets::default();
        }
    };

    let issue_has_required_metadata = block_rq_has_required_metadata(&issue);
    let complete_has_required_metadata = block_rq_has_required_metadata(&complete);

    if !issue_has_required_metadata || !complete_has_required_metadata {
        log::warn!(
            "block_io_required_metadata_invalid issue_ok={} complete_ok={} fallback=disabled",
            issue_has_required_metadata,
            complete_has_required_metadata
        );
        return BlockIoTracepointOffsets::default();
    }

    let issue_rq_offset = validated_request_pointer_offset(&issue);
    let complete_rq_offset = validated_request_pointer_offset(&complete);

    let block_rq_key_offset = match (issue_rq_offset, complete_rq_offset) {
        (Some(issue_offset), Some(complete_offset)) if issue_offset == complete_offset => {
            Some(issue_offset)
        }
        (Some(issue_offset), Some(complete_offset)) => {
            log::warn!(
                "block_io_request_pointer_offset_mismatch issue_offset={} complete_offset={} fallback=dev_sector",
                issue_offset,
                complete_offset
            );
            None
        }
        _ => None,
    };

    let block_rq_issue_nr_sector_offset = validated_tracepoint_field_offset(
        &issue,
        "nr_sector",
        BLOCK_RQ_NR_SECTOR_MIN_SIZE,
        "u32 nr_sector",
    );
    let block_rq_complete_nr_sector_offset = validated_tracepoint_field_offset(
        &complete,
        "nr_sector",
        BLOCK_RQ_NR_SECTOR_MIN_SIZE,
        "u32 nr_sector",
    );

    let block_rq_issue_rwbs_offset =
        validated_tracepoint_field_offset(&issue, "rwbs", BLOCK_RQ_RWBS_MIN_SIZE, "u64 rwbs bytes");
    let block_rq_complete_rwbs_offset = validated_tracepoint_field_offset(
        &complete,
        "rwbs",
        BLOCK_RQ_RWBS_MIN_SIZE,
        "u64 rwbs bytes",
    );

    BlockIoTracepointOffsets {
        block_rq: true,
        block_rq_has_rwbs: block_rq_issue_rwbs_offset.is_some()
            && block_rq_complete_rwbs_offset.is_some(),
        block_rq_key_offset,
        block_rq_issue_nr_sector_offset,
        block_rq_issue_rwbs_offset,
        block_rq_complete_nr_sector_offset,
        block_rq_complete_rwbs_offset,
    }
}

fn validated_request_pointer_offset(format: &TracepointFormat) -> Option<u32> {
    for field_name in ["rq", "req", "request"] {
        if let Some(offset) = validated_tracepoint_field_offset(
            format,
            field_name,
            BLOCK_RQ_REQUEST_POINTER_MIN_SIZE,
            "u64 request pointer",
        ) {
            return Some(offset);
        }
    }

    None
}

fn block_rq_has_required_metadata(format: &TracepointFormat) -> bool {
    BLOCK_RQ_REQUIRED_METADATA_FIELDS
        .iter()
        .all(|(field_name, expected_offset, min_size)| {
            tracepoint_field_has_offset_and_size(
                format,
                field_name,
                *expected_offset as u32,
                *min_size,
            )
        })
}

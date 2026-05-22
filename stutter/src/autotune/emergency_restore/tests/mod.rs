
use super::manual_command::*;
use crate::actions::*;
use std::path::PathBuf;
mod support;
mod executors;
mod orchestration;

    #[test]
    fn manual_commands_cover_non_cpu_affinity_tokens() {
        let nice = manual_restore_command_for_token(&RollbackToken::NiceRestore {
            records: vec![NiceRestoreRecord::new(
                TaskRestoreIdentity::observed(7, None, Some("test".to_owned()), None, None),
                3,
            )],
        });
        assert_eq!(nice, "sudo renice -n 3 -p 7");

        let sysfs = manual_restore_command_for_token(&RollbackToken::SysfsRestore {
            path: PathBuf::from("/tmp/example knob"),
            original_value: "auto".to_owned(),
        });
        assert!(sysfs.contains("'/tmp/example knob'"));
        assert!(sysfs.contains("auto"));
    }

    #[test]
    fn action_id_helpers_extract_kind_and_candidate_name() {
        assert_eq!(
            action_kind_from_action_id("cpu-affinity-profile:game-main"),
            "cpu_affinity_profile"
        );
        assert_eq!(
            candidate_name_from_action_id("cpu-affinity-profile:game-main"),
            Some("game-main".to_owned())
        );
        assert_eq!(candidate_name_from_action_id("sysfs-restore"), None);
        assert_eq!(ActionId::new("test".to_owned()).as_str(), "test");
    }

//! The process tree a recording or experiment names explicitly (`--tree-pid`, `--pid`) is the
//! workload under analysis. The recorder classifies tasks from names and paths, so an arbitrary
//! target that is not a known game executable lands in a generic class (`Service`, `Helper`,
//! `Render`, `Unknown`). Tasks of an explicitly named process whose class is only such a generic
//! fallback are treated as the target. Infrastructure and specialised classes (compositor,
//! audio, browser, kernel, build tools, ...) are never reclassified: naming a process does not
//! make its helpers a game.
use super::model::TaskClass;

/// Classes the recorder assigns when it cannot recognise a workload.
pub fn is_generic_fallback(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Service | TaskClass::Helper | TaskClass::Render | TaskClass::Unknown
    )
}

/// The class a task counts as, given whether it belongs to an explicitly named process.
pub fn class_for_explicit_target(class: TaskClass, is_explicit_pid: bool) -> TaskClass {
    if is_explicit_pid && is_generic_fallback(class) {
        TaskClass::Game
    } else {
        class
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_class_of_a_named_process_becomes_the_target() {
        for class in [
            TaskClass::Service,
            TaskClass::Helper,
            TaskClass::Render,
            TaskClass::Unknown,
        ] {
            assert_eq!(class_for_explicit_target(class, true), TaskClass::Game);
        }
    }

    #[test]
    fn specialised_classes_are_never_promoted() {
        for class in [
            TaskClass::Compositor,
            TaskClass::AudioRealtime,
            TaskClass::KernelThread,
        ] {
            assert_eq!(class_for_explicit_target(class, true), class);
        }
    }

    #[test]
    fn unrelated_processes_keep_their_class() {
        assert_eq!(
            class_for_explicit_target(TaskClass::Service, false),
            TaskClass::Service
        );
    }
}

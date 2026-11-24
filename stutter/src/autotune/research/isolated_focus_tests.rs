use super::*;

/// A Sway tree with one workspace holding two windows; `focused` says which of pids 100 and 200 has focus.
fn tree(focused: u32) -> String {
    let window = |pid: u32| {
        serde_json::json!({ "id": pid, "type": "con", "pid": pid, "app_id": "game", "name": "w",
                            "focused": pid == focused, "nodes": [], "floating_nodes": [] })
    };
    serde_json::json!({ "id": 1, "type": "root", "nodes": [
        { "id": 2, "type": "output", "nodes": [
            { "id": 3, "type": "workspace", "name": "1", "nodes": [window(100), window(200)], "floating_nodes": [] }
        ], "floating_nodes": [] }
    ], "floating_nodes": [] })
    .to_string()
}

#[test]
fn only_the_attested_games_own_window_having_focus_passes() {
    assert_eq!(game_is_focused(&tree(100), 100), Ok(()));
    assert_eq!(
        game_is_focused(&tree(200), 100),
        Err("GameNotFocusedInTheIsolatedSession")
    );
    // Nothing focused, or not a tree at all, is unreadable and never a pass.
    assert_eq!(
        game_is_focused(&tree(0), 100),
        Err("IsolatedFocusUnreadable")
    );
    assert_eq!(
        game_is_focused("not json", 100),
        Err("IsolatedFocusUnreadable")
    );
}

#[test]
fn a_focused_window_with_no_process_id_is_unattributed() {
    let tree = serde_json::json!({ "id": 1, "type": "root", "nodes": [
        { "id": 3, "type": "workspace", "name": "1", "nodes": [
            { "id": 9, "type": "con", "app_id": "game", "focused": true, "nodes": [], "floating_nodes": [] }
        ], "floating_nodes": [] }
    ], "floating_nodes": [] })
    .to_string();
    assert_eq!(
        game_is_focused(&tree, 100),
        Err("IsolatedFocusUnattributed")
    );
}

/// The query runs the named program with only the isolated socket in its environment, and a slow or failing one is refused.
#[test]
fn the_query_passes_only_the_isolated_socket_and_refuses_a_slow_or_failing_compositor() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let socket = dir.path().join("sway.sock");
    let echo = script("echo-env", "echo \"$SWAYSOCK|$WAYLAND_DISPLAY|$HOME|$*\"");
    let answer = query_tree(&echo, &socket).unwrap();
    let expected = format!("{}|||-t get_tree\n", socket.display());
    assert_eq!(answer, expected);
    assert_eq!(
        query_tree(&script("fails", "exit 3"), &socket),
        Err("IsolatedFocusUnreadable")
    );
    assert_eq!(
        query_tree(&script("hangs", "sleep 30"), &socket),
        Err("IsolatedFocusUnreadable")
    );
    assert_eq!(
        query_tree(Path::new("relative"), &socket),
        Err("IsolatedFocusProbeUnusable")
    );
}

/// A Hyprland session is asked with its own instance signature and runtime directory, and only its active window's own pid passes.
#[test]
fn a_hyprland_session_is_asked_through_its_own_instance_and_only_the_games_pid_passes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let probe = |hyprctl: std::path::PathBuf| Probe::Hyprland {
        hyprctl,
        instance_signature: "sig-77".to_owned(),
        runtime_dir: dir.path().join("run"),
    };
    // The program answers only when it was given exactly the isolated session's instance and arguments.
    let active = script(
        "hyprctl",
        &format!(
            "[ \"$HYPRLAND_INSTANCE_SIGNATURE\" = sig-77 ] && [ \"$XDG_RUNTIME_DIR\" = '{}' ] && [ \"$*\" = '-j activewindow' ] \
             && echo '{{\"address\":\"0x1\",\"class\":\"game\",\"pid\":100,\"title\":\"t\"}}'",
            dir.path().join("run").display()
        ),
    );
    assert_eq!(probe(active.clone()).game_is_focused(100), Ok(()));
    assert_eq!(
        probe(active.clone()).game_is_focused(200),
        Err("GameNotFocusedInTheIsolatedSession")
    );
    // No active window ({} / empty), or a program that fails, never passes.
    let none = script("none", "echo '{}'");
    assert!(probe(none).game_is_focused(100).is_err());
    assert_eq!(
        probe(script("fails", "exit 1")).game_is_focused(100),
        Err("IsolatedFocusUnreadable")
    );
}

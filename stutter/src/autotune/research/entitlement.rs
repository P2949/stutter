//! Local evidence that the authenticated Steam account owns and has run the game. Steam keeps no
//! unencrypted license list, so this is deliberately weaker than a license proof: the account that
//! the running client is signed in as must have a recorded launch (a nonzero `LastPlayed` and
//! `Playtime`) for the app in its own `localconfig.vdf`. The appmanifest's `LastOwner` is not used
//! because a family-shared install legitimately names another account. The owned launch itself
//! remains the final proof, and every unreadable or ambiguous source yields `false`.

use std::path::Path;

use super::{proton::SteamProcessIdentity, vdf::VdfDocument};

/// Offset between a 64-bit SteamID of the public universe and its 32-bit account id.
const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;
const MAX_LOCALCONFIG_BYTES: u64 = 8 * 1024 * 1024;

/// The signed-in account id carried by the Steam web helper, bound to the expected client pid.
fn authenticated_account(cmdline: &[u8], steam_pid: u32) -> Option<u32> {
    let mut account = None;
    let mut bound = false;
    for arg in cmdline.split(|b| *b == 0) {
        let arg = std::str::from_utf8(arg).ok()?;
        if let Some(value) = arg.strip_prefix("-steamid=") {
            // A repeated argument is ambiguous; refuse it.
            if account.is_some() {
                return None;
            }
            let id: u64 = value.parse().ok()?;
            account = Some(u32::try_from(id.checked_sub(STEAM_ID64_BASE)?).ok()?);
        } else if let Some(value) = arg.strip_prefix("-steampid=") {
            bound = value.parse::<u32>().ok() == Some(steam_pid);
        }
    }
    account.filter(|account| *account != 0 && bound)
}

fn positive(value: Option<&str>) -> bool {
    value
        .and_then(|text| text.parse::<u64>().ok())
        .is_some_and(|number| number > 0)
}

/// Whether the account's own store records a launch of `app_id`.
fn account_has_played(localconfig: &[u8], app_id: u32) -> anyhow::Result<bool> {
    let document = VdfDocument::parse(localconfig)?;
    anyhow::ensure!(
        document.name == "userlocalconfigstore",
        "unexpected localconfig root"
    );
    let mut node = document.root_object()?;
    for key in ["software", "valve", "steam", "apps"] {
        match node.get(key).and_then(|value| value.object()) {
            Some(next) => node = next,
            None => return Ok(false),
        }
    }
    let Some(app) = node
        .get(&app_id.to_string())
        .and_then(|value| value.object())
    else {
        return Ok(false);
    };
    let text = |key: &str| app.get(key).and_then(|value| value.string());
    Ok(positive(text("lastplayed")) && positive(text("playtime")))
}

/// Whether the account the running client is signed in as has played the app. Any failure is
/// reported as `false`; it never raises an error that could be mistaken for proof.
pub fn authenticated_account_has_played(
    proc_root: &Path,
    steam_root: &Path,
    process: &SteamProcessIdentity,
    app_id: u32,
) -> bool {
    let check = || -> anyhow::Result<bool> {
        anyhow::ensure!(process.logged_in, "client is not signed in");
        let cmdline = std::fs::read(
            proc_root
                .join(process.webhelper_pid.to_string())
                .join("cmdline"),
        )?;
        let Some(account) = authenticated_account(&cmdline, process.steam_pid) else {
            return Ok(false);
        };
        let path = steam_root
            .join("userdata")
            .join(account.to_string())
            .join("config/localconfig.vdf");
        let file = std::fs::File::open(&path)?;
        anyhow::ensure!(
            file.metadata()?.len() <= MAX_LOCALCONFIG_BYTES,
            "localconfig is unexpectedly large"
        );
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(file, MAX_LOCALCONFIG_BYTES),
            &mut bytes,
        )?;
        account_has_played(&bytes, app_id)
    };
    check().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#""UserLocalConfigStore"
{
    "Software" { "Valve" { "Steam" { "apps" {
        "1091500" { "LastPlayed" "1794184164" "Playtime" "14757" }
        "42" { "LastPlayed" "0" "Playtime" "0" }
        "43" { "LastPlayed" "5" }
    } } } }
}"#;

    fn cmdline(steamid: &str, pid: &str) -> Vec<u8> {
        format!("./steamwebhelper\0-steampid={pid}\0-steamid={steamid}\0").into_bytes()
    }

    #[test]
    fn the_account_id_comes_only_from_a_helper_bound_to_the_expected_client() {
        assert_eq!(
            authenticated_account(&cmdline("76561198992219951", "7"), 7),
            Some(1_031_954_223)
        );
        assert_eq!(
            authenticated_account(&cmdline("76561198992219951", "7"), 8),
            None
        );
        assert_eq!(authenticated_account(&cmdline("0", "7"), 7), None);
        assert_eq!(
            authenticated_account(&cmdline("76561197960265728", "7"), 7),
            None
        );
        assert_eq!(authenticated_account(&cmdline("12", "7"), 7), None);
        let mut twice = cmdline("76561198992219951", "7");
        twice.extend_from_slice(b"-steamid=76561198992219952\0");
        assert_eq!(authenticated_account(&twice, 7), None);
    }

    #[test]
    fn only_a_recorded_launch_with_playtime_counts() {
        assert!(account_has_played(CONFIG.as_bytes(), 1_091_500).unwrap());
        assert!(!account_has_played(CONFIG.as_bytes(), 42).unwrap());
        assert!(!account_has_played(CONFIG.as_bytes(), 43).unwrap());
        assert!(!account_has_played(CONFIG.as_bytes(), 44).unwrap());
        assert!(account_has_played(b"\"Other\" { }", 1_091_500).is_err());
        assert!(!account_has_played(b"\"UserLocalConfigStore\" { }", 1_091_500).unwrap());
    }

    #[test]
    fn evidence_is_read_from_the_signed_in_account_and_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (proc_root, steam_root) = (dir.path().join("proc"), dir.path().join("steam"));
        std::fs::create_dir_all(proc_root.join("11")).unwrap();
        std::fs::write(
            proc_root.join("11/cmdline"),
            cmdline("76561198992219951", "10"),
        )
        .unwrap();
        let config_dir = steam_root.join("userdata/1031954223/config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("localconfig.vdf"), CONFIG).unwrap();
        let process = SteamProcessIdentity {
            steam_pid: 10,
            steam_start_ticks: 1,
            webhelper_pid: 11,
            webhelper_start_ticks: 2,
            logged_in: true,
        };
        assert!(authenticated_account_has_played(
            &proc_root,
            &steam_root,
            &process,
            1_091_500
        ));
        assert!(!authenticated_account_has_played(
            &proc_root,
            &steam_root,
            &process,
            7
        ));
        // Another account's record is never consulted.
        std::fs::rename(
            steam_root.join("userdata/1031954223"),
            steam_root.join("userdata/5"),
        )
        .unwrap();
        assert!(!authenticated_account_has_played(
            &proc_root,
            &steam_root,
            &process,
            1_091_500
        ));
        let signed_out = SteamProcessIdentity {
            logged_in: false,
            ..process
        };
        assert!(!authenticated_account_has_played(
            &proc_root,
            &steam_root,
            &signed_out,
            1_091_500
        ));
    }
}

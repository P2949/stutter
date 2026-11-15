//! Pure planning for the unprivileged Steam compatibility-tool dispatcher. Steam invokes the
//! dispatcher with Proton-style arguments; this module decides, without touching the host, what
//! the real Proton invocation must be or why the launch is refused. The caller consumes the launch
//! ticket first and supplies the tool directory resolved from the verified inventory.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::launch_ticket::SteamVerb;

const CYBERPUNK_APP_ID: u32 = 1_091_500;
const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 4096;
/// Prefixes the campaign alone may control; Steam launch options or the ambient environment must
/// not smuggle in factors that the frozen arm does not declare.
const CONTROLLED_PREFIXES: [&str; 6] =
    ["PROTON_", "WINE", "DXVK_", "VKD3D_", "MLFG_", "WAYLANDDRV_"];
const CONTROLLED_EXACT: [&str; 3] = ["LD_PRELOAD", "SteamDeck", "STEAM_COMPAT_MOUNTS"];
/// Controlled-set variables that Steam itself sets on launches, accepted only in the exact form Steam
/// produces: cache and crash-report locations as well-formed absolute paths, its container mounts as
/// absolute paths, and its overlay preload as nothing but `gameoverlayrenderer.so` libraries. Any
/// other value, and every other controlled key, is a smuggled setting.
fn steam_provided(key: &str, value: &str) -> bool {
    let well_formed = |path: &str| absolute_normal(Path::new(path));
    match key {
        "DXVK_STATE_CACHE_PATH" | "PROTON_CRASH_REPORT_DIR" => well_formed(value),
        "STEAM_COMPAT_MOUNTS" => !value.is_empty() && value.split(':').all(well_formed),
        "LD_PRELOAD" => {
            !value.is_empty()
                && value.split([':', ' ']).all(|library| {
                    well_formed(library)
                        && Path::new(library)
                            .file_name()
                            .is_some_and(|name| name == "gameoverlayrenderer.so")
                })
        }
        _ => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DispatchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

pub(crate) fn controlled(key: &str) -> bool {
    CONTROLLED_EXACT.contains(&key)
        || CONTROLLED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn absolute_normal(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}

/// `argv[0]` is Steam's view of the tool entry point and is never trusted. `arm_env` is the frozen
/// per-arm factor environment that the campaign applies; it replaces any ambient controlled keys.
pub fn plan_dispatch(
    argv: &[String],
    ambient_env: &BTreeMap<String, String>,
    verb_expected: SteamVerb,
    study_compat_data: &Path,
    tool_dir: &Path,
    arm_env: &BTreeMap<String, String>,
) -> anyhow::Result<DispatchPlan> {
    anyhow::ensure!(
        argv.len() >= 3 && argv.len() <= MAX_ARGS,
        "unexpected Steam dispatcher argument count"
    );
    anyhow::ensure!(
        argv.iter()
            .all(|arg| arg.len() <= MAX_ARG_BYTES && !arg.contains('\0')),
        "invalid Steam dispatcher argument"
    );
    let verb = SteamVerb::from_steam_argument(&argv[1])
        .ok_or_else(|| anyhow::anyhow!("unsupported Steam compatibility verb"))?;
    anyhow::ensure!(
        verb == verb_expected,
        "Steam verb differs from the launch ticket"
    );
    anyhow::ensure!(
        absolute_normal(tool_dir) && absolute_normal(study_compat_data),
        "invalid dispatcher path"
    );
    let app_id = |key: &str| {
        ambient_env
            .get(key)
            .and_then(|value| value.parse::<u32>().ok())
    };
    // `STEAM_COMPAT_APP_ID` is set for every verb, whereas the `run` verb (install scripts and
    // runtime preparation) carries no `SteamAppId`/`SteamGameId`. It is the authoritative identity,
    // and the other two must agree with it whenever Steam provides them.
    anyhow::ensure!(
        app_id("STEAM_COMPAT_APP_ID") == Some(CYBERPUNK_APP_ID)
            && ["SteamAppId", "SteamGameId"].iter().all(|key| {
                ambient_env.get(*key).is_none() || app_id(key) == Some(CYBERPUNK_APP_ID)
            }),
        "Steam did not launch Cyberpunk 2077"
    );
    // Steam always hands the tool the player's own per-game compatdata. That path must be present and
    // well formed, and is then discarded: the real Proton runs only in the isolated study prefix, so
    // the player's prefix is never opened by a research launch.
    anyhow::ensure!(
        ambient_env
            .get("STEAM_COMPAT_DATA_PATH")
            .is_some_and(|value| absolute_normal(Path::new(value))),
        "Steam did not provide its compatibility data path"
    );
    anyhow::ensure!(
        ambient_env
            .get("STEAM_COMPAT_CLIENT_INSTALL_PATH")
            .is_some_and(|value| absolute_normal(Path::new(value))),
        "missing Steam client install path"
    );
    let smuggled = ambient_env
        .iter()
        .find(|(key, value)| controlled(key) && !steam_provided(key, value));
    if let Some((key, _)) = smuggled {
        anyhow::bail!("ambient controlled setting {key} is not authorized for this arm");
    }
    anyhow::ensure!(
        arm_env.keys().all(|key| controlled(key)),
        "arm environment contains a key outside the controlled set"
    );
    let mut env = ambient_env.clone();
    env.extend(arm_env.clone());
    env.insert(
        "STEAM_COMPAT_DATA_PATH".to_owned(),
        study_compat_data.to_string_lossy().into_owned(),
    );
    let mut args = vec![verb.as_steam_argument().to_owned()];
    args.extend(argv[2..].iter().cloned());
    Ok(DispatchPlan {
        program: tool_dir.join("proton"),
        args,
        env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> BTreeMap<String, String> {
        [
            ("SteamAppId", "1091500"),
            ("SteamGameId", "1091500"),
            ("STEAM_COMPAT_APP_ID", "1091500"),
            ("STEAM_COMPAT_DATA_PATH", "/home/u/study/compatdata"),
            ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/home/u/.steam/steam"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    fn argv(verb: &str) -> Vec<String> {
        ["/x/proton", verb, "/g/Cyberpunk2077.exe", "-benchmark"]
            .map(str::to_owned)
            .to_vec()
    }

    fn plan(argv: &[String], env: &BTreeMap<String, String>) -> anyhow::Result<DispatchPlan> {
        plan_dispatch(
            argv,
            env,
            SteamVerb::Run,
            Path::new("/home/u/study/compatdata"),
            Path::new("/tools/GE"),
            &BTreeMap::from([("PROTON_ENABLE_WAYLAND".to_owned(), "1".to_owned())]),
        )
    }

    #[test]
    fn plans_the_real_entry_point_and_ignores_steams_argv0() {
        let plan = plan(&argv("run"), &env()).unwrap();
        assert_eq!(plan.program, Path::new("/tools/GE/proton"));
        assert_eq!(plan.args[0], "run");
        assert_eq!(plan.args[1], "/g/Cyberpunk2077.exe");
        assert_eq!(plan.env["PROTON_ENABLE_WAYLAND"], "1");
    }

    #[test]
    fn steams_own_prefix_is_replaced_by_the_isolated_study_prefix() {
        let mut steam = env();
        steam.insert(
            "STEAM_COMPAT_DATA_PATH".into(),
            "/home/u/.local/share/Steam/steamapps/compatdata/1091500".into(),
        );
        let plan = plan(&argv("run"), &steam).unwrap();
        assert_eq!(
            plan.env["STEAM_COMPAT_DATA_PATH"],
            "/home/u/study/compatdata"
        );
        assert!(
            !plan
                .env
                .values()
                .any(|value| value.contains("steamapps/compatdata"))
        );
    }

    #[test]
    fn the_run_verb_without_steam_app_ids_is_identified_by_the_compat_app_id() {
        let mut run = env();
        run.remove("SteamAppId");
        run.remove("SteamGameId");
        assert!(plan(&argv("run"), &run).is_ok());
    }

    #[test]
    fn steams_own_cache_location_is_tolerated_but_no_other_controlled_setting() {
        let mut steam = env();
        steam.insert(
            "DXVK_STATE_CACHE_PATH".into(),
            "/home/u/.local/share/Steam/steamapps/shadercache/1091500".into(),
        );
        assert!(plan(&argv("run"), &steam).is_ok());
        steam.insert("DXVK_STATE_CACHE_PATH".into(), "relative/cache".into());
        assert!(plan(&argv("run"), &steam).is_err());
        steam.insert("DXVK_STATE_CACHE_PATH".into(), "/c/../x".into());
        assert!(plan(&argv("run"), &steam).is_err());
        let mut other = env();
        other.insert("DXVK_STATE_CACHE".into(), "0".into());
        assert!(plan(&argv("run"), &other).is_err());
        // The other controlled keys Steam sets itself, in their real forms.
        let mut steam = env();
        for (key, value) in [
            ("PROTON_CRASH_REPORT_DIR", "/home/u/.local/share/Steam/logs"),
            ("STEAM_COMPAT_MOUNTS", "/a/steamapps:/b/lib"),
            (
                "LD_PRELOAD",
                "/s/steamrt32/gameoverlayrenderer.so:/s/steamrt64/gameoverlayrenderer.so",
            ),
        ] {
            steam.insert(key.into(), value.into());
        }
        assert!(plan(&argv("run"), &steam).is_ok());
        for (key, value) in [
            ("LD_PRELOAD", "/tmp/evil.so"),
            ("LD_PRELOAD", "/s/gameoverlayrenderer.so:/tmp/evil.so"),
            ("LD_PRELOAD", ""),
            ("STEAM_COMPAT_MOUNTS", "relative:/a"),
            ("PROTON_CRASH_REPORT_DIR", "rel"),
            ("PROTON_LOG", "1"),
        ] {
            let mut bad = env();
            bad.insert(key.into(), value.into());
            assert!(plan(&argv("run"), &bad).is_err(), "{key}={value}");
        }
    }

    #[test]
    fn refuses_wrong_verb_game_prefix_and_smuggled_settings() {
        assert!(plan(&argv("waitforexitandrun"), &env()).is_err());
        assert!(plan(&argv("getcompatpath"), &env()).is_err());
        let mut other = env();
        other.insert("SteamAppId".into(), "220".into());
        assert!(plan(&argv("run"), &other).is_err());
        let mut other = env();
        other.insert("STEAM_COMPAT_APP_ID".into(), "220".into());
        assert!(plan(&argv("run"), &other).is_err());
        other.remove("STEAM_COMPAT_APP_ID");
        assert!(plan(&argv("run"), &other).is_err());
        let mut other = env();
        other.insert("SteamGameId".into(), "220".into());
        assert!(plan(&argv("run"), &other).is_err());
        let mut other = env();
        other.remove("STEAM_COMPAT_DATA_PATH");
        assert!(plan(&argv("run"), &other).is_err());
        other.insert(
            "STEAM_COMPAT_DATA_PATH".into(),
            "relative/compatdata".into(),
        );
        assert!(plan(&argv("run"), &other).is_err());
        let mut other = env();
        other.insert("DXVK_HUD".into(), "1".into());
        assert!(plan(&argv("run"), &other).is_err());
        assert!(plan(&argv("run")[..2], &env()).is_err());
    }
}

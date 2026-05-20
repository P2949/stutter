//! Public API surface architecture tests; this module owns expected public module checks.

use std::fs;

use super::{
    allowlists::{EXPECTED_API_PUBLIC_MODULES, EXPECTED_ROOT_PUBLIC_MODULES},
    crate_src_root,
};

fn public_modules_from_source(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let module = line.strip_prefix("pub mod ")?;
            let module = module.trim();
            let module = module
                .strip_suffix(';')
                .or_else(|| module.strip_suffix('{'))?;
            Some(module.trim().to_owned())
        })
        .collect()
}

fn root_public_modules_from_lib_rs(source: &str) -> Vec<String> {
    public_modules_from_source(source)
}

fn expected_root_public_module_names() -> Vec<String> {
    EXPECTED_ROOT_PUBLIC_MODULES
        .iter()
        .map(|module| module.name.to_owned())
        .collect()
}

fn expected_api_public_module_names() -> Vec<String> {
    EXPECTED_API_PUBLIC_MODULES
        .iter()
        .map(|module| module.name.to_owned())
        .collect()
}

#[test]
fn root_public_modules_are_intentional() {
    for module in EXPECTED_ROOT_PUBLIC_MODULES {
        assert!(
            !module.reason.trim().is_empty(),
            "public module '{}' must have a reason explaining why it is exported",
            module.name
        );
    }

    let lib_rs_path = crate_src_root().join("lib.rs");
    let lib_rs = fs::read_to_string(&lib_rs_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", lib_rs_path.display()));

    let actual = root_public_modules_from_lib_rs(&lib_rs);
    let expected = expected_root_public_module_names();

    assert_eq!(
        actual, expected,
        "root public module exports changed; update EXPECTED_ROOT_PUBLIC_MODULES with the intentional public module list and a reason for every exported module"
    );
}

#[test]
fn api_public_modules_are_intentional() {
    for module in EXPECTED_API_PUBLIC_MODULES {
        assert!(
            !module.reason.trim().is_empty(),
            "api public module '{}' must have a reason explaining why it is exported",
            module.name
        );
    }

    let api_rs_path = crate_src_root().join("api.rs");
    let api_rs = fs::read_to_string(&api_rs_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", api_rs_path.display()));

    let actual = public_modules_from_source(&api_rs);
    let expected = expected_api_public_module_names();

    assert_eq!(
        actual, expected,
        "api public module exports changed; update EXPECTED_API_PUBLIC_MODULES with the intentional public facade module list and a reason for every exported section"
    );
}

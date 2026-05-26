import os
import re

with open("stutter-config/src/model.rs", "r") as f:
    config_model_code = f.read()

with open("stutter/src/config/model.rs", "r") as f:
    monitor_config_code = f.read()

# Fix imports in monitor_config_code
monitor_config_code = monitor_config_code.replace(
    "use crate::config::{", "use crate::{"
)
monitor_config_code = monitor_config_code.replace(
    "crate::foreground::DEFAULT_FOREGROUND_POLL_MS", "DEFAULT_FOREGROUND_POLL_MS"
)
# remove the use std::... from monitor_config_code because we will merge it.
monitor_config_code = monitor_config_code.replace("use std::{path::PathBuf, time::Duration};\n", "")

combined = """use std::{path::PathBuf, time::Duration};

""" + config_model_code + "\n\npub const DEFAULT_FOREGROUND_POLL_MS: u64 = 1_000;\n\n" + monitor_config_code

with open("stutter-config/src/model.rs", "w") as f:
    f.write(combined)

# Replace stutter/src/config/model.rs with just the re-export
with open("stutter/src/config/model.rs", "w") as f:
    f.write("pub use stutter_config::model::*;\n")

# Remove DEFAULT_FOREGROUND_POLL_MS from stutter/src/foreground/model.rs
with open("stutter/src/foreground/model.rs", "r") as f:
    fg_code = f.read()

fg_code = fg_code.replace("pub const DEFAULT_FOREGROUND_POLL_MS: u64 = 1_000;", "pub use stutter_config::model::DEFAULT_FOREGROUND_POLL_MS;")

with open("stutter/src/foreground/model.rs", "w") as f:
    f.write(fg_code)


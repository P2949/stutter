mod classify;
mod discover;
mod model;
mod read;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
use classify::parse_nvidia_smi_sample;
#[cfg(test)]
use discover::discover_drm_hwmon_root;
pub use discover::probe_hwmon_with_options;
pub use model::HwmonReader;
#[cfg(test)]
use model::NvidiaState;

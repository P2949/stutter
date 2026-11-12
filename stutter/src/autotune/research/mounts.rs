//! Mount-namespace preflight for detecting study paths that alias Steam through bind mounts.
//! It is not a Steam Cloud proof and must be repeated in the namespace that launches the game.
use std::{
    ffi::OsString,
    fs::File,
    io::Read,
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
};

const MAX_MOUNTINFO_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MOUNTS: usize = 8192;

fn decode_path(value: &[u8]) -> anyhow::Result<PathBuf> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'\\' {
            anyhow::ensure!(index + 3 < value.len(), "invalid mount escape");
            let digits = &value[index + 1..index + 4];
            anyhow::ensure!(
                digits.iter().all(|byte| (b'0'..=b'7').contains(byte)),
                "invalid mount escape"
            );
            let number = u16::from(digits[0] - b'0') * 64
                + u16::from(digits[1] - b'0') * 8
                + u16::from(digits[2] - b'0');
            anyhow::ensure!(number > 0 && number <= 255, "invalid mount escape");
            decoded.push(number as u8);
            index += 4;
        } else {
            anyhow::ensure!(value[index] != 0, "invalid mount path");
            decoded.push(value[index]);
            index += 1;
        }
    }
    let path = PathBuf::from(OsString::from_vec(decoded));
    anyhow::ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "invalid mount path"
    );
    Ok(path)
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedMount {
    device: Vec<u8>,
    filesystem: Vec<u8>,
    backing_path: PathBuf,
}

fn resolve(snapshot: &[u8], target: &Path) -> anyhow::Result<ResolvedMount> {
    anyhow::ensure!(
        target.is_absolute()
            && target
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "invalid mount target"
    );
    let mut best: Option<(usize, ResolvedMount)> = None;
    let mut count = 0;
    for line in snapshot
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        count += 1;
        anyhow::ensure!(count <= MAX_MOUNTS, "too many mounts");
        let separator = line
            .windows(3)
            .position(|window| window == b" - ")
            .ok_or_else(|| anyhow::anyhow!("invalid mount record"))?;
        let mut fields = line[..separator].split(|byte| *byte == b' ');
        let _mount_id = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing mount ID"))?;
        let _parent_id = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing mount parent"))?;
        let device = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing mount device"))?;
        let mut numbers = device.split(|byte| *byte == b':');
        let major = numbers.next().unwrap_or_default();
        let minor = numbers.next().unwrap_or_default();
        anyhow::ensure!(
            !major.is_empty()
                && !minor.is_empty()
                && numbers.next().is_none()
                && major.iter().chain(minor).all(u8::is_ascii_digit),
            "invalid mount device"
        );
        let root = decode_path(
            fields
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing mount root"))?,
        )?;
        let mountpoint = decode_path(
            fields
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing mountpoint"))?,
        )?;
        let Some(relative) = target.strip_prefix(&mountpoint).ok() else {
            continue;
        };
        let depth = mountpoint.components().count();
        if best.as_ref().is_some_and(|(current, _)| depth < *current) {
            continue;
        }
        anyhow::ensure!(
            !best.as_ref().is_some_and(|(current, _)| depth == *current),
            "ambiguous stacked mountpoint"
        );
        let filesystem = line[separator + 3..]
            .split(|byte| *byte == b' ')
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing filesystem"))?;
        anyhow::ensure!(!filesystem.is_empty(), "missing filesystem");
        best = Some((
            depth,
            ResolvedMount {
                device: device.to_vec(),
                filesystem: filesystem.to_vec(),
                backing_path: root.join(relative),
            },
        ));
    }
    best.map(|(_, mount)| mount)
        .ok_or_else(|| anyhow::anyhow!("target mount unavailable"))
}

/// Resolve both paths from the same bounded mount snapshot. Network and layered filesystems can
/// expose a backing tree under different device identities and are refused pending qualification.
pub(super) fn ensure_disjoint(snapshot: &[u8], left: &Path, right: &Path) -> anyhow::Result<()> {
    let left = resolve(snapshot, left)?;
    let right = resolve(snapshot, right)?;
    let supported = |fs: &[u8]| matches!(fs, b"btrfs" | b"ext4" | b"xfs" | b"tmpfs");
    anyhow::ensure!(
        supported(&left.filesystem) && supported(&right.filesystem),
        "mount filesystem alias is unverified"
    );
    anyhow::ensure!(
        left.device != right.device
            || !(left.backing_path.starts_with(&right.backing_path)
                || right.backing_path.starts_with(&left.backing_path)),
        "study path aliases a Steam mount"
    );
    Ok(())
}

pub(super) fn snapshot() -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open("/proc/self/mountinfo")?
        .take(MAX_MOUNTINFO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_MOUNTINFO_BYTES,
        "mount snapshot unavailable or too large"
    );
    Ok(bytes)
}

#[cfg(test)]
#[path = "mounts_tests.rs"]
mod tests;

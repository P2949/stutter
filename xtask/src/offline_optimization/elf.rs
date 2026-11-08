//! The GNU build id of an ELF file, read without a tool: the `.note.gnu.build-id` note.
use std::{fs, io, path::Path};

const NT_GNU_BUILD_ID: u32 = 3;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// The build id as lowercase hex, `None` when the file is not a little-endian ELF64 or has none.
pub fn parse_build_id(bytes: &[u8]) -> Option<String> {
    if bytes.get(0..4)? != b"\x7fELF" || *bytes.get(4)? != 2 || *bytes.get(5)? != 1 {
        return None;
    }
    let section_offset = usize::try_from(u64_at(bytes, 0x28)?).ok()?;
    let entry_size = usize::from(u16_at(bytes, 0x3a)?);
    let count = usize::from(u16_at(bytes, 0x3c)?);
    for index in 0..count {
        let header = section_offset.checked_add(index.checked_mul(entry_size)?)?;
        if u32_at(bytes, header + 4)? != 7 {
            continue; // SHT_NOTE only
        }
        let offset = usize::try_from(u64_at(bytes, header + 0x18)?).ok()?;
        let size = usize::try_from(u64_at(bytes, header + 0x20)?).ok()?;
        let section = bytes.get(offset..offset.checked_add(size)?)?;
        let mut at = 0;
        while at + 12 <= section.len() {
            let name_size = usize::try_from(u32_at(section, at)?).ok()?;
            let desc_size = usize::try_from(u32_at(section, at + 4)?).ok()?;
            let kind = u32_at(section, at + 8)?;
            let name_start = at + 12;
            let desc_start = name_start.checked_add(name_size.next_multiple_of(4))?;
            let desc = section.get(desc_start..desc_start.checked_add(desc_size)?)?;
            if kind == NT_GNU_BUILD_ID
                && section.get(name_start..name_start + name_size)? == b"GNU\0"
            {
                return Some(desc.iter().map(|byte| format!("{byte:02x}")).collect());
            }
            at = desc_start.checked_add(desc_size.next_multiple_of(4))?;
        }
    }
    None
}

/// Reads at most 256 MiB of `path`; anything larger is refused rather than loaded.
pub fn build_id(path: &Path) -> io::Result<Option<String>> {
    let meta = fs::metadata(path)?;
    if meta.len() > 256 * 1024 * 1024 {
        return Err(io::Error::other("binary is too large to inspect"));
    }
    Ok(parse_build_id(&fs::read(path)?))
}

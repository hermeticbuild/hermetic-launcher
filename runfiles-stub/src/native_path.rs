extern crate alloc;

use alloc::vec::Vec;

#[cfg(any(test, target_os = "windows"))]
const WINDOWS_SEPARATOR: u16 = b'\\' as u16;

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub fn unix_path_is_absolute(path: &[u8]) -> bool {
    path.first() == Some(&b'/')
}

/// Join a UTF-8 fallback to the byte path used to launch a Unix executable.
/// The executable prefix stays opaque so valid non-UTF-8 path bytes survive.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub fn unix_executable_relative(executable: &[u8], fallback: &[u8]) -> Option<Vec<u8>> {
    let separator = executable.iter().rposition(|byte| *byte == b'/')?;
    let mut path = Vec::from(&executable[..separator]);
    if path.last() != Some(&b'/') {
        path.push(b'/');
    }
    path.extend_from_slice(fallback);
    Some(path)
}

/// Join a UTF-16 fallback to the path of a Windows executable without
/// narrowing any code units.
#[cfg(any(test, target_os = "windows"))]
pub fn windows_executable_relative(executable: &[u16], fallback: &str) -> Option<Vec<u16>> {
    let separator = executable
        .iter()
        .rposition(|unit| *unit == b'/' as u16 || *unit == b'\\' as u16)?;
    let mut path = Vec::from(&executable[..separator]);
    if !matches!(path.last(), Some(unit) if *unit == b'/' as u16 || *unit == b'\\' as u16) {
        path.push(b'\\' as u16);
    }
    path.extend(fallback.encode_utf16().map(|unit| {
        if unit == b'/' as u16 {
            b'\\' as u16
        } else {
            unit
        }
    }));
    Some(path)
}

#[cfg(any(test, target_os = "windows"))]
fn windows_is_separator(unit: u16) -> bool {
    unit == b'/' as u16 || unit == WINDOWS_SEPARATOR
}

#[cfg(any(test, target_os = "windows"))]
fn windows_starts_with_ascii(path: &[u16], prefix: &[u8]) -> bool {
    path.len() >= prefix.len()
        && path[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(&unit, &byte)| {
                let unit = if (b'a' as u16..=b'z' as u16).contains(&unit) {
                    unit - 32
                } else {
                    unit
                };
                let byte = byte.to_ascii_uppercase() as u16;
                unit == byte
            })
}

#[cfg(any(test, target_os = "windows"))]
fn windows_next_component<'a>(path: &'a [u16], position: &mut usize) -> Option<&'a [u16]> {
    while *position < path.len() && windows_is_separator(path[*position]) {
        *position += 1;
    }
    if *position == path.len() {
        return None;
    }
    let start = *position;
    while *position < path.len() && !windows_is_separator(path[*position]) {
        *position += 1;
    }
    Some(&path[start..*position])
}

#[cfg(any(test, target_os = "windows"))]
fn windows_normalized_components(path: &[u16], mut position: usize) -> Vec<&[u16]> {
    let mut components = Vec::new();
    while let Some(component) = windows_next_component(path, &mut position) {
        if component == [b'.' as u16] || component.is_empty() {
            continue;
        }
        if component == [b'.' as u16, b'.' as u16] {
            components.pop();
            continue;
        }
        components.push(component);
    }
    components
}

#[cfg(any(test, target_os = "windows"))]
fn windows_push_components(path: &mut Vec<u16>, components: &[&[u16]]) {
    for component in components {
        if path.last() != Some(&WINDOWS_SEPARATOR) {
            path.push(WINDOWS_SEPARATOR);
        }
        path.extend_from_slice(component);
    }
}

/// Convert an absolute DOS or UNC path to normalized Win32 verbatim form.
/// Relative and drive-relative paths are rejected so their current-directory
/// semantics are not accidentally changed.
#[cfg(any(test, target_os = "windows"))]
pub fn windows_extended_path(path: &[u16]) -> Option<Vec<u16>> {
    let path = path.strip_suffix(&[0]).unwrap_or(path);
    let has_verbatim_prefix = windows_starts_with_ascii(path, b"\\\\?\\");

    let unc_start = if windows_starts_with_ascii(path, b"\\\\?\\UNC\\") {
        Some(8)
    } else if !has_verbatim_prefix
        && path.len() >= 2
        && windows_is_separator(path[0])
        && windows_is_separator(path[1])
    {
        Some(2)
    } else {
        None
    };
    if let Some(mut position) = unc_start {
        let server = windows_next_component(path, &mut position)?;
        let share = windows_next_component(path, &mut position)?;
        if server.is_empty()
            || share.is_empty()
            || server == [b'.' as u16]
            || server == [b'.' as u16, b'.' as u16]
            || share == [b'.' as u16]
            || share == [b'.' as u16, b'.' as u16]
        {
            return None;
        }

        let mut extended: Vec<u16> = b"\\\\?\\UNC\\".iter().map(|&byte| byte as u16).collect();
        extended.extend_from_slice(server);
        extended.push(WINDOWS_SEPARATOR);
        extended.extend_from_slice(share);
        let components = windows_normalized_components(path, position);
        windows_push_components(&mut extended, &components);
        return Some(extended);
    }

    let drive_start = if has_verbatim_prefix { 4 } else { 0 };
    let drive = path.get(drive_start).copied().unwrap_or(0);
    if path.len() < drive_start + 3
        || !((b'A' as u16..=b'Z' as u16).contains(&drive)
            || (b'a' as u16..=b'z' as u16).contains(&drive))
        || path[drive_start + 1] != b':' as u16
        || !windows_is_separator(path[drive_start + 2])
    {
        return None;
    }

    let mut extended: Vec<u16> = b"\\\\?\\".iter().map(|&byte| byte as u16).collect();
    extended.extend_from_slice(&path[drive_start..drive_start + 2]);
    extended.push(WINDOWS_SEPARATOR);
    let components = windows_normalized_components(path, drive_start + 3);
    windows_push_components(&mut extended, &components);
    Some(extended)
}

/// Return a NUL-terminated path for a Win32 file/process API. Absolute DOS and
/// UNC paths use verbatim form to avoid MAX_PATH; relative paths retain their
/// original current-directory semantics.
#[cfg(any(test, target_os = "windows"))]
pub fn windows_api_path(path: &[u16]) -> Vec<u16> {
    let path = path.strip_suffix(&[0]).unwrap_or(path);
    let mut api_path = windows_extended_path(path).unwrap_or_else(|| {
        path.iter()
            .map(|&unit| {
                if unit == b'/' as u16 {
                    WINDOWS_SEPARATOR
                } else {
                    unit
                }
            })
            .collect()
    });
    api_path.push(0);
    api_path
}

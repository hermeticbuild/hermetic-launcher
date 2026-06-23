#[path = "../src/native_path.rs"]
mod native_path;

#[test]
fn executable_relative_path_preserves_non_utf8_prefix() {
    let executable = b"/tmp/launcher-\xff/stub";

    assert!(native_path::unix_path_is_absolute(executable));
    let path = native_path::unix_executable_relative(executable, b"../bin/tool").unwrap();

    assert_eq!(path, b"/tmp/launcher-\xff/../bin/tool");
}

#[test]
fn windows_executable_relative_preserves_long_utf16_path() {
    let mut executable: Vec<u16> = "C:\\launcher\\".encode_utf16().collect();
    executable.extend(core::iter::repeat_n(b'x' as u16, 260));
    executable.extend("\\stub.exe".encode_utf16());

    let resolved =
        native_path::windows_executable_relative(&executable, "../bin/工具.exe").unwrap();
    assert!(resolved.len() > 260);
    assert_eq!(
        String::from_utf16(&resolved).unwrap(),
        format!("C:\\launcher\\{}\\..\\bin\\工具.exe", "x".repeat(260)),
    );
}

#[test]
fn windows_api_path_normalizes_drive_path_for_win32() {
    let path: Vec<u16> = "C:/launcher/one/../two/tool.exe".encode_utf16().collect();
    let api_path = native_path::windows_api_path(&path);

    assert_eq!(
        String::from_utf16(&api_path[..api_path.len() - 1]).unwrap(),
        "\\\\?\\C:\\launcher\\two\\tool.exe",
    );
    assert_eq!(api_path.last(), Some(&0));
}

#[test]
fn windows_api_path_normalizes_unc_path_for_win32() {
    let path: Vec<u16> = "\\\\server\\share\\one\\..\\工具.exe"
        .encode_utf16()
        .collect();
    let api_path = native_path::windows_api_path(&path);

    assert_eq!(
        String::from_utf16(&api_path[..api_path.len() - 1]).unwrap(),
        "\\\\?\\UNC\\server\\share\\工具.exe",
    );
}

#[test]
fn windows_api_path_preserves_relative_semantics() {
    let path: Vec<u16> = "relative/dir/tool.exe".encode_utf16().collect();
    let api_path = native_path::windows_api_path(&path);

    assert_eq!(
        String::from_utf16(&api_path[..api_path.len() - 1]).unwrap(),
        "relative\\dir\\tool.exe",
    );
}

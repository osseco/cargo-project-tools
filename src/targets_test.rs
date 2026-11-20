use super::*;

const MAC: &str = "aarch64-apple-darwin";
const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";

fn allowed() -> Vec<String> {
    vec![MAC.into(), LINUX.into()]
}

#[test]
fn unset_allows_every_target() {
    assert!(missing(WINDOWS, None, None).is_empty());
}

#[test]
fn listed_host_is_allowed() {
    assert!(missing(MAC, None, Some(&allowed())).is_empty());
    assert!(missing(MAC, Some(LINUX), Some(&allowed())).is_empty());
}

#[test]
fn unlisted_host_or_build_target_is_missing() {
    assert_eq!(missing(WINDOWS, None, Some(&allowed())), [WINDOWS]);
    assert_eq!(missing(MAC, Some(WINDOWS), Some(&allowed())), [WINDOWS]);
    assert_eq!(missing(WINDOWS, Some(WINDOWS), Some(&allowed())), [WINDOWS]);
}

#[test]
fn host_comes_from_rustc() {
    let host = host().unwrap();
    assert!(host.split('-').count() >= 3, "{host}");
}

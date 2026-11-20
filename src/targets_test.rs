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

#[test]
fn runner_labels_map_to_their_native_triple() {
    for (label, triple) in [
        ("ubuntu-latest", Some(LINUX)),
        ("ubuntu-24.04", Some(LINUX)),
        ("ubuntu-24.04-arm", Some("aarch64-unknown-linux-gnu")),
        ("macos-latest", Some(MAC)),
        ("macos-15", Some(MAC)),
        ("macos-14-xlarge", Some(MAC)),
        ("macos-13", Some("x86_64-apple-darwin")),
        ("macos-15-intel", Some("x86_64-apple-darwin")),
        ("macos-14-large", Some("x86_64-apple-darwin")),
        ("windows-latest", Some(WINDOWS)),
        ("windows-2025", Some(WINDOWS)),
        ("windows-11-arm", Some("aarch64-pc-windows-msvc")),
        ("windows", None),
        ("ubuntu-foo", None),
        ("self-hosted", None),
        ("macos-15-huge", None),
    ] {
        assert_eq!(runner_triple(label), triple, "{label}");
    }
}

#[test]
fn workflow_runners_are_found_in_every_form() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(".github/workflows");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("a.yml"),
        "jobs:\n  t:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, 'macos-15']\n        include:\n          - os: \"windows-latest\"\n    env:\n      X: ${{ startsWith(matrix.os, 'windows') }}\n",
    )
    .unwrap();
    fs::write(
        dir.join("b.yaml"),
        "jobs:\n  t:\n    runs-on: ubuntu-24.04\n",
    )
    .unwrap();
    fs::write(dir.join("notes.txt"), "macos-13").unwrap();
    // Found from a crate inside the repository too.
    let member = tmp.path().join("crates/x");
    fs::create_dir_all(&member).unwrap();
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let found = from_workflows(&member);
    let got: Vec<(&str, Vec<&str>)> = found
        .iter()
        .map(|s| {
            (
                s.triple.as_str(),
                s.sources.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (LINUX, vec!["a.yml", "b.yaml"]),
            (MAC, vec!["a.yml"]),
            (WINDOWS, vec!["a.yml"]),
        ]
    );
}

#[test]
fn workflows_outside_the_repository_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join(".github/workflows")).unwrap();
    fs::write(
        tmp.path().join(".github/workflows/a.yml"),
        "runs-on: ubuntu-latest\n",
    )
    .unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    assert!(from_workflows(&repo).is_empty());
}

fn suggested() -> Vec<Suggestion> {
    [MAC, LINUX]
        .iter()
        .map(|t| Suggestion {
            triple: t.to_string(),
            sources: vec!["x".into()],
        })
        .collect()
}

#[test]
fn prompt_answers() {
    let s = suggested();
    assert_eq!(parse_choice("", &s).unwrap().unwrap(), [MAC, LINUX]);
    assert_eq!(parse_choice(" 2 ", &s).unwrap().unwrap(), [LINUX]);
    assert_eq!(
        parse_choice("1, x86_64-pc-windows-msvc 1", &s)
            .unwrap()
            .unwrap(),
        [MAC, WINDOWS]
    );
    assert!(parse_choice("ALL", &s).unwrap().is_none());
    assert!(parse_choice("3", &s).is_err());
    assert!(parse_choice("0", &s).is_err());
    assert!(parse_choice("mac", &s).is_err());
    assert!(parse_choice("--x", &s).is_err());
}

#[test]
fn triples() {
    for good in [
        MAC,
        LINUX,
        WINDOWS,
        "wasm32-unknown-unknown",
        "thumbv7em-none-eabihf",
    ] {
        assert!(is_triple(good), "{good}");
    }
    for bad in [
        "", "linux", "-x", "x-", "a--b", "a b-c", "../x-y", "a/b-c", "--target",
    ] {
        assert!(!is_triple(bad), "{bad}");
    }
}

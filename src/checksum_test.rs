use super::*;

fn fixture() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project {
        root: dir.path().to_path_buf(),
    };
    fs::create_dir_all(project.bin_dir()).unwrap();
    fs::write(project.bin_dir().join("a"), b"binary a").unwrap();
    fs::write(project.bin_dir().join("b"), b"binary b").unwrap();
    write(&project, "tool", &["b".into(), "a".into()]).unwrap();
    (dir, project)
}

#[test]
fn round_trip_in_sha256sum_format() {
    let (_dir, project) = fixture();
    let text = fs::read_to_string(path(&project, "tool")).unwrap();
    assert_eq!(
        text.lines().next().unwrap(),
        format!("{}  a", crate::source::registry::sha256_hex(b"binary a"))
    );
    let recorded = read(&project, "tool").unwrap();
    assert_eq!(
        recorded.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(check(&project, "tool").unwrap().is_empty());
}

#[test]
fn tampered_and_missing_binaries_are_reported() {
    let (_dir, project) = fixture();
    fs::write(project.bin_dir().join("a"), b"binary a!").unwrap();
    fs::remove_file(project.bin_dir().join("b")).unwrap();
    assert_eq!(
        check(&project, "tool").unwrap(),
        [Mismatch::Differs("a".into()), Mismatch::Missing("b".into())]
    );
}

#[test]
fn missing_checksum_file_is_an_error() {
    let (_dir, project) = fixture();
    let err = check(&project, "other").unwrap_err().to_string();
    assert!(err.contains("no checksum for other"), "{err}");
}

#[test]
fn escaping_names_are_rejected() {
    let (_dir, project) = fixture();
    let hash = "0".repeat(64);
    for name in ["../x", "sub/x", "sub\\x", ".."] {
        fs::write(path(&project, "tool"), format!("{hash}  {name}\n")).unwrap();
        assert!(read(&project, "tool").is_err(), "{name}");
    }
}

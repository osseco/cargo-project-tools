use super::*;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A local repository with `kept` and `edited` committed; returns the commit.
fn repo(dir: &Path) -> String {
    run_git(dir, &["init", "--quiet"]);
    fs::write(dir.join("kept"), "kept").unwrap();
    fs::write(dir.join("edited"), "original").unwrap();
    run_git(dir, &["add", "kept", "edited"]);
    run_git(dir, &["commit", "--quiet", "-m", "init"]);
    run_git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn clone_at_rev_is_reset_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let rev = repo(dir.path());
    let kept_mtime = fs::metadata(dir.path().join("kept"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(dir.path().join("edited"), "changed").unwrap();
    fs::write(dir.path().join("untracked"), "x").unwrap();

    assert!(reuse(dir.path(), &rev));
    assert_eq!(
        fs::read_to_string(dir.path().join("edited")).unwrap(),
        "original"
    );
    assert!(!dir.path().join("untracked").exists());
    assert_eq!(
        fs::metadata(dir.path().join("kept"))
            .unwrap()
            .modified()
            .unwrap(),
        kept_mtime
    );
}

#[test]
fn other_or_short_revs_are_not_reused() {
    let dir = tempfile::tempdir().unwrap();
    let rev = repo(dir.path());
    assert!(!reuse(dir.path(), &rev[..12]));
    assert!(!reuse(dir.path(), &"0".repeat(40)));
    let empty = tempfile::tempdir().unwrap();
    assert!(!reuse(empty.path(), &rev));
}

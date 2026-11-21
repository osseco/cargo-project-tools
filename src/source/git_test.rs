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

const URL: &str = "https://example.com/r.git";

/// A clone in `<tmp>/src` as `fresh_clone` leaves it, marker included; returns the commit.
fn our_clone(tmp: &Path) -> (PathBuf, String) {
    let dest = tmp.join("src");
    fs::create_dir(&dest).unwrap();
    let rev = repo(&dest);
    fs::write(marker(&dest), marker_text(URL, &rev)).unwrap();
    (dest, rev)
}

#[test]
fn clone_at_rev_is_reset_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, rev) = our_clone(tmp.path());
    let kept_mtime = fs::metadata(dir.join("kept")).unwrap().modified().unwrap();
    fs::write(dir.join("edited"), "changed").unwrap();
    fs::write(dir.join("untracked"), "x").unwrap();

    assert!(reuse(&dir, URL, &rev));
    assert_eq!(fs::read_to_string(dir.join("edited")).unwrap(), "original");
    assert!(!dir.join("untracked").exists());
    assert_eq!(
        fs::metadata(dir.join("kept")).unwrap().modified().unwrap(),
        kept_mtime
    );
}

#[test]
fn other_or_short_revs_are_not_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, rev) = our_clone(tmp.path());
    assert!(!reuse(&dir, URL, &rev[..12]));
    assert!(!reuse(&dir, URL, &"0".repeat(40)));
    let empty = tempfile::tempdir().unwrap();
    assert!(!reuse(empty.path(), URL, &rev));
}

#[test]
fn only_our_own_clone_is_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, rev) = our_clone(tmp.path());
    // Another url, or no marker (a `.git` that came with a crate archive, say).
    assert!(!reuse(&dir, "https://example.com/other.git", &rev));
    fs::remove_file(marker(&dir)).unwrap();
    assert!(!reuse(&dir, URL, &rev));
}

#[test]
fn a_git_file_is_not_a_clone() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, rev) = our_clone(tmp.path());
    // `gitdir:` would point git at another repository.
    fs::rename(dir.join(".git"), tmp.path().join("elsewhere")).unwrap();
    fs::write(
        dir.join(".git"),
        format!("gitdir: {}\n", tmp.path().join("elsewhere").display()),
    )
    .unwrap();
    assert!(!reuse(&dir, URL, &rev));
}

#[cfg(unix)]
#[test]
fn a_repository_config_runs_no_hooks() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, rev) = our_clone(tmp.path());
    let ran = tmp.path().join("ran");
    let hooks = tmp.path().join("hooks");
    fs::create_dir(&hooks).unwrap();
    run_git(&dir, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    run_git(
        &dir,
        &[
            "config",
            "core.fsmonitor",
            &format!("touch {}", ran.display()),
        ],
    );
    assert!(reuse(&dir, URL, &rev));
    assert!(!ran.exists());
}

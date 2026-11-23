use super::*;

/// The repository git finds from `dir`, resolved like the paths it's compared with: git
/// prints `C:/x` on Windows, where `canonicalize` gives `\\?\C:\x`.
fn toplevel(dir: &Path, ceiling: Option<&Path>) -> Option<PathBuf> {
    let mut cmd = Command::new("git");
    cmd.arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_CEILING_DIRECTORIES");
    if let Some(c) = ceiling {
        git_ceiling(&mut cmd, c).unwrap();
    }
    let out = cmd.output().unwrap();
    out.status.success().then(|| {
        PathBuf::from(String::from_utf8(out.stdout).unwrap().trim())
            .canonicalize()
            .unwrap()
    })
}

#[test]
fn git_ceiling_hides_the_project_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().canonicalize().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let target = project.join(".project-tools/targets/tool");
    let script_dir = target.join("vendor/dep");
    std::fs::create_dir_all(&script_dir).unwrap();

    assert_eq!(toplevel(&script_dir, None), Some(project));
    assert_eq!(toplevel(&script_dir, Some(&target)), None);
}

#[test]
fn git_ceiling_keeps_a_repository_below_it() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().canonicalize().unwrap().join("tool");
    let src = target.join("src");
    std::fs::create_dir_all(&src).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&src)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(toplevel(&src, Some(&target)), Some(src));
}

#[test]
fn verbatim_prefixes_are_dropped() {
    assert_eq!(
        without_verbatim(PathBuf::from(r"\\?\C:\p\targets")),
        PathBuf::from(r"C:\p\targets")
    );
    assert_eq!(
        without_verbatim(PathBuf::from(r"\\?\UNC\host\share\p")),
        PathBuf::from(r"\\host\share\p")
    );
    assert_eq!(
        without_verbatim(PathBuf::from("/home/p/targets")),
        PathBuf::from("/home/p/targets")
    );
}

#[test]
fn a_failed_command_carries_its_exit_code() {
    let mut cmd = Command::new("git");
    cmd.arg("--no-such-option").stderr(Stdio::null());
    let err = run(&mut cmd).unwrap_err();
    let failed = err.downcast_ref::<Failed>().unwrap();
    assert!(failed.code.is_some_and(|c| c != 0), "{failed:?}");
    // The message is what it was before the error had a type.
    assert!(
        err.to_string()
            .starts_with("`git --no-such-option` failed ("),
        "{err}"
    );
}

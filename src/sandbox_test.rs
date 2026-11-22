use super::*;

fn writable(dir: &Path) -> Writable {
    Writable {
        dirs: vec![dir.join("out")],
        files: vec![dir.join("root/.crates.toml")],
    }
}

#[test]
fn the_profile_denies_the_network_and_writes_elsewhere() {
    let w = Writable {
        dirs: vec![PathBuf::from("/p/targets/tool")],
        files: vec![PathBuf::from("/p/.crates.toml"), PathBuf::from("/p/a\"b")],
    };
    let profile = seatbelt_profile(&w);
    assert!(profile.contains("(deny network*)"), "{profile}");
    assert!(profile.contains("(deny file-write*)"), "{profile}");
    assert!(
        profile.contains("(subpath \"/p/targets/tool\")"),
        "{profile}"
    );
    assert!(
        profile.contains("(literal \"/p/.crates.toml\")"),
        "{profile}"
    );
    assert!(profile.contains("(literal \"/p/a\\\"b\")"), "{profile}");
}

#[test]
fn bwrap_binds_only_what_is_writable() {
    let w = Writable {
        dirs: vec![PathBuf::from("/p/targets/tool")],
        files: vec![PathBuf::from("/p/.crates.toml")],
    };
    let args: Vec<String> = bwrap_args(&w)
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let line = args.join(" ");
    assert!(line.starts_with("--ro-bind / / "), "{line}");
    assert!(line.contains("--unshare-net"), "{line}");
    assert!(
        line.contains("--bind /p/targets/tool /p/targets/tool"),
        "{line}"
    );
    assert!(
        line.contains("--bind /p/.crates.toml /p/.crates.toml"),
        "{line}"
    );
}

fn choice(mode: SandboxMode, from_cli: bool) -> Choice {
    Choice {
        mode,
        from_cli,
        line: Some(7),
    }
}

#[test]
fn the_mode_decides_what_a_missing_sandbox_means() {
    let missing: std::result::Result<Kind, String> = Err("bwrap isn't on PATH".into());
    let found = Ok(Kind::SandboxExec);

    assert_eq!(
        decide("t", &choice(SandboxMode::Off, false), &found).unwrap(),
        None
    );
    assert_eq!(
        decide("t", &choice(SandboxMode::Auto, false), &found).unwrap(),
        Some(Kind::SandboxExec)
    );
    // Auto carries on without one; require fails, naming where it was required.
    assert_eq!(
        decide("t", &choice(SandboxMode::Auto, false), &missing).unwrap(),
        None
    );
    let err = decide("t", &choice(SandboxMode::Require, false), &missing)
        .unwrap_err()
        .to_string();
    assert!(err.contains("sandbox = \"require\""), "{err}");
    assert!(err.contains("bwrap isn't on PATH"), "{err}");
    let err = decide("t", &choice(SandboxMode::Require, true), &missing)
        .unwrap_err()
        .to_string();
    assert!(err.contains("--sandbox require"), "{err}");
}

#[test]
fn kinds_have_record_names() {
    assert_eq!(Kind::SandboxExec.name(), "sandbox-exec");
    assert_eq!(Kind::Bwrap(PathBuf::from("/usr/bin/bwrap")).name(), "bwrap");
}

/// A sandboxed `/bin/sh -c script` in `dir`, or `None` where this machine has no sandbox.
fn sandboxed(dir: &Path, script: &str) -> Option<std::process::Output> {
    let kind = available().ok()?;
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", script]).current_dir(dir);
    let mut cmd = wrap(cmd, &kind, &writable(dir), &dir.join("out/tmp")).unwrap();
    Some(cmd.output().unwrap())
}

#[test]
fn a_sandboxed_command_writes_only_where_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    fs::create_dir_all(dir.join("root")).unwrap();
    let Some(out) = sandboxed(
        &dir,
        "echo in > out/file; echo in > root/.crates.toml; echo out > outside; echo tmp > \"$TMPDIR/t\"",
    ) else {
        return;
    };
    // The shell carries on after the refused write.
    assert!(String::from_utf8_lossy(&out.stderr).contains("outside"));
    assert_eq!(fs::read_to_string(dir.join("out/file")).unwrap(), "in\n");
    assert_eq!(
        fs::read_to_string(dir.join("root/.crates.toml")).unwrap(),
        "in\n"
    );
    assert_eq!(fs::read_to_string(dir.join("out/tmp/t")).unwrap(), "tmp\n");
    assert!(!dir.join("outside").exists());
}

#[test]
fn a_sandboxed_command_has_no_network() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    // Only the loopback: nothing leaves the machine even when the sandbox lets it through.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let probe = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");
    // Unsandboxed, bash reaches it, so a failure below is the sandbox's doing.
    let reachable = Command::new("bash")
        .args(["-c", &probe])
        .status()
        .is_ok_and(|s| s.success());
    if !reachable {
        return;
    }
    fs::create_dir_all(dir.join("root")).unwrap();
    let Some(out) = sandboxed(&dir, &format!("bash -c '{probe}'")) else {
        return;
    };
    assert!(!out.status.success());
}

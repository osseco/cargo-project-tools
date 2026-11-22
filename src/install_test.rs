use super::*;
use clap::Parser;

fn install_args(extra: &[&str]) -> BuildArgs {
    let mut argv = vec!["cargo-project-tools", "install"];
    argv.extend(extra);
    match crate::cli::Cli::parse_from(argv).command {
        crate::cli::Command::Install(a) => a.build,
        _ => unreachable!(),
    }
}

fn tool() -> Tool {
    toml::from_str("version = \"1.0.0\"\nfeatures = [\"a\"]\ndefault-features = false\n").unwrap()
}

#[test]
fn build_settings_become_config() {
    let build: Build = toml::from_str(
        "profile = \"release\"\nrustflags = [\"-Ctarget-cpu=x86-64-v2\"]\n[profile-settings]\nopt-level = \"z\"\ncodegen-units = 1\n",
    )
    .unwrap();
    let args = cargo_args(&tool(), &build, &install_args(&["-j", "4"]));
    let s = args.join(" ");
    assert!(s.contains("--features a --no-default-features"));
    assert!(s.contains("--profile release"));
    assert!(s.contains("--config profile.release.opt-level=\"z\""));
    assert!(s.contains("--config profile.release.codegen-units=1"));
    assert!(s.contains("--config build.rustflags=[\"-Ctarget-cpu=x86-64-v2\"]"));
    assert!(s.contains("--jobs 4"));
}

#[test]
fn feature_selection_args() {
    assert_eq!(
        feature_args(&tool()),
        ["--features", "a", "--no-default-features"]
    );
    let plain: Tool = toml::from_str("version = \"1.0.0\"\n").unwrap();
    assert!(feature_args(&plain).is_empty());
}

#[test]
fn stamp_ignores_noise_flags() {
    let build = Build::default();
    let a = cargo_args(&tool(), &build, &install_args(&[]));
    let b = cargo_args(
        &tool(),
        &build,
        &install_args(&["-j", "8", "-v", "--offline"]),
    );
    let c = cargo_args(&tool(), &build, &install_args(&["--debug"]));
    assert_eq!(
        stamp_key("t", &tool(), "lock", "", &a, None),
        stamp_key("t", &tool(), "lock", "", &b, None)
    );
    assert_ne!(
        stamp_key("t", &tool(), "lock", "", &a, None),
        stamp_key("t", &tool(), "lock", "", &c, None)
    );
    assert_ne!(
        stamp_key("t", &tool(), "lock", "", &a, None),
        stamp_key("t", &tool(), "lock2", "", &a, None)
    );
    assert_ne!(
        stamp_key("t", &tool(), "lock", "", &a, None),
        stamp_key("t", &tool(), "lock", "review", &a, None)
    );
}

#[test]
fn stamp_depends_on_platform() {
    let args = cargo_args(&tool(), &Build::default(), &install_args(&[]));
    let key = |os, arch| stamp_key_on("t", &tool(), "lock", "", &args, None, os, arch);
    assert_ne!(key("macos", "x86_64"), key("macos", "aarch64"));
    assert_ne!(key("linux", "x86_64"), key("macos", "x86_64"));
}

#[test]
fn the_review_resolves_with_the_builds_flags_and_config() {
    let build: Build = toml::from_str(
        "profile = \"release\"\nrustflags = [\"--cfg\", \"tokio_unstable\"]\n[profile-settings]\nopt-level = \"z\"\n",
    )
    .unwrap();
    let resolve = resolve_config(&build, &["net.offline=true".to_string()]);
    assert_eq!(
        resolve,
        [
            "--config",
            "build.rustflags=[\"--cfg\", \"tokio_unstable\"]",
            "--config",
            "net.offline=true",
        ]
    );
    // And the build passes the same.
    let args = cargo_args(
        &tool(),
        &build,
        &install_args(&["--config", "net.offline=true"]),
    )
    .join(" ");
    assert!(args.contains(&resolve.join(" ")), "{args}");
}

#[test]
fn stamp_depends_on_targets() {
    let args = cargo_args(&tool(), &Build::default(), &install_args(&[]));
    let linux = ["x86_64-unknown-linux-gnu".to_string()];
    let both = [linux[0].clone(), "aarch64-apple-darwin".to_string()];
    let key = |t: Option<&[String]>| stamp_key("t", &tool(), "lock", "", &args, t);
    assert_ne!(key(Some(&linux)), key(Some(&both)));
    assert_ne!(key(Some(&linux)), key(None));
}

#[test]
fn the_build_is_offline_and_puts_bin_last_on_path() {
    let tmp = tempfile::tempdir().unwrap();
    let project = Project {
        root: tmp.path().to_path_buf(),
    };
    let target = tmp.path().join("t");
    fs::create_dir_all(&target).unwrap();
    let cmd = build_command(&project, &target, tmp.path(), &[], &[]).unwrap();
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args.iter().filter(|a| *a == "--offline").count(), 1);
    let path = cmd
        .get_envs()
        .find(|(k, _)| *k == "PATH")
        .and_then(|(_, v)| v)
        .unwrap();
    assert_eq!(std::env::split_paths(path).last(), Some(project.bin_dir()));

    // Not twice when the user passed it.
    let cmd = build_command(&project, &target, tmp.path(), &[], &["--offline".into()]).unwrap();
    assert_eq!(cmd.get_args().filter(|a| *a == "--offline").count(), 1);
}

#[test]
fn the_sandbox_choice_names_the_setting_that_applied() {
    let tmp = tempfile::tempdir().unwrap();
    let project = Project {
        root: tmp.path().to_path_buf(),
    };
    let os = std::env::consts::OS;
    let text = format!(
        "[build]\nsandbox = \"auto\"\n\n[build.sandbox-os]\n{os} = \"require\"\n\n[tool.t]\nversion = \"1.0.0\"\n"
    );
    fs::write(project.config_path(), &text).unwrap();
    let config: crate::config::Config = toml::from_str(&text).unwrap();
    let tool = &config.tool["t"];
    let build = config.build.merged(tool.build.as_ref());

    let c = sandbox_choice(&project, "t", tool, &build, &install_args(&[]));
    assert_eq!(c.mode, SandboxMode::Require);
    assert_eq!(c.line, Some(5));
    assert!(!c.from_cli);

    let c = sandbox_choice(
        &project,
        "t",
        tool,
        &build,
        &install_args(&["--sandbox", "off"]),
    );
    assert_eq!((c.mode, c.from_cli, c.line), (SandboxMode::Off, true, None));
    let c = sandbox_choice(
        &project,
        "t",
        tool,
        &build,
        &install_args(&["--no-sandbox"]),
    );
    assert_eq!(c.mode, SandboxMode::Off);
    assert!(
        crate::cli::Cli::try_parse_from([
            "cargo-project-tools",
            "install",
            "--sandbox",
            "auto",
            "--no-sandbox"
        ])
        .is_err()
    );
}

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
        stamp_key("t", &tool(), "lock", &a),
        stamp_key("t", &tool(), "lock", &b)
    );
    assert_ne!(
        stamp_key("t", &tool(), "lock", &a),
        stamp_key("t", &tool(), "lock", &c)
    );
    assert_ne!(
        stamp_key("t", &tool(), "lock", &a),
        stamp_key("t", &tool(), "lock2", &a)
    );
}

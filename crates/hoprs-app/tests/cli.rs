//! The `hoprs` binary's argument handling, end to end: these paths exit before the
//! instance lock and the window.

use std::process::Command;

fn hoprs(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hoprs"))
        .args(args)
        .output()
        .expect("run hoprs")
}

#[test]
fn version_prints_name_and_version() {
    for flag in ["--version", "-V"] {
        let out = hoprs(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("hoprs {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = hoprs(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("Usage: hoprs"));
}

#[test]
fn unknown_argument_exits_two() {
    let out = hoprs(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bogus"));
}

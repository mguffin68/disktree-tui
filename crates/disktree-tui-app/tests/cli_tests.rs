//! Integration tests for CLI argument parsing.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_disktree-tui");

fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to execute binary")
}

#[test]
fn default_no_args_shows_help() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("disktree-tui"));
    assert!(stdout.contains("DIRECTORY"));
}

#[test]
fn version_flag_prints_version() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0.1.0"));
}

#[test]
fn scan_only_exits_cleanly() {
    let output = run(&["--scan-only"]);
    assert!(output.status.success());
}

#[test]
fn apparent_flag_is_accepted() {
    let output = run(&["--scan-only", "--apparent"]);
    assert!(output.status.success());
}

#[test]
fn no_hidden_flag_is_accepted() {
    let output = run(&["--scan-only", "--no-hidden"]);
    assert!(output.status.success());
}

#[test]
fn depth_flag_is_accepted() {
    let output = run(&["--scan-only", "-D", "3"]);
    assert!(output.status.success());
}

#[test]
fn mode_flag_accepts_valid_modes() {
    assert!(run(&["--scan-only", "--mode", "size"]).status.success());
    assert!(run(&["--scan-only", "--mode", "files"]).status.success());
    assert!(run(&["--scan-only", "--mode", "age"]).status.success());
}

#[test]
fn mode_flag_rejects_invalid_mode() {
    let output = run(&["--scan-only", "--mode", "invalid"]);
    assert!(!output.status.success());
}

#[test]
fn disk_flag_is_accepted() {
    let output = run(&["--scan-only", "--disk"]);
    assert!(output.status.success());
}

#[test]
fn directory_positional_arg_is_accepted() {
    let output = run(&["--scan-only", "/tmp"]);
    assert!(output.status.success());
}

#[test]
fn all_flags_together() {
    let output = run(&[
        "--scan-only",
        "--disk",
        "--apparent",
        "--no-hidden",
        "-D",
        "5",
        "--mode",
        "size",
        "/tmp",
    ]);
    assert!(output.status.success());
}

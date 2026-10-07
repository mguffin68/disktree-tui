//! Integration tests for CLI argument parsing.

use std::process::Command;

use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_disktree-tui");

/// Small fixture tree so `--scan-only` tests never scan real directories.
fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("big/inner")).unwrap();
    std::fs::write(tmp.path().join("big/inner/data.bin"), vec![0u8; 4096]).unwrap();
    std::fs::create_dir(tmp.path().join("small")).unwrap();
    std::fs::write(tmp.path().join("small/note.txt"), b"hi").unwrap();
    std::fs::write(tmp.path().join(".hidden"), vec![0u8; 100]).unwrap();
    tmp
}

/// Run `--scan-only` against a fresh fixture with extra args.
fn scan_fixture(args: &[&str]) -> std::process::Output {
    let tmp = fixture();
    let dir = tmp.path().to_str().unwrap();
    let mut full = vec!["--scan-only"];
    full.extend_from_slice(args);
    full.push(dir);
    run(&full)
}

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
    let output = scan_fixture(&[]);
    assert!(output.status.success());
}

#[test]
fn apparent_flag_is_accepted() {
    let output = scan_fixture(&["--apparent"]);
    assert!(output.status.success());
}

#[test]
fn no_hidden_flag_is_accepted() {
    let output = scan_fixture(&["--no-hidden"]);
    assert!(output.status.success());
}

#[test]
fn depth_flag_is_accepted() {
    let output = scan_fixture(&["-D", "3"]);
    assert!(output.status.success());
}

#[test]
fn mode_flag_accepts_valid_modes() {
    assert!(scan_fixture(&["--mode", "size"]).status.success());
    assert!(scan_fixture(&["--mode", "files"]).status.success());
    assert!(scan_fixture(&["--mode", "age"]).status.success());
}

#[test]
fn mode_flag_rejects_invalid_mode() {
    let output = scan_fixture(&["--mode", "invalid"]);
    assert!(!output.status.success());
}

#[test]
fn disk_flag_is_parsed_but_not_yet_supported() {
    // Whole-disk scanning arrives with the volume picker; it must not scan `/` here.
    let output = scan_fixture(&["--disk"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--disk"));
}

#[test]
fn directory_positional_arg_is_accepted() {
    let output = scan_fixture(&[]);
    assert!(output.status.success());
}

#[test]
fn scan_only_prints_summary() {
    let output = scan_fixture(&["--apparent"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("4.1 KiB (apparent)"), "{stdout}");
    assert!(stdout.contains("3 files"), "{stdout}");
    assert!(stdout.contains("big/"), "{stdout}");
    assert!(stdout.contains("small/"), "{stdout}");
    // Default summary depth is 1.
    assert!(!stdout.contains("inner/"), "{stdout}");
    // Largest directory listed first.
    assert!(stdout.find("big/") < stdout.find("small/"), "{stdout}");
}

#[test]
fn scan_only_respects_depth_and_hidden() {
    let output = scan_fixture(&["--apparent", "--no-hidden", "-D", "2"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("inner/"), "{stdout}");
    assert!(stdout.contains("2 files"), "{stdout}");
}

#[test]
fn scan_only_missing_directory_fails() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("missing");
    let output = run(&["--scan-only", missing.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot scan"));
}

#[test]
fn all_flags_together() {
    let output = scan_fixture(&["--apparent", "--no-hidden", "-D", "5", "--mode", "size"]);
    assert!(output.status.success());
}

//! End-to-end tests for bombadil using the examples directory.
//!
//! These tests run inside a container for complete isolation from the host system.
//! Uses host-compiled binary mounted into a lightweight runtime container for speed.
//!
//! Prerequisites:
//!   cargo build --release
//!   podman build -t bombadil-e2e-runtime --target runtime -f examples/e2e/Containerfile .

use std::env;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Once;

use speculoos::prelude::*;

static INIT: Once = Once::new();

/// Get the repository root directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Detect available container runtime (podman preferred, docker fallback).
fn container_runtime() -> &'static str {
    if Command::new("podman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "podman"
    } else {
        "docker"
    }
}

/// Find the host binary (release preferred, debug fallback).
fn host_binary() -> PathBuf {
    let root = repo_root();
    let release = root.join("target/release/bombadil");
    if release.exists() {
        return release;
    }
    let debug = root.join("target/debug/bombadil");
    if debug.exists() {
        return debug;
    }
    panic!(
        "No bombadil binary found. Run `cargo build --release` first.\n\
         Looked in:\n  {}\n  {}",
        release.display(),
        debug.display()
    );
}

/// Build the runtime container image if needed.
fn ensure_runtime_image() {
    INIT.call_once(|| {
        let runtime = container_runtime();
        let image_name = "bombadil-e2e-runtime";

        // Check if image exists
        let check = Command::new(runtime)
            .args(["image", "inspect", image_name])
            .output()
            .expect("Failed to check image");

        if !check.status.success() {
            let status = Command::new(runtime)
                .args([
                    "build",
                    "-t",
                    image_name,
                    "--target",
                    "runtime",
                    "-f",
                    "examples/e2e/Containerfile",
                    ".",
                ])
                .current_dir(repo_root())
                .status()
                .expect("Failed to build runtime image");

            assert!(status.success(), "Failed to build runtime container image");
        }
    });
}

/// Run a command inside a fresh container with host binary mounted.
fn run_in_container(script: &str) -> Output {
    ensure_runtime_image();

    let runtime = container_runtime();
    let repo = repo_root();
    let binary = host_binary();

    Command::new(runtime)
        .args([
            "run",
            "--rm",
            "--security-opt",
            "label=disable",
            "-v",
            &format!("{}:/usr/local/bin/bombadil:ro", binary.display()),
            "-v",
            &format!("{}:/examples:ro", repo.join("examples").display()),
            "-w",
            "/home/testuser",
            "bombadil-e2e-runtime",
            "bash",
            "-c",
            script,
        ])
        .output()
        .expect("Failed to run container")
}

/// Standard setup script that copies examples and runs bombadil install + link.
fn setup_and_link_script(profile: Option<&str>, extra_setup: &str) -> String {
    let profile_arg = profile
        .map(|p| format!("--profiles {}", p))
        .unwrap_or_default();

    format!(
        r#"
        set -e
        cp -r /examples/base/. ~/ 2>/dev/null || true
        cp -r /examples/dotfiles ~/dotfiles
        mkdir -p ~/dotfiles/.dots ~/.config ~/.local/bin
        {extra_setup}
        bombadil install ~/dotfiles 2>/dev/null
        bombadil link --force {profile_arg} 2>&1
        "#,
    )
}

/// Run bombadil link in container and return output.
fn run_bombadil_in_container(profile: Option<&str>, extra_setup: &str) -> Output {
    run_in_container(&setup_and_link_script(profile, extra_setup))
}

/// Check if a file exists after bombadil link.
fn file_exists_after_link(path: &str, profile: Option<&str>) -> bool {
    let setup = setup_and_link_script(profile, "");
    let script = format!("{setup}\ntest -e {path} && echo EXISTS || echo MISSING");
    let output = run_in_container(&script);
    String::from_utf8_lossy(&output.stdout).contains("EXISTS")
}

/// Check if a path is a symlink after bombadil link.
fn is_symlink_after_link(path: &str, profile: Option<&str>) -> bool {
    let setup = setup_and_link_script(profile, "");
    let script = format!("{setup}\ntest -L {path} && echo SYMLINK || echo NOT_SYMLINK");
    let output = run_in_container(&script);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim().ends_with("SYMLINK") && !stdout.contains("NOT_SYMLINK")
}

/// Read file content after bombadil link.
fn read_file_after_link(path: &str, profile: Option<&str>) -> String {
    let setup = setup_and_link_script(profile, "");
    let script = format!("{setup}\ncat {path}");
    let output = run_in_container(&script);
    String::from_utf8_lossy(&output.stdout).to_string()
}

// =============================================================================
// Full Strategy Tests
// =============================================================================

#[test]
fn test_full_strategy_simple_file() {
    let output = run_bombadil_in_container(None, "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "bombadil link failed:\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    assert!(file_exists_after_link(".config/full-simple/config.conf", None));
    assert!(is_symlink_after_link(".config/full-simple/config.conf", None));

    let content = read_file_after_link(".config/full-simple/config.conf", None);
    assert_that(&content).contains("setting_one = \"new_value\"");
}

#[test]
fn test_full_strategy_directory() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(file_exists_after_link(".config/full-directory/file1.conf", None));
    assert!(file_exists_after_link(".config/full-directory/file2.conf", None));
    assert!(file_exists_after_link(
        ".config/full-directory/subdir/nested.conf",
        None
    ));
}

// =============================================================================
// Templating Tests
// =============================================================================

#[test]
fn test_variable_substitution() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    let content = read_file_after_link(".config/full-simple/templated.conf", None);
    assert_that(&content).contains("editor = \"nvim\"");
    assert_that(&content).contains("terminal = \"alacritty\"");
    assert_that(&content).contains("background = \"#1a1b26\"");
}

#[test]
fn test_local_vars_override() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    let content = read_file_after_link(".config/full-simple/templated.conf", None);
    // font_size should be 16 from local-vars.toml (overrides base 14)
    assert_that(&content).contains("font_size = 16");
    assert_that(&content).contains("local_setting = \"this_is_local\"");
}

#[test]
fn test_platform_context_injection() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    let content = read_file_after_link(".config/full-simple/templated.conf", None);
    // Container runs Linux
    assert_that(&content).contains("platform_specific = \"linux_value\"");
}

// =============================================================================
// Ignore Patterns Tests
// =============================================================================

#[test]
fn test_ignore_patterns_links_non_ignored() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(file_exists_after_link(".config/ignore-demo/config.conf", None));
}

#[test]
fn test_ignore_patterns_excludes_tmp() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(!file_exists_after_link(
        ".config/ignore-demo/temp.tmp",
        None
    ));
}

#[test]
fn test_ignore_patterns_excludes_bak() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(!file_exists_after_link(
        ".config/ignore-demo/backup.bak",
        None
    ));
}

#[test]
fn test_ignore_patterns_excludes_cache_contents() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(!file_exists_after_link(
        ".config/ignore-demo/cache/cached.dat",
        None
    ));
}

// =============================================================================
// Import Tests
// =============================================================================

#[test]
fn test_imports_dots_from_imported_file() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(file_exists_after_link(".config/imported/config.conf", None));

    let content = read_file_after_link(".config/imported/config.conf", None);
    assert_that(&content).contains("imported = true");
}

// =============================================================================
// Hard Copy Tests
// =============================================================================

#[test]
fn test_hard_copy_creates_file() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(file_exists_after_link(".local/bin/my-script", None));
}

#[test]
fn test_hard_copy_not_symlink() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    assert!(!is_symlink_after_link(".local/bin/my-script", None));
}

#[test]
fn test_hard_copy_has_variable_substitution() {
    let output = run_bombadil_in_container(None, "");
    assert!(output.status.success());

    let content = read_file_after_link(".local/bin/my-script", None);
    assert_that(&content).contains("echo \"Editor: nvim\"");
}

#[test]
fn test_hard_copy_permissions() {
    let setup = setup_and_link_script(None, "");
    let script = format!("{setup}\nstat -c \"%a\" .local/bin/my-script");
    let output = run_in_container(&script);
    let perms = String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    assert_eq!(perms, "755", "Expected 755 permissions, got {}", perms);
}

// =============================================================================
// Hooks Tests
// =============================================================================

#[test]
fn test_hooks_run() {
    let output = run_bombadil_in_container(None, "");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("pre-install hook") || combined.contains("post-install hook"),
        "Hook output not found in: {}",
        combined
    );
}

// =============================================================================
// Profile Tests
// =============================================================================

#[test]
fn test_profile_laptop_specific_file() {
    let output = run_bombadil_in_container(Some("laptop"), "");
    assert!(output.status.success());

    assert!(file_exists_after_link(
        ".config/profile-specific/laptop.conf",
        Some("laptop")
    ));

    let content = read_file_after_link(".config/profile-specific/laptop.conf", Some("laptop"));
    assert_that(&content).contains("battery_warning = 20");
}

#[test]
fn test_profile_work_inherits_laptop() {
    let output = run_bombadil_in_container(Some("work"), "");
    assert!(output.status.success());

    // Work profile uses extra_profiles = ["laptop"]
    assert!(file_exists_after_link(
        ".config/profile-specific/laptop.conf",
        Some("work")
    ));
    assert!(file_exists_after_link(
        ".config/profile-specific/work.conf",
        Some("work")
    ));
}

#[test]
fn test_profile_work_source_override() {
    let output = run_bombadil_in_container(Some("work"), "");
    assert!(output.status.success());

    let content = read_file_after_link(".config/full-simple/config.conf", Some("work"));
    assert_that(&content).contains("work_specific = \"only_at_work\"");
}

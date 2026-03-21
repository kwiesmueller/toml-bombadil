use anyhow::{anyhow, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tracing::{debug, info, warn};

/// Result of running a hook.
#[derive(Debug)]
pub struct HookResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
}

impl HookResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Hook {
    pub command: String,
    pub dotfiles_path: PathBuf,
    pub run_in_dotfiles_dir: bool,
}

const ENV_BOMBADIL_DOTFILES_PATH: &str = "BOMBADIL_DOTFILES_PATH";

impl Hook {
    /// Run the hook and capture output without printing.
    pub(crate) fn run_capture(&self) -> Result<HookResult> {
        debug!(command = %self.command, "Running hook");

        let mut child = Command::new("sh");
        let mut child = child
            .args(["-c", &self.command])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .env(ENV_BOMBADIL_DOTFILES_PATH, self.dotfiles_path.as_os_str());

        if self.run_in_dotfiles_dir {
            child = child.current_dir(&self.dotfiles_path);
        }

        let mut child = child.spawn()?;

        let stdout: Vec<String> = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .filter_map(|line| line.ok())
            .collect();

        let stderr: Vec<String> = BufReader::new(child.stderr.take().unwrap())
            .lines()
            .filter_map(|line| line.ok())
            .collect();

        let exit_code = child.wait()?.code().unwrap_or(-1);

        if exit_code == 0 {
            info!(command = %self.command, "Hook completed successfully");
        } else {
            warn!(command = %self.command, exit_code, "Hook failed");
        }

        Ok(HookResult {
            command: self.command.clone(),
            exit_code,
            stdout,
            stderr,
        })
    }

    /// Run the hook with legacy output (for backwards compatibility).
    pub(crate) fn run(&self) -> Result<()> {
        let command_display = format!("`{}`", &self.command.green());
        println!("Running install hook : {}", command_display);

        let mut child = Command::new("sh");
        let mut child = child
            .args(["-c", &self.command])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .env(ENV_BOMBADIL_DOTFILES_PATH, self.dotfiles_path.as_os_str());

        if self.run_in_dotfiles_dir {
            child = child.current_dir(&self.dotfiles_path);
        }

        let mut child = child.spawn()?;

        BufReader::new(child.stdout.take().unwrap())
            .lines()
            .for_each(|line| {
                println!(
                    "[{}] {}",
                    command_display,
                    line.unwrap_or_else(|_| "".into())
                )
            });

        BufReader::new(child.stderr.take().unwrap())
            .lines()
            .for_each(|line| {
                eprintln!(
                    "[{}] {}",
                    command_display,
                    line.unwrap_or_else(|_| "".into())
                )
            });

        child
            .wait()
            .map(|exit_code| {
                if exit_code.success() {
                    Ok(())
                } else {
                    Err(anyhow!("Hook run failed with status {}", exit_code))
                }
            })
            .unwrap()
    }

    pub fn new(dotfiles_path: PathBuf, command: &str, run_in_dotfiles_dir: bool) -> Self {
        let command = command.to_owned();
        Hook {
            dotfiles_path,
            command,
            run_in_dotfiles_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::hook::Hook;
    use sealed_test::prelude::tempfile::tempdir;
    use speculoos::prelude::*;

    #[test]
    fn should_run_command() {
        // Arrange
        let hook = Hook {
            command: "echo hello world".to_string(),
            dotfiles_path: PathBuf::from("/tmp"),
            run_in_dotfiles_dir: false,
        };

        // Act
        let result = hook.run();

        // Assert
        assert_that!(result).is_ok();
    }

    #[test]
    fn should_fail_to_run_invalid_command() {
        // Arrange
        let hook = Hook {
            command: "azmroih".to_string(),
            dotfiles_path: PathBuf::from("/tmp"),
            run_in_dotfiles_dir: false,
        };

        // Act
        let result = hook.run();

        // Assert
        assert_that!(result).is_err();
    }

    #[test]
    fn should_set_dotfiles_path_env() {
        let temp_dir = tempdir().expect("a temp dir is created");

        const TEST_FILE: &str = "test_bombadil_env";

        // Arrange
        let hook = Hook {
            command: "touch ${BOMBADIL_DOTFILES_PATH}/".to_string() + TEST_FILE,
            dotfiles_path: PathBuf::from(temp_dir.path()),
            run_in_dotfiles_dir: false,
        };

        // Act
        let result = hook.run();

        // Assert
        assert_that!(result).is_ok();
        assert_that!(temp_dir.path().join(TEST_FILE)).exists();
    }

    #[test]
    fn should_run_hooks_in_dotfiles_path() {
        let temp_dir = tempdir().expect("a temp dir is created");

        const TEST_FILE: &str = "test_bombadil_env";

        // Arrange
        let hook = Hook {
            command: "touch ".to_string() + TEST_FILE,
            dotfiles_path: PathBuf::from(temp_dir.path()),
            run_in_dotfiles_dir: true,
        };

        // Act
        let result = hook.run();

        // Assert
        assert_that!(result).is_ok();
        assert_that!(temp_dir.path().join(TEST_FILE)).exists();
    }
}

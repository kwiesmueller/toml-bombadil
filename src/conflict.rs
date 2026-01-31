use crate::settings::dotfile_dir;
use anyhow::{anyhow, Result};
use colored::*;
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Strategy for handling conflicts when target files differ from dotfiles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictStrategy {
    /// Prompt user for each conflict (default)
    #[default]
    Interactive,
    /// Always use dotfile, backup system file
    DotfileWins,
    /// Always use system file, update dotfile source
    SystemWins,
    /// Skip all conflicts
    Skip,
}

impl std::str::FromStr for ConflictStrategy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "interactive" => Ok(ConflictStrategy::Interactive),
            "dotfile-wins" | "dotfile" => Ok(ConflictStrategy::DotfileWins),
            "system-wins" | "system" => Ok(ConflictStrategy::SystemWins),
            "skip" => Ok(ConflictStrategy::Skip),
            _ => Err(format!("Unknown strategy: {}", s)),
        }
    }
}

/// Resolution chosen for a single conflict
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Use the dotfile version, overwrite system
    UseDotfile,
    /// Use the system version, update dotfile source
    UseSystem,
    /// Skip this file, don't change anything
    Skip,
    /// Apply this resolution to all remaining conflicts
    UseDotfileForAll,
    UseSystemForAll,
    SkipAll,
}

/// Represents a conflict between a dotfile and an existing system file
#[derive(Debug)]
pub struct Conflict {
    /// Name of the dot entry
    pub dot_name: String,
    /// Path to the target (system) file
    pub target_path: PathBuf,
    /// Path to the source file in dotfiles repo
    pub source_path: PathBuf,
    /// Path to the rendered file in .dots/
    pub rendered_path: PathBuf,
    /// Content of the rendered dotfile
    pub dotfile_content: String,
    /// Content of the existing system file
    pub system_content: String,
}

impl Conflict {
    /// Detect if there's a conflict between the rendered dotfile and the target
    pub fn detect(
        dot_name: &str,
        rendered_path: &Path,
        target_path: &Path,
        source_path: &Path,
    ) -> Result<Option<Self>> {
        // If target doesn't exist, no conflict
        if !target_path.exists() {
            return Ok(None);
        }

        // If target is already a symlink pointing to our .dots/, no conflict
        if target_path.is_symlink() {
            if let Ok(link_target) = target_path.canonicalize() {
                let dots_dir = dotfile_dir().join(".dots");
                if link_target.starts_with(&dots_dir) {
                    return Ok(None);
                }
            }
        }

        // Check if target is a directory - handle differently
        if target_path.is_dir() && !target_path.is_symlink() {
            // For directories, we'd need recursive conflict detection
            // For now, treat as conflict
            return Ok(Some(Conflict {
                dot_name: dot_name.to_string(),
                target_path: target_path.to_path_buf(),
                source_path: source_path.to_path_buf(),
                rendered_path: rendered_path.to_path_buf(),
                dotfile_content: "[directory]".to_string(),
                system_content: "[directory]".to_string(),
            }));
        }

        // Read both files
        let dotfile_content = fs::read_to_string(rendered_path).unwrap_or_else(|_| String::new());
        let system_content = fs::read_to_string(target_path).unwrap_or_else(|_| String::new());

        // If contents are identical, no conflict
        if dotfile_content == system_content {
            return Ok(None);
        }

        Ok(Some(Conflict {
            dot_name: dot_name.to_string(),
            target_path: target_path.to_path_buf(),
            source_path: source_path.to_path_buf(),
            rendered_path: rendered_path.to_path_buf(),
            dotfile_content,
            system_content,
        }))
    }

    /// Generate a unified diff between dotfile and system file
    pub fn generate_diff(&self) -> String {
        let diff = TextDiff::from_lines(&self.dotfile_content, &self.system_content);
        let mut output = String::new();

        for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
            if idx > 0 {
                output.push_str("...\n");
            }
            for op in group {
                for change in diff.iter_changes(op) {
                    let (sign, line) = match change.tag() {
                        ChangeTag::Delete => ("-".red(), change.value().red()),
                        ChangeTag::Insert => ("+".green(), change.value().green()),
                        ChangeTag::Equal => (" ".normal(), change.value().normal()),
                    };
                    output.push_str(&format!("{}{}", sign, line));
                    if change.missing_newline() {
                        output.push('\n');
                    }
                }
            }
        }

        output
    }

    /// Show diff header
    pub fn print_header(&self) {
        println!();
        println!("{}", "═".repeat(60).yellow());
        println!(
            "{} {}",
            "Conflict:".yellow().bold(),
            self.target_path.display()
        );
        println!("{}", "─".repeat(60).yellow());
        println!(
            "  {} {} (dotfile)",
            "---".red(),
            self.rendered_path.display()
        );
        println!(
            "  {} {} (system)",
            "+++".green(),
            self.target_path.display()
        );
        println!("{}", "─".repeat(60).yellow());
    }

    /// Prompt user for resolution
    pub fn prompt_user(&self) -> Result<ConflictResolution> {
        self.print_header();
        println!("{}", self.generate_diff());
        println!("{}", "─".repeat(60).yellow());
        println!();
        println!("Choose action:");
        println!("  {} Use dotfile (overwrite system)", "[d]".cyan());
        println!("  {} Use system (update dotfile source)", "[s]".cyan());
        println!("  {} Open merge tool ($EDITOR or vimdiff)", "[m]".cyan());
        println!("  {} Skip this file", "[k]".cyan());
        println!();
        println!(
            "  {} Use dotfile for ALL remaining conflicts",
            "[D]".cyan().bold()
        );
        println!(
            "  {} Use system for ALL remaining conflicts",
            "[S]".cyan().bold()
        );
        println!("  {} Skip ALL remaining conflicts", "[K]".cyan().bold());
        println!();
        print!("{} ", ">".green().bold());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;

        match input.trim() {
            "d" => Ok(ConflictResolution::UseDotfile),
            "s" => Ok(ConflictResolution::UseSystem),
            "m" => {
                self.open_merge_tool()?;
                // After merge, ask again
                self.prompt_user()
            }
            "k" => Ok(ConflictResolution::Skip),
            "D" => Ok(ConflictResolution::UseDotfileForAll),
            "S" => Ok(ConflictResolution::UseSystemForAll),
            "K" => Ok(ConflictResolution::SkipAll),
            _ => {
                println!("{}", "Invalid choice, please try again.".red());
                self.prompt_user()
            }
        }
    }

    /// Open a merge tool for manual resolution
    pub fn open_merge_tool(&self) -> Result<()> {
        // Try $EDITOR first, then vimdiff, then vim -d
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vimdiff".to_string());

        println!("{}", format!("Opening merge tool: {} ...", editor).yellow());

        // Create a temp file with system content to edit
        let temp_path = self.target_path.with_extension("conflict-merge");
        fs::copy(&self.target_path, &temp_path)?;

        let status = if editor.contains("vim") || editor == "vimdiff" {
            Command::new("vimdiff")
                .arg(&self.rendered_path)
                .arg(&temp_path)
                .status()
        } else if editor.contains("code") {
            Command::new(&editor)
                .arg("--wait")
                .arg("--diff")
                .arg(&self.rendered_path)
                .arg(&temp_path)
                .status()
        } else {
            Command::new(&editor)
                .arg(&self.rendered_path)
                .arg(&temp_path)
                .status()
        };

        match status {
            Ok(s) if s.success() => {
                println!(
                    "{}",
                    "Merge tool closed. Review changes and select an action.".green()
                );
            }
            Ok(_) => {
                println!("{}", "Merge tool exited with error.".yellow());
            }
            Err(e) => {
                println!("{}: {}", "Failed to open merge tool".red(), e);
            }
        }

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);

        Ok(())
    }

    /// Apply the "use system" resolution - copy system content back to dotfile source
    pub fn apply_use_system(&self) -> Result<()> {
        println!(
            "  {} Updating dotfile source: {}",
            "→".blue(),
            self.source_path.display()
        );

        // Copy system file content to the source file in dotfiles repo
        fs::copy(&self.target_path, &self.source_path)
            .map_err(|e| anyhow!("Failed to copy system file to dotfile source: {}", e))?;

        Ok(())
    }
}

/// Context for tracking conflict resolution state across multiple conflicts
#[derive(Default)]
pub struct ConflictContext {
    /// Current strategy
    pub strategy: ConflictStrategy,
    /// Sticky resolution (from "for all" choices)
    pub sticky_resolution: Option<ConflictResolution>,
    /// Count of conflicts resolved
    pub resolved_count: usize,
    /// Count of conflicts skipped
    pub skipped_count: usize,
}

impl ConflictContext {
    pub fn new(strategy: ConflictStrategy) -> Self {
        Self {
            strategy,
            sticky_resolution: None,
            resolved_count: 0,
            skipped_count: 0,
        }
    }

    /// Resolve a conflict according to current strategy/context
    pub fn resolve(&mut self, conflict: &Conflict) -> Result<ConflictResolution> {
        // Check for sticky resolution first
        if let Some(ref sticky) = self.sticky_resolution {
            return Ok(match sticky {
                ConflictResolution::UseDotfileForAll => ConflictResolution::UseDotfile,
                ConflictResolution::UseSystemForAll => ConflictResolution::UseSystem,
                ConflictResolution::SkipAll => ConflictResolution::Skip,
                other => other.clone(),
            });
        }

        let resolution = match self.strategy {
            ConflictStrategy::Interactive => conflict.prompt_user()?,
            ConflictStrategy::DotfileWins => ConflictResolution::UseDotfile,
            ConflictStrategy::SystemWins => ConflictResolution::UseSystem,
            ConflictStrategy::Skip => ConflictResolution::Skip,
        };

        // Update sticky if "for all" was chosen
        match &resolution {
            ConflictResolution::UseDotfileForAll
            | ConflictResolution::UseSystemForAll
            | ConflictResolution::SkipAll => {
                self.sticky_resolution = Some(resolution.clone());
            }
            _ => {}
        }

        // Update counts
        match &resolution {
            ConflictResolution::Skip | ConflictResolution::SkipAll => {
                self.skipped_count += 1;
            }
            _ => {
                self.resolved_count += 1;
            }
        }

        Ok(resolution)
    }

    /// Print summary at the end
    pub fn print_summary(&self) {
        if self.resolved_count > 0 || self.skipped_count > 0 {
            println!();
            println!("{}", "─".repeat(40));
            println!(
                "Conflicts: {} resolved, {} skipped",
                self.resolved_count.to_string().green(),
                self.skipped_count.to_string().yellow()
            );
        }
    }
}

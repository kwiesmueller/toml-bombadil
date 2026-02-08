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
    /// Detect if there's a conflict between the rendered dotfile and the target.
    ///
    /// Backward-compatible 4-argument version that uses the global `dotfile_dir()`
    /// to locate the `.dots` directory. Prefer `detect_in()` for new code that has
    /// an explicit dotfiles directory available.
    pub fn detect(
        dot_name: &str,
        rendered_path: &Path,
        target_path: &Path,
        source_path: &Path,
    ) -> Result<Option<Self>> {
        let dotfiles_dir = crate::settings::dotfile_dir();
        Self::detect_in(dot_name, rendered_path, target_path, source_path, &dotfiles_dir)
    }

    /// Detect if there's a conflict between the rendered dotfile and the target.
    ///
    /// `dotfiles_dir` is the absolute path to the dotfiles repository root.
    /// It is used to locate the `.dots` directory for symlink comparison.
    /// This variant avoids the global `dotfile_dir()` call, making it suitable
    /// for use with explicitly-loaded v4 configuration.
    pub fn detect_in(
        dot_name: &str,
        rendered_path: &Path,
        target_path: &Path,
        source_path: &Path,
        dotfiles_dir: &Path,
    ) -> Result<Option<Self>> {
        // If target doesn't exist, no conflict
        if !target_path.exists() {
            return Ok(None);
        }

        // If target is already a symlink pointing to our .dots/, no conflict
        if target_path.is_symlink() {
            if let Ok(link_target) = target_path.canonicalize() {
                let dots_dir = dotfiles_dir.join(".dots");
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

/// Resolution for a reviewed change
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewResolution {
    /// Apply this change
    Apply,
    /// Skip this change
    Skip,
    /// Edit the file before applying
    Edit,
    /// Apply all remaining changes without prompting
    ApplyAll,
    /// Skip all remaining changes
    SkipAll,
}

/// Represents a change to be reviewed (create, update, or delete)
#[derive(Debug)]
pub struct ReviewChange {
    /// Name of the dot entry (if applicable)
    pub dot_name: Option<String>,
    /// Path to the target file
    pub target_path: PathBuf,
    /// Type of change
    pub change_type: ReviewChangeType,
    /// Content before (for updates/deletes)
    pub before_content: Option<String>,
    /// Content after (for creates/updates)
    pub after_content: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewChangeType {
    Create,
    Update,
    Delete,
}

impl ReviewChange {
    /// Create a new ReviewChange for a file creation
    pub fn create(dot_name: Option<String>, target_path: PathBuf, content: String) -> Self {
        Self {
            dot_name,
            target_path,
            change_type: ReviewChangeType::Create,
            before_content: None,
            after_content: Some(content),
        }
    }

    /// Create a new ReviewChange for a file update
    pub fn update(dot_name: Option<String>, target_path: PathBuf, before: String, after: String) -> Self {
        Self {
            dot_name,
            target_path,
            change_type: ReviewChangeType::Update,
            before_content: Some(before),
            after_content: Some(after),
        }
    }

    /// Create a new ReviewChange for a file deletion
    pub fn delete(dot_name: Option<String>, target_path: PathBuf, content: String) -> Self {
        Self {
            dot_name,
            target_path,
            change_type: ReviewChangeType::Delete,
            before_content: Some(content),
            after_content: None,
        }
    }

    /// Generate a diff for this change
    pub fn generate_diff(&self) -> String {
        let before = self.before_content.as_deref().unwrap_or("");
        let after = self.after_content.as_deref().unwrap_or("");

        let diff = TextDiff::from_lines(before, after);
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

    /// Show header for this change
    pub fn print_header(&self) {
        println!();
        println!("{}", "═".repeat(60).yellow());
        let change_label = match self.change_type {
            ReviewChangeType::Create => "Create".green(),
            ReviewChangeType::Update => "Update".blue(),
            ReviewChangeType::Delete => "Delete".red(),
        };
        print!("{} {}", change_label.bold(), self.target_path.display());
        if let Some(ref dot_name) = self.dot_name {
            print!(" ({})", dot_name.cyan());
        }
        println!();
        println!("{}", "─".repeat(60).yellow());
    }

    /// Prompt user to review and approve this change
    pub fn prompt_user(&self) -> Result<ReviewResolution> {
        self.print_header();

        // Show diff
        let diff = self.generate_diff();
        if !diff.is_empty() {
            println!("{}", diff);
        } else if let Some(ref content) = self.after_content {
            // For creates with no before content, just show the new content
            println!("{}", content.green());
        }

        println!("{}", "─".repeat(60).yellow());
        println!();
        println!("Choose action:");
        println!("  {} Apply this change", "[y]".cyan());
        println!("  {} Skip this change", "[n]".cyan());
        println!("  {} Edit the file", "[e]".cyan());
        println!();
        println!(
            "  {} Apply ALL remaining changes",
            "[Y]".cyan().bold()
        );
        println!("  {} Skip ALL remaining changes", "[N]".cyan().bold());
        println!();
        print!("{} ", ">".green().bold());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;

        match input.trim() {
            "y" | "" => Ok(ReviewResolution::Apply),
            "n" => Ok(ReviewResolution::Skip),
            "e" => {
                self.open_editor()?;
                self.prompt_user()
            }
            "Y" => Ok(ReviewResolution::ApplyAll),
            "N" => Ok(ReviewResolution::SkipAll),
            _ => {
                println!("{}", "Invalid choice, please try again.".red());
                self.prompt_user()
            }
        }
    }

    /// Open editor for this file
    fn open_editor(&self) -> Result<()> {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
        println!("{}", format!("Opening editor: {} ...", editor).yellow());

        // For creates/updates, edit the after content file (rendered path in .dots/)
        // The after_content is already at a path we need to determine
        // For now, we'll just note this is a limitation
        println!(
            "{}",
            "Note: Manual editing should be done on the source file in your dotfiles repo.".yellow()
        );

        Ok(())
    }
}

/// Context for tracking review state across multiple changes
#[derive(Default)]
pub struct ReviewContext {
    /// Whether review mode is active
    pub active: bool,
    /// Sticky resolution (from "for all" choices)
    pub sticky_resolution: Option<ReviewResolution>,
    /// Count of changes applied
    pub applied_count: usize,
    /// Count of changes skipped
    pub skipped_count: usize,
}

impl ReviewContext {
    pub fn new(active: bool) -> Self {
        Self {
            active,
            sticky_resolution: None,
            applied_count: 0,
            skipped_count: 0,
        }
    }

    /// Review a change, returning whether it should be applied
    pub fn review(&mut self, change: &ReviewChange) -> Result<bool> {
        if !self.active {
            return Ok(true);
        }

        // Check for sticky resolution first
        if let Some(ref sticky) = self.sticky_resolution {
            return Ok(match sticky {
                ReviewResolution::ApplyAll => {
                    self.applied_count += 1;
                    true
                }
                ReviewResolution::SkipAll => {
                    self.skipped_count += 1;
                    false
                }
                _ => true,
            });
        }

        let resolution = change.prompt_user()?;

        // Update sticky if "for all" was chosen
        match &resolution {
            ReviewResolution::ApplyAll | ReviewResolution::SkipAll => {
                self.sticky_resolution = Some(resolution.clone());
            }
            _ => {}
        }

        // Update counts and return decision
        match resolution {
            ReviewResolution::Apply | ReviewResolution::ApplyAll | ReviewResolution::Edit => {
                self.applied_count += 1;
                Ok(true)
            }
            ReviewResolution::Skip | ReviewResolution::SkipAll => {
                self.skipped_count += 1;
                Ok(false)
            }
        }
    }

    /// Print summary at the end
    pub fn print_summary(&self) {
        if self.active && (self.applied_count > 0 || self.skipped_count > 0) {
            println!();
            println!("{}", "─".repeat(40));
            println!(
                "Review: {} applied, {} skipped",
                self.applied_count.to_string().green(),
                self.skipped_count.to_string().yellow()
            );
        }
    }
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

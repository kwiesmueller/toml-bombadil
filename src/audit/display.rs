//! Display formatting for audit output.
//!
//! Handles formatting of action logs, session summaries, and diffs.

use super::action::{Action, ActionType};
use super::capture::CapturedTrace;
use super::file_index::{FileAction, FileActionType};
use super::plan::{ActionPlan, PlanSummary, PlannedAction};
use super::session::Session;
use super::storage::AuditStorage;
use colored::*;
use similar::{ChangeTag, TextDiff};
use std::io::{self, Write};
use std::path::Path;

/// Format a planned action for display.
pub fn format_planned_action(action: &PlannedAction, index: usize) -> String {
    let indicator = action.action_type.type_indicator();
    let description = action.action_type.description();

    format!("  [{:>3}] {} {}", index, indicator.bold(), description)
}

/// Format an executed action for display.
pub fn format_action(action: &Action) -> String {
    let indicator = action.action_type.type_indicator();
    let description = action.action_type.description();

    format!(
        "  {} [{}] {} {}",
        "✓".green(),
        action.id.as_str().cyan(),
        indicator,
        description
    )
}

/// Format a failed action for display.
pub fn format_action_error(action: &Action, error: &str) -> String {
    let indicator = action.action_type.type_indicator();
    let description = action.action_type.description();

    format!(
        "  {} [{}] {} {}: {}",
        "✗".red(),
        action.id.as_str().cyan(),
        indicator,
        description,
        error.red()
    )
}

/// Print the action plan header.
pub fn print_plan_header(plan: &ActionPlan) {
    let summary = PlanSummary::from_plan(plan);

    if plan.is_empty() {
        println!("{}", "No changes to make.".yellow());
        return;
    }

    println!("\n{}", "Planned actions:".bold());
    for (i, action) in plan.actions().iter().enumerate() {
        println!("{}", format_planned_action(action, i + 1));
    }
    println!();
    println!("Summary: {}", summary.one_line());
}

/// Print session completion summary.
pub fn print_session_summary(session: &Session) {
    if session.is_empty() {
        println!("\n{}", "No changes were made.".yellow());
        return;
    }

    println!();
    for action in &session.actions {
        println!("{}", format_action(action));
    }

    println!();
    println!("{}", "Session Summary:".bold());
    println!("  {}", session.stats.summary());
    println!(
        "  Use '{}' to review | '{}' to undo",
        "bombadil log".cyan(),
        "bombadil revert <id>".cyan()
    );
}

/// Format a session for the log command.
pub fn format_session_log(session: &Session, verbose: bool) -> String {
    let mut output = String::new();

    let date = session.started_at.format("%Y-%m-%d %H:%M:%S");
    let status = if session.completed_at.is_some() {
        "completed".green()
    } else {
        "incomplete".yellow()
    };

    output.push_str(&format!(
        "{} {} [{}] - {}\n",
        date.to_string().dimmed(),
        session.id.to_string().cyan(),
        status,
        session.command
    ));

    if verbose {
        for action in &session.actions {
            output.push_str(&format!(
                "    [{}] {} {}\n",
                action.id.as_str().cyan(),
                action.action_type.type_indicator(),
                action.action_type.description()
            ));
        }
    } else {
        output.push_str(&format!("    {}\n", session.stats.summary()));
    }

    output
}

/// Format an action's details for the inspect command.
pub fn format_action_details(action: &Action) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "{}: {}\n",
        "Action ID".bold(),
        action.id.as_str().cyan()
    ));
    output.push_str(&format!("{}: {}\n", "Timestamp".bold(), action.timestamp));
    output.push_str(&format!(
        "{}: {}\n",
        "Type".bold(),
        action.action_type.type_indicator()
    ));

    if let Some(dot) = &action.dot_name {
        output.push_str(&format!("{}: {}\n", "Dot".bold(), dot));
    }

    output.push_str(&format!(
        "{}: {}\n",
        "Description".bold(),
        action.action_type.description()
    ));
    output.push_str(&format!(
        "{}: {}\n",
        "Revertible".bold(),
        if action.is_revertible() {
            "yes".green()
        } else {
            "no".red()
        }
    ));

    // Add type-specific details
    match &action.action_type {
        ActionType::FileCreate {
            target,
            source,
            content_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Source: {}\n", source.display()));
            output.push_str(&format!("  Content hash: {}\n", content_hash));
        }
        ActionType::FileUpdate {
            target,
            source,
            before_hash,
            after_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Source: {}\n", source.display()));
            output.push_str(&format!("  Before hash: {}\n", before_hash));
            output.push_str(&format!("  After hash: {}\n", after_hash));
        }
        ActionType::FilePatch {
            target,
            base,
            patches_applied,
            before_hash,
            after_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Base: {}\n", base.display()));
            output.push_str(&format!("  Patches applied: {}\n", patches_applied.len()));
            output.push_str(&format!("  Before hash: {}\n", before_hash));
            output.push_str(&format!("  After hash: {}\n", after_hash));
        }
        ActionType::SemanticPatch {
            target,
            format,
            operations,
            before_hash,
            after_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Format: {}\n", format));
            output.push_str(&format!("  Operations: {}\n", operations.len()));
            output.push_str(&format!("  Before hash: {}\n", before_hash));
            output.push_str(&format!("  After hash: {}\n", after_hash));
        }
        ActionType::Inject {
            target,
            marker,
            before_hash,
            after_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Marker: {}\n", marker));
            output.push_str(&format!("  Before hash: {}\n", before_hash));
            output.push_str(&format!("  After hash: {}\n", after_hash));
        }
        ActionType::SymlinkCreate { source, target } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Source: {}\n", source.display()));
            output.push_str(&format!("  Target: {}\n", target.display()));
        }
        ActionType::SymlinkRemove {
            target,
            was_pointing_to,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!(
                "  Was pointing to: {}\n",
                was_pointing_to.display()
            ));
        }
        ActionType::Backup {
            original,
            backup_location,
            content_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Original: {}\n", original.display()));
            output.push_str(&format!("  Backup: {}\n", backup_location.display()));
            output.push_str(&format!("  Content hash: {}\n", content_hash));
        }
        ActionType::ConflictResolved {
            target,
            resolution,
            dotfile_hash,
            system_hash,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Target: {}\n", target.display()));
            output.push_str(&format!("  Resolution: {}\n", resolution));
            output.push_str(&format!("  Dotfile hash: {}\n", dotfile_hash));
            output.push_str(&format!("  System hash: {}\n", system_hash));
        }
        ActionType::HookExecuted {
            command,
            hook_type,
            exit_code,
        } => {
            output.push_str(&format!("\n{}\n", "Details:".bold()));
            output.push_str(&format!("  Command: {}\n", command));
            output.push_str(&format!("  Hook type: {}\n", hook_type));
            output.push_str(&format!("  Exit code: {}\n", exit_code));
        }
    }

    output
}

/// Generate and format a diff between before and after content.
pub fn format_diff(before: &str, after: &str, target_path: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut output = String::new();

    output.push_str(&format!("--- a/{}\n", target_path));
    output.push_str(&format!("+++ b/{}\n", target_path));

    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            output.push_str("...\n");
        }

        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, color): (&str, fn(&str) -> ColoredString) = match change.tag() {
                    ChangeTag::Delete => ("-", |s: &str| s.red()),
                    ChangeTag::Insert => ("+", |s: &str| s.green()),
                    ChangeTag::Equal => (" ", |s: &str| s.normal()),
                };

                let line = change.value();
                output.push_str(&format!("{}{}", color(sign), color(line)));
                if change.missing_newline() {
                    output.push_str("\n\\ No newline at end of file\n");
                }
            }
        }
    }

    output
}

/// Generate a diff from storage for an action.
pub fn generate_diff_from_storage(storage: &AuditStorage, action: &Action) -> Option<String> {
    let before = storage.load_before_content(&action.id).ok()?;
    let after = storage.load_after_content(&action.id).ok()?;

    let before_str = String::from_utf8_lossy(&before);
    let after_str = String::from_utf8_lossy(&after);

    let target = action.action_type.target_path()?;
    Some(format_diff(
        &before_str,
        &after_str,
        &target.to_string_lossy(),
    ))
}

/// Interactive prompt for plan approval.
pub fn prompt_execute(plan: &ActionPlan) -> io::Result<bool> {
    print!(
        "\nExecute {} action(s)? [y/n/i for interactive]: ",
        plan.approved_count()
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        _ => Ok(false),
    }
}

/// Interactive prompt result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptResult {
    /// Execute approved actions.
    Execute,
    /// Abort and exit.
    Abort,
    /// Enter interactive mode.
    Interactive,
}

/// Interactive prompt for plan approval with 'i' option.
pub fn prompt_execute_interactive(plan: &ActionPlan) -> io::Result<PromptResult> {
    print!("{}", format_interactive_prompt(plan));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(PromptResult::Execute),
        "i" | "interactive" => Ok(PromptResult::Interactive),
        _ => Ok(PromptResult::Abort),
    }
}

/// Format interactive plan prompt with i option.
pub fn format_interactive_prompt(plan: &ActionPlan) -> String {
    format!(
        "\nExecute {} action(s)? [{}/{}/{} for interactive]: ",
        plan.approved_count(),
        "y".green(),
        "n".red(),
        "i".cyan()
    )
}

/// Print interactive mode help.
pub fn print_interactive_help() {
    println!("\n{}", "Interactive mode commands:".bold());
    println!("  {}         Show details of action N", "<N>".cyan());
    println!("  {}    Show diff for action N", "diff <N>".cyan());
    println!("  {}     Show captured logs for action N", "log <N>".cyan());
    println!("  {} Show all actions affecting path", "file <path>".cyan());
    println!("  {}    Skip action N (won't execute)", "skip <N>".cyan());
    println!("  {}  Restore skipped action", "unskip <N>".cyan());
    println!(
        "  {}        Show plan again (with skip status)",
        "list".cyan()
    );
    println!("  {}   Execute approved actions", "y/execute".cyan());
    println!("  {}     Cancel and exit", "n/abort".cyan());
    println!("  {} Save plan to custom file", "save <file>".cyan());
}

/// Format file history for inspect --file command.
///
/// Output format:
/// ```text
/// File: ~/.bashrc
/// Total actions: 3
///
/// [2024-02-04 10:30:00] Session: link (profiles: default, work)
///   [abc12] + Create file (from dotfiles/bashrc)
///           Content: 847291a3... (142 bytes)
///
/// [2024-02-04 14:15:00] Session: link (profiles: default, work)
///   [def34] ~ Update file
///           Before: 847291a3... -> After: f3928bc1...
///           -----------------------------------------
///           @@ -10,3 +10,5 @@
///            export PATH="$HOME/bin:$PATH"
///           +export EDITOR="nvim"
///           +alias ll="ls -la"
///           -----------------------------------------
///
/// [2024-02-04 16:00:00] Session: link (profiles: default)
///   [ghi56] I Inject at marker '### BOMBADIL ###'
///           Before: f3928bc1... -> After: 9a7bc3d2...
///           Logs captured (use --log to view)
/// ```
pub fn format_file_history(
    path: &Path,
    actions: &[(FileAction, Option<&Action>, Option<&Session>)],
    show_diffs: bool,
    storage: Option<&AuditStorage>,
) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "{}: {}\n",
        "File".bold(),
        path.display().to_string().cyan()
    ));
    output.push_str(&format!(
        "{}: {}\n\n",
        "Total actions".bold(),
        actions.len()
    ));

    // Group actions by session for display
    // Use Option<Option<String>> so None means "no session seen yet"
    // while Some(None) means "last session had no ID"
    let mut current_session_id: Option<Option<String>> = None;

    for (file_action, action, session) in actions {
        // Print session header if session changed
        let session_id = session.map(|s| s.id.as_str().to_string());
        if current_session_id.as_ref() != Some(&session_id) {
            current_session_id = Some(session_id.clone());

            if let Some(sess) = session {
                let date = sess.started_at.format("%Y-%m-%d %H:%M:%S");
                let profiles = if sess.profiles.is_empty() {
                    String::new()
                } else {
                    format!(" (profiles: {})", sess.profiles.join(", "))
                };
                output.push_str(&format!(
                    "[{}] Session: {}{}\n",
                    date.to_string().dimmed(),
                    sess.command.yellow(),
                    profiles.dimmed()
                ));
            } else {
                let date = file_action.timestamp.format("%Y-%m-%d %H:%M:%S");
                output.push_str(&format!(
                    "[{}] Session: {}\n",
                    date.to_string().dimmed(),
                    "unknown".dimmed()
                ));
            }
        }

        // Format the file action
        output.push_str(&format_file_action(file_action, *action, *session));

        // Show diff if requested and available
        if show_diffs {
            if let (Some(act), Some(store)) = (action, storage) {
                if let Some(diff) = generate_diff_from_storage(store, act) {
                    output.push_str(&format!("          {}\n", "-".repeat(41).dimmed()));
                    for line in diff.lines().skip(2) {
                        // Skip header lines
                        output.push_str(&format!("          {}\n", line));
                    }
                    output.push_str(&format!("          {}\n", "-".repeat(41).dimmed()));
                }
            }
        }

        output.push('\n');
    }

    output
}

/// Format a single file action in history view.
pub fn format_file_action(
    file_action: &FileAction,
    action: Option<&Action>,
    _session: Option<&Session>,
) -> String {
    let mut output = String::new();

    let indicator = match file_action.action_type {
        FileActionType::Create => "+".green(),
        FileActionType::Update => "~".yellow(),
        FileActionType::Delete => "-".red(),
        FileActionType::Read => "R".dimmed(),
    };

    let action_id = file_action.action_id.as_str();
    let action_type_desc = match file_action.action_type {
        FileActionType::Create => "Create file",
        FileActionType::Update => "Update file",
        FileActionType::Delete => "Delete file",
        FileActionType::Read => "Read file",
    };

    // Basic action line
    output.push_str(&format!(
        "  [{}] {} {}\n",
        action_id.cyan(),
        indicator.bold(),
        action_type_desc
    ));

    // Add details from the full Action if available
    if let Some(act) = action {
        match &act.action_type {
            ActionType::FileCreate {
                source,
                content_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          (from {})\n",
                    source.display().to_string().dimmed()
                ));
                output.push_str(&format!(
                    "          Content: {}...\n",
                    truncate_hash(content_hash).dimmed()
                ));
            }
            ActionType::FileUpdate {
                before_hash,
                after_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Before: {}... {} After: {}...\n",
                    truncate_hash(before_hash).dimmed(),
                    "->".dimmed(),
                    truncate_hash(after_hash).dimmed()
                ));
            }
            ActionType::FilePatch {
                base,
                patches_applied,
                before_hash,
                after_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Base: {}, {} patches applied\n",
                    base.display().to_string().dimmed(),
                    patches_applied.len()
                ));
                output.push_str(&format!(
                    "          Before: {}... {} After: {}...\n",
                    truncate_hash(before_hash).dimmed(),
                    "->".dimmed(),
                    truncate_hash(after_hash).dimmed()
                ));
            }
            ActionType::SemanticPatch {
                format,
                operations,
                before_hash,
                after_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Format: {}, {} operations\n",
                    format.dimmed(),
                    operations.len()
                ));
                output.push_str(&format!(
                    "          Before: {}... {} After: {}...\n",
                    truncate_hash(before_hash).dimmed(),
                    "->".dimmed(),
                    truncate_hash(after_hash).dimmed()
                ));
            }
            ActionType::Inject {
                marker,
                before_hash,
                after_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Inject at marker '{}'\n",
                    marker.yellow()
                ));
                output.push_str(&format!(
                    "          Before: {}... {} After: {}...\n",
                    truncate_hash(before_hash).dimmed(),
                    "->".dimmed(),
                    truncate_hash(after_hash).dimmed()
                ));
            }
            ActionType::SymlinkCreate { source, .. } => {
                output.push_str(&format!(
                    "          Symlink to {}\n",
                    source.display().to_string().dimmed()
                ));
            }
            ActionType::SymlinkRemove {
                was_pointing_to, ..
            } => {
                output.push_str(&format!(
                    "          Was pointing to {}\n",
                    was_pointing_to.display().to_string().dimmed()
                ));
            }
            ActionType::Backup {
                backup_location,
                content_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Backed up to {}\n",
                    backup_location.display().to_string().dimmed()
                ));
                output.push_str(&format!(
                    "          Content: {}...\n",
                    truncate_hash(content_hash).dimmed()
                ));
            }
            ActionType::ConflictResolved {
                resolution,
                dotfile_hash,
                system_hash,
                ..
            } => {
                output.push_str(&format!(
                    "          Resolution: {}\n",
                    resolution.to_string().yellow()
                ));
                output.push_str(&format!(
                    "          Dotfile: {}... System: {}...\n",
                    truncate_hash(dotfile_hash).dimmed(),
                    truncate_hash(system_hash).dimmed()
                ));
            }
            ActionType::HookExecuted { .. } => {}
        }

        // Show if logs were captured
        if act.log_captured {
            output.push_str(&format!(
                "          {}\n",
                "Logs captured (use --log to view)".dimmed()
            ));
        }
    } else if let Some(hash) = &file_action.content_hash {
        // Fall back to file_action content hash if no full action
        output.push_str(&format!(
            "          Content: {}...\n",
            truncate_hash(hash).dimmed()
        ));
    }

    output
}

/// Format captured traces for display.
pub fn format_traces(traces: &[CapturedTrace]) -> String {
    let mut output = String::new();

    if traces.is_empty() {
        output.push_str(&format!("{}\n", "No logs captured.".dimmed()));
        return output;
    }

    output.push_str(&format!(
        "{} ({} entries):\n\n",
        "Captured logs".bold(),
        traces.len()
    ));

    for trace in traces {
        let timestamp = trace.timestamp.format("%H:%M:%S%.3f");
        let level_colored = match trace.level.as_str() {
            "ERROR" => trace.level.red(),
            "WARN" => trace.level.yellow(),
            "INFO" => trace.level.green(),
            "DEBUG" => trace.level.blue(),
            "TRACE" => trace.level.magenta(),
            _ => trace.level.normal(),
        };

        let span_prefix = if trace.span_path.is_empty() {
            String::new()
        } else {
            format!("[{}] ", trace.span_path.join("::").dimmed())
        };

        let fields_str = if trace.fields.as_object().is_none_or(|m| m.is_empty()) {
            String::new()
        } else {
            format!(" {}", trace.fields.to_string().dimmed())
        };

        output.push_str(&format!(
            "{} {:>5} {}{}: {}{}\n",
            timestamp.to_string().dimmed(),
            level_colored,
            span_prefix,
            trace.target.dimmed(),
            trace.message,
            fields_str
        ));
    }

    output
}

/// Truncate a hash to first 8 characters for display.
fn truncate_hash(hash: &str) -> &str {
    if hash.len() > 8 {
        &hash[..8]
    } else {
        hash
    }
}

/// Print legend for action indicators.
pub fn print_legend() {
    println!("\n{}", "Legend:".bold());
    println!(
        "  {} Create file    {} Update file    {} Patch file",
        "+".bold(),
        "~".bold(),
        "P".bold()
    );
    println!(
        "  {} Semantic patch {} Inject         {} Symlink",
        "S".bold(),
        "I".bold(),
        "L".bold()
    );
    println!(
        "  {} Unlink         {} Backup         {} Conflict",
        "U".bold(),
        "B".bold(),
        "C".bold()
    );
    println!("  {} Hook", "H".bold());
}

#[cfg(test)]
mod tests {
    use super::super::action::ActionId;
    use super::super::session::SessionId;
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    #[test]
    fn test_format_diff() {
        let before = "line1\nline2\nline3\n";
        let after = "line1\nmodified\nline3\n";

        let diff = format_diff(before, after, "test.txt");
        assert!(diff.contains("--- a/test.txt"));
        assert!(diff.contains("+++ b/test.txt"));
    }

    #[test]
    fn test_truncate_hash() {
        assert_eq!(truncate_hash("abcdefghijklmnop"), "abcdefgh");
        assert_eq!(truncate_hash("short"), "short");
        assert_eq!(truncate_hash("12345678"), "12345678");
        assert_eq!(truncate_hash(""), "");
    }

    #[test]
    fn test_prompt_result_enum() {
        // Just verify the enum variants exist and can be compared
        assert_eq!(PromptResult::Execute, PromptResult::Execute);
        assert_ne!(PromptResult::Execute, PromptResult::Abort);
        assert_ne!(PromptResult::Execute, PromptResult::Interactive);
        assert_ne!(PromptResult::Abort, PromptResult::Interactive);
    }

    #[test]
    fn test_format_interactive_prompt() {
        let plan = ActionPlan::new();
        let prompt = format_interactive_prompt(&plan);
        assert!(prompt.contains("0 action(s)"));
        // The prompt should contain y, n, i options (with ANSI codes)
        assert!(prompt.contains("for interactive"));
    }

    #[test]
    fn test_format_file_action_create() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("abc12"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: Some("1234567890abcdef".to_string()),
        };

        let output = format_file_action(&file_action, None, None);
        assert!(output.contains("abc12"));
        assert!(output.contains("Create file"));
        assert!(output.contains("12345678")); // truncated hash
    }

    #[test]
    fn test_format_file_action_update() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("def34"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("abcdef1234567890".to_string()),
        };

        let output = format_file_action(&file_action, None, None);
        assert!(output.contains("def34"));
        assert!(output.contains("Update file"));
    }

    #[test]
    fn test_format_file_action_delete() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("ghi56"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Delete,
            content_hash: None,
        };

        let output = format_file_action(&file_action, None, None);
        assert!(output.contains("ghi56"));
        assert!(output.contains("Delete file"));
    }

    #[test]
    fn test_format_file_action_with_action_details() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("abc12"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: Some("1234567890abcdef".to_string()),
        };

        let action = Action::new(
            ActionType::FileCreate {
                target: PathBuf::from("/home/user/.bashrc"),
                source: PathBuf::from("/dotfiles/bashrc"),
                content_hash: "abcdef1234567890abcdef1234567890".to_string(),
            },
            Some("bashrc".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("abc12"));
        assert!(output.contains("Create file"));
        assert!(output.contains("/dotfiles/bashrc"));
        assert!(output.contains("abcdef12")); // truncated hash
    }

    #[test]
    fn test_format_file_action_with_inject() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("inj01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("newcontent12345".to_string()),
        };

        let action = Action::new(
            ActionType::Inject {
                target: PathBuf::from("/home/user/.bashrc"),
                marker: "### BOMBADIL ###".to_string(),
                before_hash: "beforehash12345678".to_string(),
                after_hash: "afterhash123456789".to_string(),
            },
            Some("bashrc".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("inj01"));
        assert!(output.contains("### BOMBADIL ###"));
        assert!(output.contains("beforeha")); // truncated
        assert!(output.contains("afterhas")); // truncated
    }

    #[test]
    fn test_format_file_action_with_logs_captured() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("log01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("content".to_string()),
        };

        let mut action = Action::new(
            ActionType::FileUpdate {
                target: PathBuf::from("/home/user/.bashrc"),
                source: PathBuf::from("/dotfiles/bashrc"),
                before_hash: "before".to_string(),
                after_hash: "after".to_string(),
            },
            Some("bashrc".to_string()),
        );
        action.log_captured = true;

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("Logs captured"));
    }

    #[test]
    fn test_format_file_history_empty() {
        let path = Path::new("/home/user/.bashrc");
        let actions: Vec<(FileAction, Option<&Action>, Option<&Session>)> = vec![];

        let output = format_file_history(path, &actions, false, None);
        assert!(output.contains("/home/user/.bashrc"));
        assert!(output.contains("Total actions: 0"));
    }

    #[test]
    fn test_format_file_history_single_action() {
        let path = Path::new("/home/user/.bashrc");
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();

        let file_action = FileAction {
            action_id: ActionId::from_string("abc12"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: Some("1234567890abcdef".to_string()),
        };

        let session = Session::new("link", vec!["default".to_string(), "work".to_string()]);

        let actions: Vec<(FileAction, Option<&Action>, Option<&Session>)> =
            vec![(file_action, None, Some(&session))];

        let output = format_file_history(path, &actions, false, None);
        assert!(output.contains("/home/user/.bashrc"));
        assert!(output.contains("Total actions: 1"));
        assert!(output.contains("link"));
        assert!(output.contains("default, work"));
        assert!(output.contains("abc12"));
        assert!(output.contains("Create file"));
    }

    #[test]
    fn test_format_file_history_multiple_actions() {
        let path = Path::new("/home/user/.bashrc");
        let t1 = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 2, 4, 14, 15, 0).unwrap();

        let file_action1 = FileAction {
            action_id: ActionId::from_string("abc12"),
            session_id: SessionId::from_string("session_001"),
            timestamp: t1,
            action_type: FileActionType::Create,
            content_hash: Some("hash1".to_string()),
        };

        let file_action2 = FileAction {
            action_id: ActionId::from_string("def34"),
            session_id: SessionId::from_string("session_002"),
            timestamp: t2,
            action_type: FileActionType::Update,
            content_hash: Some("hash2".to_string()),
        };

        let session1 = Session::new("link", vec!["default".to_string()]);
        let session2 = Session::new("link", vec!["work".to_string()]);

        let actions: Vec<(FileAction, Option<&Action>, Option<&Session>)> = vec![
            (file_action1, None, Some(&session1)),
            (file_action2, None, Some(&session2)),
        ];

        let output = format_file_history(path, &actions, false, None);
        assert!(output.contains("Total actions: 2"));
        assert!(output.contains("abc12"));
        assert!(output.contains("def34"));
        assert!(output.contains("Create file"));
        assert!(output.contains("Update file"));
    }

    #[test]
    fn test_format_file_history_without_session() {
        let path = Path::new("/home/user/.bashrc");
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();

        let file_action = FileAction {
            action_id: ActionId::from_string("xyz99"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: Some("hash".to_string()),
        };

        let actions: Vec<(FileAction, Option<&Action>, Option<&Session>)> =
            vec![(file_action, None, None)];

        let output = format_file_history(path, &actions, false, None);
        assert!(output.contains("unknown")); // Session shown as unknown
    }

    #[test]
    fn test_format_traces_empty() {
        let traces: Vec<CapturedTrace> = vec![];
        let output = format_traces(&traces);
        assert!(output.contains("No logs captured"));
    }

    #[test]
    fn test_format_traces_single() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let traces = vec![CapturedTrace {
            timestamp,
            level: "INFO".to_string(),
            target: "bombadil::link".to_string(),
            message: "Processing dot bashrc".to_string(),
            fields: serde_json::json!({}),
            span_path: vec![],
        }];

        let output = format_traces(&traces);
        assert!(output.contains("Captured logs"));
        assert!(output.contains("1 entries"));
        assert!(output.contains("INFO"));
        assert!(output.contains("Processing dot bashrc"));
        assert!(output.contains("bombadil::link"));
    }

    #[test]
    fn test_format_traces_with_spans() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let traces = vec![CapturedTrace {
            timestamp,
            level: "DEBUG".to_string(),
            target: "bombadil::dots".to_string(),
            message: "Rendering template".to_string(),
            fields: serde_json::json!({"file": "/home/user/.bashrc"}),
            span_path: vec!["link".to_string(), "process_dot".to_string()],
        }];

        let output = format_traces(&traces);
        assert!(output.contains("link::process_dot"));
        assert!(output.contains("Rendering template"));
    }

    #[test]
    fn test_format_traces_multiple_levels() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let traces = vec![
            CapturedTrace {
                timestamp,
                level: "INFO".to_string(),
                target: "test".to_string(),
                message: "info message".to_string(),
                fields: serde_json::json!({}),
                span_path: vec![],
            },
            CapturedTrace {
                timestamp,
                level: "WARN".to_string(),
                target: "test".to_string(),
                message: "warning message".to_string(),
                fields: serde_json::json!({}),
                span_path: vec![],
            },
            CapturedTrace {
                timestamp,
                level: "ERROR".to_string(),
                target: "test".to_string(),
                message: "error message".to_string(),
                fields: serde_json::json!({}),
                span_path: vec![],
            },
        ];

        let output = format_traces(&traces);
        assert!(output.contains("3 entries"));
        assert!(output.contains("INFO"));
        assert!(output.contains("WARN"));
        assert!(output.contains("ERROR"));
        assert!(output.contains("info message"));
        assert!(output.contains("warning message"));
        assert!(output.contains("error message"));
    }

    #[test]
    fn test_format_file_action_with_semantic_patch() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("sem01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("content".to_string()),
        };

        let action = Action::new(
            ActionType::SemanticPatch {
                target: PathBuf::from("/home/user/config.json"),
                format: "json".to_string(),
                operations: vec!["set $.key".to_string(), "delete $.old".to_string()],
                before_hash: "before123456789".to_string(),
                after_hash: "after1234567890".to_string(),
            },
            Some("config".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("sem01"));
        assert!(output.contains("json"));
        assert!(output.contains("2 operations"));
    }

    #[test]
    fn test_format_file_action_with_file_patch() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("pat01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("content".to_string()),
        };

        let action = Action::new(
            ActionType::FilePatch {
                target: PathBuf::from("/home/user/.vimrc"),
                base: PathBuf::from("/dotfiles/vimrc"),
                patches_applied: vec!["patch1.patch".to_string(), "patch2.patch".to_string()],
                before_hash: "before123456789".to_string(),
                after_hash: "after1234567890".to_string(),
            },
            Some("vimrc".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("pat01"));
        assert!(output.contains("/dotfiles/vimrc"));
        assert!(output.contains("2 patches applied"));
    }

    #[test]
    fn test_format_file_action_with_symlink_create() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("sym01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: None,
        };

        let action = Action::new(
            ActionType::SymlinkCreate {
                source: PathBuf::from("/dotfiles/bashrc"),
                target: PathBuf::from("/home/user/.bashrc"),
            },
            Some("bashrc".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("sym01"));
        assert!(output.contains("Symlink to"));
        assert!(output.contains("/dotfiles/bashrc"));
    }

    #[test]
    fn test_format_file_action_with_backup() {
        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("bak01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Create,
            content_hash: Some("backup_content_hash".to_string()),
        };

        let action = Action::new(
            ActionType::Backup {
                original: PathBuf::from("/home/user/.bashrc"),
                backup_location: PathBuf::from("/home/user/.bashrc.backup"),
                content_hash: "backuphash1234567890".to_string(),
            },
            None,
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("bak01"));
        assert!(output.contains("Backed up to"));
        assert!(output.contains(".bashrc.backup"));
    }

    #[test]
    fn test_format_file_action_with_conflict_resolved() {
        use super::super::action::ConflictResolution;

        let timestamp = Utc.with_ymd_and_hms(2024, 2, 4, 10, 30, 0).unwrap();
        let file_action = FileAction {
            action_id: ActionId::from_string("cnf01"),
            session_id: SessionId::from_string("session_001"),
            timestamp,
            action_type: FileActionType::Update,
            content_hash: Some("resolved_hash".to_string()),
        };

        let action = Action::new(
            ActionType::ConflictResolved {
                target: PathBuf::from("/home/user/.bashrc"),
                resolution: ConflictResolution::UseDotfile,
                dotfile_hash: "dotfilehash12345".to_string(),
                system_hash: "systemhash123456".to_string(),
            },
            Some("bashrc".to_string()),
        );

        let output = format_file_action(&file_action, Some(&action), None);
        assert!(output.contains("cnf01"));
        assert!(output.contains("Resolution:"));
        assert!(output.contains("use dotfile"));
        assert!(output.contains("Dotfile:"));
        assert!(output.contains("System:"));
    }
}

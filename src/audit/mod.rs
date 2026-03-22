//! Action-based auditing system for Bombadil.
//!
//! This module provides comprehensive tracking of all changes made by bombadil,
//! enabling users to review, inspect diffs, and selectively revert individual changes.
//!
//! # Overview
//!
//! Every change bombadil makes is tracked as an "action" with:
//! - A unique short hash ID (e.g., "awjwu") for easy reference
//! - A description of what was done
//! - Before/after content snapshots (for revert capability)
//!
//! # Example Usage
//!
//! ```text
//! $ bombadil link
//! Planned actions:
//!   [  1] ~ Update file ~/.config/kitty/kitty.conf
//!   [  2] P Patch ~/.config/sway/config (3 patches)
//!   [  3] L Create symlink ~/.config/alacritty/alacritty.yml
//!
//! Execute 3 action(s)? [y/n/i for interactive]: y
//!
//!   ✓ [abcde] ~ Update file ~/.config/kitty/kitty.conf
//!   ✓ [bcdef] P Patch ~/.config/sway/config (3 patches)
//!   ✓ [cdefg] L Create symlink ~/.config/alacritty/alacritty.yml
//!
//! Session Summary:
//!   1 file updated, 1 file patched, 1 symlink created
//!   Use 'bombadil log' to review | 'bombadil revert <id>' to undo
//! ```
//!
//! # Commands
//!
//! - `bombadil log` - Show action history
//! - `bombadil inspect <id>` - Show action details
//! - `bombadil inspect <id> diff` - Show the diff for an action
//! - `bombadil revert <id>` - Revert an action
//!
//! # Storage
//!
//! Audit data is stored in `.dots/actions/`:
//! - `sessions/` - Session files with all actions
//! - `content/` - Before/after file snapshots
//! - `index.toml` - Action ID to session lookup

pub mod action;
pub mod capture;
pub mod display;
pub mod executor;
pub mod file_index;
pub mod objects;
pub mod plan;
pub mod revert;
pub mod session;
pub mod storage;

pub use action::{Action, ActionId, ActionType, ConflictResolution, HookType};
pub use capture::{
    deserialize_traces, format_captured_traces, serialize_traces, ActionCaptureLayer,
    CaptureBuffer, CapturedTrace,
};
pub use display::{
    format_action, format_action_details, format_action_error, format_diff, format_file_action,
    format_file_history, format_interactive_prompt, format_planned_action, format_session_log,
    format_traces, generate_diff_from_storage, print_interactive_help, print_legend,
    print_plan_header, print_session_summary, prompt_execute, prompt_execute_interactive,
    PromptResult,
};
pub use executor::{helpers, ActionExecutor, ExecutionResult};
pub use file_index::{FileAction, FileActionType, FileIndex};
pub use objects::{sha256_hash, ObjectStore};
pub use plan::{ActionPlan, PlanSummary, PlannedAction};
pub use revert::{
    FileRevertOptions, RevertAnalysis, RevertBlocker, RevertCheck, RevertEngine, RevertResult,
    RevertWarning,
};
pub use session::{Session, SessionId, SessionStats};
pub use storage::{content_hash, ActionIndex, AuditStorage};

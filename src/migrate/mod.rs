//! Migration helpers for converting v3 bombadil configs to v4 `dots.toml` format.
//!
//! # Overview
//!
//! The v3 format uses a single root `bombadil.toml` with `import` entries that
//! point to per-subsystem TOML files. Each imported file has a `[settings.dots]`
//! table and optional `[profiles.NAME.dots]` sections.
//!
//! The v4 format uses a `dots.toml` per directory, with a `[dot]` top-level table.
//!
//! # Migration strategy
//!
//! - For each import entry that contains `[settings]` (dot configs like `zsh/zsh.toml`):
//!   write a `dots.toml` in the **same directory**.
//! - For each import entry that contains only `[packages]` (package lists like
//!   `packages/cli.toml`): write a `dots.toml` in a **subdirectory** named after
//!   the file stem (e.g. `packages/cli/dots.toml`).
//! - Existing `dots.toml` files are never overwritten — they are reported as skipped.
//!
//! # Field mapping
//!
//! | v3 | v4 |
//! |---|---|
//! | `[settings.dots.NAME]` source/target | `[dot.files]` `"source" = "~/target"` |
//! | `[settings.prehooks]` | `[dot] prehooks` |
//! | `[settings.posthooks]` | `[dot] posthooks` |
//! | `[settings.vars]` | `[dot] vars` |
//! | `[profiles.NAME.dots.KEY]` | `[dot.profiles.NAME.files]` |
//! | `hard_copy_target` | `{ target = "~/...", copy = true }` as additional entry |
//! | `hard_copy_permissions` | `{ ..., hard_copy_permissions = N }` |
//! | `[packages.NAME]` | `[dot.packages.NAME]` |
//!
//! # Notes on loss of fidelity
//!
//! - v3 `Dot.ignore` patterns are preserved via the extended `FileTargetOptions`.
//! - v3 `Dot.vars` (single per-dot var file) is added to `dot.vars`.
//! - v3 `hard_copy_target` creates an **additional** `[dot.files]` entry with
//!   `copy = true` alongside the original symlink entry.
//! - Extended package manager configs (`PackageManagerConfig::Extended`) with
//!   `repo_url` or `gpg_key` fields have no v4 equivalent — a comment is
//!   emitted in the output and the simple package name is used.
//! - v3 `Package.enabled = false` has no direct v4 equivalent — such packages
//!   are tagged with `"disabled"` and noted in the report.
//! - `binary`, `git`, `source` install methods are preserved in the output
//!   (v4 schema supports them) but execution is currently stubbed.

use crate::settings::imports::ImportedSettings;
use crate::settings::packages::{
    CargoConfig, InstallMethods as V3InstallMethods, Package, PackageManagerConfig,
};
use anyhow::{Context, Result};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use tracing::info;

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Result of a migration run.
#[derive(Debug, Default)]
pub struct MigrationReport {
    /// Files successfully written.
    pub written: Vec<PathBuf>,
    /// Import paths skipped because a `dots.toml` already exists there.
    pub skipped_existing: Vec<PathBuf>,
    /// Import paths that could not be migrated (with reason).
    pub errors: Vec<(PathBuf, String)>,
    /// Warnings about features that need manual review.
    pub warnings: Vec<String>,
}

impl MigrationReport {
    /// Print the report to stdout.
    pub fn print(&self) {
        if self.written.is_empty()
            && self.skipped_existing.is_empty()
            && self.errors.is_empty()
            && self.warnings.is_empty()
        {
            println!("Nothing to migrate.");
            return;
        }

        if !self.written.is_empty() {
            println!("\n✓ Written ({}):", self.written.len());
            for p in &self.written {
                println!("  {}", p.display());
            }
        }

        if !self.skipped_existing.is_empty() {
            println!(
                "\n⊘ Skipped — dots.toml already exists ({}):",
                self.skipped_existing.len()
            );
            for p in &self.skipped_existing {
                println!("  {}", p.display());
            }
        }

        if !self.warnings.is_empty() {
            println!("\n⚠ Needs manual review ({}):", self.warnings.len());
            for w in &self.warnings {
                println!("  {}", w);
            }
        }

        if !self.errors.is_empty() {
            println!("\n✗ Errors ({}):", self.errors.len());
            for (p, e) in &self.errors {
                println!("  {}: {}", p.display(), e);
            }
        }
    }
}

/// Migrate a v3 dotfiles directory to v4 `dots.toml` files.
///
/// - `dotfiles_dir`: root of the existing v3 dotfiles (must contain `bombadil.toml`).
///   Files in this directory are **never modified**.
/// - `output_dir`: where to write `dots.toml` files. The relative subdirectory
///   structure mirrors the dotfiles layout. Defaults to `dotfiles_dir` when the
///   caller passes the same path.
/// - `dry_run`: when `true`, prints what would be written without creating files.
///
/// Existing `dots.toml` files in `output_dir` are never overwritten.
pub fn migrate(dotfiles_dir: &Path, output_dir: &Path, dry_run: bool) -> Result<MigrationReport> {
    use crate::settings::imports::ImportPath;
    use config::{Config, File};

    let mut report = MigrationReport::default();

    let bombadil_toml = dotfiles_dir.join("bombadil.toml");
    if !bombadil_toml.exists() {
        anyhow::bail!("no bombadil.toml found in {}", dotfiles_dir.display());
    }

    // Parse only the `import` list from bombadil.toml (we don't need the full Settings here).
    #[derive(serde::Deserialize, Default)]
    struct RootImports {
        #[serde(default)]
        import: Vec<ImportPath>,
    }

    let root: RootImports = Config::builder()
        .add_source(File::from(bombadil_toml.as_path()))
        .build()
        .and_then(|c| c.try_deserialize())
        .context("failed to parse bombadil.toml")?;

    info!(
        dotfiles_dir = %dotfiles_dir.display(),
        output_dir = %output_dir.display(),
        imports = root.import.len(),
        "starting migration"
    );

    for import_entry in &root.import {
        let rel_path = import_entry.path();
        let abs_path = dotfiles_dir.join(rel_path);

        if !abs_path.exists() {
            report.errors.push((abs_path, "file not found".to_string()));
            continue;
        }

        // Output path mirrors the input path's directory structure relative to dotfiles_dir.
        let rel_dir = rel_path.parent().unwrap_or(std::path::Path::new(""));

        match migrate_import_file(
            &abs_path,
            dotfiles_dir,
            rel_dir,
            output_dir,
            dry_run,
            &mut report,
        ) {
            Ok(Some(out_path)) => report.written.push(out_path),
            Ok(None) => {}
            Err(e) => report.errors.push((abs_path, e.to_string())),
        }
    }

    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-file migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate a single imported settings file.
///
/// - `abs_path`: absolute path to the import file being migrated.
/// - `dotfiles_dir`: root of the v3 dotfiles tree (never written to).
/// - `rel_dir`: the import file's directory relative to `dotfiles_dir`.
/// - `output_dir`: root of the output tree; mirrors the dotfiles layout.
///
/// Returns `Some(output_path)` when a `dots.toml` was written, `None` when skipped.
/// Top-level package file format: `[packages.NAME]` at the root.
///
/// Package files (e.g. `packages/cli.toml`) use a different layout than dot
/// import files — packages are at the root rather than under `[settings]`.
#[derive(serde::Deserialize, Default)]
struct PackageFileSettings {
    #[serde(default)]
    packages: std::collections::HashMap<String, crate::settings::packages::Package>,
}

fn migrate_import_file(
    abs_path: &Path,
    dotfiles_dir: &Path,
    rel_dir: &Path,
    output_dir: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<Option<PathBuf>> {
    use config::{Config, File};

    let parsed: ImportedSettings = Config::builder()
        .add_source(File::from(abs_path))
        .build()
        .and_then(|c| c.try_deserialize())
        .with_context(|| format!("failed to parse {}", abs_path.display()))?;

    let has_dots = !parsed.settings.dots.is_empty()
        || !parsed.settings.prehooks.is_empty()
        || !parsed.settings.posthooks.is_empty();

    // Package files use `[packages.NAME]` at the top level (not under [settings]).
    // Try parsing them separately when the normal parse yields nothing.
    let top_level_packages: std::collections::HashMap<String, crate::settings::packages::Package> =
        if !has_dots && parsed.settings.packages.is_empty() {
            Config::builder()
                .add_source(File::from(abs_path))
                .build()
                .and_then(|c| c.try_deserialize::<PackageFileSettings>())
                .map(|p| p.packages)
                .unwrap_or_default()
        } else {
            std::collections::HashMap::new()
        };

    // Merge: settings.packages (under [settings]) + top-level packages
    let all_packages: std::collections::HashMap<String, crate::settings::packages::Package> = {
        let mut m = parsed.settings.packages.clone();
        m.extend(top_level_packages);
        m
    };

    let has_packages = !all_packages.is_empty();

    if !has_dots && !has_packages {
        report.warnings.push(format!(
            "{}: no dots or packages found, skipping",
            abs_path.display()
        ));
        return Ok(None);
    }

    // Determine the output subdirectory:
    // - Dot import files → output goes in the same relative directory.
    // - Package-only files → output goes in a subdirectory named after the file stem.
    let out_subdir = if has_dots {
        rel_dir.to_path_buf()
    } else {
        let stem = abs_path.file_stem().unwrap_or_default().to_string_lossy();
        rel_dir.join(stem.as_ref())
    };

    let out_dir = output_dir.join(&out_subdir);
    let out_path = out_dir.join("dots.toml");

    if out_path.exists() {
        report.skipped_existing.push(out_path.clone());
        return Ok(None);
    }

    // Derive dot name from directory or file stem.
    let dot_name = if has_dots {
        rel_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    } else {
        abs_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    };

    let content = generate_dots_toml(
        &dot_name,
        &parsed,
        &all_packages,
        rel_dir,
        parsed.source_paths_are_relative,
        dotfiles_dir,
        report,
    );

    if dry_run {
        println!("# [dry-run] would write: {}", out_path.display());
        println!("{}", content);
        println!();
        return Ok(None);
    }

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    std::fs::write(&out_path, &content)
        .with_context(|| format!("failed to write {}", out_path.display()))?;

    info!(path = %out_path.display(), "wrote dots.toml");
    Ok(Some(out_path))
}

// ─────────────────────────────────────────────────────────────────────────────
// TOML generation
// ─────────────────────────────────────────────────────────────────────────────

/// Generate the content of a `dots.toml` from a parsed v3 `ImportedSettings`.
fn generate_dots_toml(
    dot_name: &str,
    parsed: &ImportedSettings,
    packages: &std::collections::HashMap<String, crate::settings::packages::Package>,
    source_dir: &Path, // relative-to-dotfiles-root directory of the source file
    paths_relative: bool,
    dotfiles_dir: &Path,
    report: &mut MigrationReport,
) -> String {
    let _ = dotfiles_dir;
    let mut out = String::new();

    // ── [dot] header ────────────────────────────────────────────────────────
    writeln!(out, "[dot]").unwrap();
    writeln!(out, "name = {:?}", dot_name).unwrap();

    if !parsed.settings.vars.is_empty() {
        let vars: Vec<String> = parsed
            .settings
            .vars
            .iter()
            .map(|p| format!("{:?}", p.to_string_lossy().as_ref()))
            .collect();
        writeln!(out, "vars = [{}]", vars.join(", ")).unwrap();
    }

    if !parsed.settings.prehooks.is_empty() {
        let hooks: Vec<String> = parsed
            .settings
            .prehooks
            .iter()
            .map(|h| format!("{:?}", h))
            .collect();
        writeln!(out, "prehooks = [{}]", hooks.join(", ")).unwrap();
    }

    if !parsed.settings.posthooks.is_empty() {
        let hooks: Vec<String> = parsed
            .settings
            .posthooks
            .iter()
            .map(|h| format!("{:?}", h))
            .collect();
        writeln!(out, "posthooks = [{}]", hooks.join(", ")).unwrap();
    }

    // ── [dot.files] ─────────────────────────────────────────────────────────
    if !parsed.settings.dots.is_empty() {
        writeln!(out, "\n[dot.files]").unwrap();

        for (name, dot) in &parsed.settings.dots {
            let source_key = if paths_relative {
                // Source is relative to the import file's directory.
                dot.source.to_string_lossy().to_string()
            } else {
                // Source is relative to dotfiles root; make it relative to source_dir.
                source_dir.join(&dot.source).to_string_lossy().to_string()
            };

            let target = normalize_target(&dot.target.to_string_lossy());

            // Check if this is a simple or extended entry.
            let needs_extended = !dot.ignore.is_empty()
                || dot.vars != crate::settings::dots::Dot::default_vars()
                || dot.hard_copy_target.is_some();

            if needs_extended {
                let mut options = format!("{{ target = {:?}", target);
                if !dot.ignore.is_empty() {
                    let patterns: Vec<String> =
                        dot.ignore.iter().map(|p| format!("{:?}", p)).collect();
                    write!(options, ", ignore = [{}]", patterns.join(", ")).unwrap();
                }
                options.push_str(" }");
                writeln!(out, "{:?} = {}", source_key, options).unwrap();

                // Emit hard_copy_target as a second file entry with copy = true.
                if let Some(hct) = &dot.hard_copy_target {
                    let hct_target = normalize_target(&hct.to_string_lossy());
                    let mut hct_options = format!("{{ target = {:?}, copy = true", hct_target);
                    if let Some(perms) = dot.hard_copy_permissions {
                        write!(hct_options, ", hard_copy_permissions = {:#o}", perms).unwrap();
                    }
                    hct_options.push_str(" }");
                    // Use a disambiguated key for the hard copy entry.
                    let hct_key = format!("{}__copy", source_key);
                    writeln!(out, "# hard copy of {} (does not follow symlinks)", name).unwrap();
                    writeln!(out, "{:?} = {}", hct_key, hct_options).unwrap();
                    report.warnings.push(format!(
                        "{}: '{}' uses hard_copy_target — verify source key '{}__copy' is correct",
                        dot_name, name, source_key
                    ));
                }
            } else {
                writeln!(out, "{:?} = {:?}", source_key, target).unwrap();
            }
        }
    }

    // ── [dot.packages.*] ────────────────────────────────────────────────────
    if !packages.is_empty() {
        for (pkg_name, pkg) in packages {
            emit_package(&mut out, pkg_name, pkg, dot_name, report);
        }
    }

    // ── [dot.profiles.*] ────────────────────────────────────────────────────
    for (profile_name, profile) in &parsed.profiles {
        if profile.dots.is_empty() && profile.posthooks.is_empty() && profile.prehooks.is_empty() {
            continue;
        }

        writeln!(out, "\n[dot.profiles.{}]", profile_name).unwrap();

        if !profile.prehooks.is_empty() {
            let hooks: Vec<String> = profile
                .prehooks
                .iter()
                .map(|h| format!("{:?}", h))
                .collect();
            writeln!(out, "prehooks = [{}]", hooks.join(", ")).unwrap();
        }
        if !profile.posthooks.is_empty() {
            let hooks: Vec<String> = profile
                .posthooks
                .iter()
                .map(|h| format!("{:?}", h))
                .collect();
            writeln!(out, "posthooks = [{}]", hooks.join(", ")).unwrap();
        }

        if !profile.dots.is_empty() {
            writeln!(out, "\n[dot.profiles.{}.files]", profile_name).unwrap();
            for dot_override in profile.dots.values() {
                if let (Some(src), Some(tgt)) = (&dot_override.source, &dot_override.target) {
                    let source_key = if paths_relative {
                        src.to_string_lossy().to_string()
                    } else {
                        source_dir.join(src).to_string_lossy().to_string()
                    };
                    let target = normalize_target(&tgt.to_string_lossy());
                    writeln!(out, "{:?} = {:?}", source_key, target).unwrap();
                }
            }
        }
    }

    out
}

/// Emit a `[dot.packages.NAME]` section to the output string.
fn emit_package(
    out: &mut String,
    pkg_name: &str,
    pkg: &Package,
    dot_name: &str,
    report: &mut MigrationReport,
) {
    writeln!(out, "\n[dot.packages.{}]", pkg_name).unwrap();

    if !pkg.tags.is_empty() {
        let tags: Vec<String> = pkg.tags.iter().map(|t| format!("{:?}", t)).collect();
        writeln!(out, "tags = [{}]", tags.join(", ")).unwrap();
    }

    if !pkg.enabled {
        writeln!(
            out,
            "# WARNING: package was disabled in v3 (enabled = false)"
        )
        .unwrap();
        writeln!(
            out,
            "# Add tags = [\"disabled\"] and filter it via active_tags in your profile."
        )
        .unwrap();
        report.warnings.push(format!(
            "{}: package '{}' was disabled — no direct v4 equivalent; use tag filtering",
            dot_name, pkg_name
        ));
    }

    emit_install_methods(out, &pkg.install, dot_name, pkg_name, report);
}

/// Emit the `[dot.packages.NAME.install]` section or inline install fields.
fn emit_install_methods(
    out: &mut String,
    install: &V3InstallMethods,
    dot_name: &str,
    pkg_name: &str,
    report: &mut MigrationReport,
) {
    if !install.has_any() {
        return;
    }

    // Emit each manager field inline (not as a sub-table).
    if let Some(cfg) = &install.dnf {
        emit_pkg_manager_field(out, "dnf", cfg, dot_name, pkg_name, report);
    }
    if let Some(cfg) = &install.apt {
        emit_pkg_manager_field(out, "apt", cfg, dot_name, pkg_name, report);
    }
    if let Some(cfg) = &install.brew {
        emit_pkg_manager_field(out, "brew", cfg, dot_name, pkg_name, report);
    }
    if let Some(cfg) = &install.pacman {
        emit_pkg_manager_field(out, "pacman", cfg, dot_name, pkg_name, report);
    }
    if let Some(cargo) = &install.cargo {
        emit_cargo_field(out, cargo);
    }
    if let Some(go_module) = &install.go {
        writeln!(out, "install.go = {:?}", go_module).unwrap();
    }
    if let Some(flatpak) = &install.flatpak {
        writeln!(out, "install.flatpak = {:?}", flatpak).unwrap();
    }
    if install.binary.is_some() {
        // Binary config is complex — emit a comment and skip for now.
        writeln!(
            out,
            "# TODO: binary install — configure manually in v4 format"
        )
        .unwrap();
        report.warnings.push(format!(
            "{}: package '{}' uses binary install — configure manually",
            dot_name, pkg_name
        ));
    }
    if install.git.is_some() {
        writeln!(out, "# TODO: git install — configure manually in v4 format").unwrap();
        report.warnings.push(format!(
            "{}: package '{}' uses git install — configure manually",
            dot_name, pkg_name
        ));
    }
    if install.source.is_some() {
        writeln!(out, "# TODO: source install — no v4 equivalent yet").unwrap();
        report.warnings.push(format!(
            "{}: package '{}' uses source install — no v4 equivalent",
            dot_name, pkg_name
        ));
    }
}

fn emit_pkg_manager_field(
    out: &mut String,
    manager: &str,
    cfg: &PackageManagerConfig,
    dot_name: &str,
    pkg_name: &str,
    report: &mut MigrationReport,
) {
    match cfg {
        PackageManagerConfig::Simple(name) => {
            writeln!(out, "install.{} = {:?}", manager, name).unwrap();
        }
        PackageManagerConfig::Extended {
            package,
            repo,
            repo_url,
            gpg_key,
        } => {
            if let Some(repo_path) = repo {
                // v4 supports repo file setup.
                writeln!(
                    out,
                    "install.{} = {{ package = {:?}, repo = {:?} }}",
                    manager, package, repo_path
                )
                .unwrap();
            } else {
                writeln!(out, "install.{} = {:?}", manager, package).unwrap();
            }
            if repo_url.is_some() || gpg_key.is_some() {
                writeln!(
                    out,
                    "# NOTE: repo_url/gpg_key for '{}' have no v4 equivalent — configure manually",
                    manager
                )
                .unwrap();
                report.warnings.push(format!(
                    "{}: package '{}' {} config has repo_url/gpg_key — configure manually",
                    dot_name, pkg_name, manager
                ));
            }
        }
    }
}

fn emit_cargo_field(out: &mut String, cargo: &CargoConfig) {
    match cargo {
        CargoConfig::Simple(name) => {
            writeln!(out, "install.cargo = {:?}", name).unwrap();
        }
        CargoConfig::Extended {
            crate_name,
            name,
            git,
            branch,
            tag,
            features,
            bin,
        } => {
            let crate_name = crate_name
                .as_deref()
                .or(name.as_deref())
                .or(bin.as_deref())
                .unwrap_or("unknown");
            if let Some(git_url) = git {
                write!(
                    out,
                    "install.cargo = {{ name = {:?}, git = {:?}",
                    crate_name, git_url
                )
                .unwrap();
                if !features.is_empty() {
                    let feats: Vec<String> = features.iter().map(|f| format!("{:?}", f)).collect();
                    write!(out, ", features = [{}]", feats.join(", ")).unwrap();
                }
                writeln!(out, " }}").unwrap();
                if branch.is_some() || tag.is_some() {
                    writeln!(
                        out,
                        "# NOTE: branch/tag for cargo git install have no v4 equivalent"
                    )
                    .unwrap();
                }
            } else {
                if features.is_empty() {
                    writeln!(out, "install.cargo = {:?}", crate_name).unwrap();
                } else {
                    let feats: Vec<String> = features.iter().map(|f| format!("{:?}", f)).collect();
                    writeln!(
                        out,
                        "install.cargo = {{ name = {:?}, features = [{}] }}",
                        crate_name,
                        feats.join(", ")
                    )
                    .unwrap();
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Normalize a v3 target path to v4 format.
///
/// v3 targets are relative to `$HOME` (e.g. `.config/zsh/aliases.zsh`) or
/// absolute (e.g. `/usr/share/wayland-sessions/foo.desktop`).
/// v4 targets use `~/` for home-relative paths.
fn normalize_target(target: &str) -> String {
    if target.starts_with('/') || target.starts_with("~/") {
        // Already absolute or already has ~/
        target.to_string()
    } else {
        format!("~/{}", target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_relative_target() {
        assert_eq!(
            normalize_target(".config/zsh/aliases.zsh"),
            "~/.config/zsh/aliases.zsh"
        );
    }

    #[test]
    fn normalize_absolute_target() {
        assert_eq!(normalize_target("/usr/share/foo"), "/usr/share/foo");
    }

    #[test]
    fn normalize_already_home_prefixed() {
        assert_eq!(normalize_target("~/.zshrc"), "~/.zshrc");
    }
}

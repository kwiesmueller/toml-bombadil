//! Template rendering for dotfiles.
//!
//! Provides Tera-based template rendering with variable injection and
//! automatic platform context.

use crate::core::{BombadilError, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tera::{Context, Tera};
use tracing::{debug, instrument, warn};

/// Render a template file with the given variables.
#[instrument(skip(vars, secrets))]
pub fn render_file(
    path: &Path,
    vars: &HashMap<String, String>,
    secrets: &HashMap<String, String>,
    profiles: &[String],
) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|e| BombadilError::Io {
        context: format!("reading template from {}", path.display()),
        source: e,
    })?;

    render_string(&content, path, vars, secrets, profiles)
}

/// Render a template string with the given variables.
#[instrument(skip(content, vars, secrets))]
pub fn render_string(
    content: &str,
    source_path: &Path,
    vars: &HashMap<String, String>,
    secrets: &HashMap<String, String>,
    profiles: &[String],
) -> Result<String> {
    let mut context = Context::new();

    // Inject platform context
    inject_platform_context(&mut context);

    // Inject user variables (can override platform vars)
    for (name, value) in vars {
        context.insert(name, value);
    }

    // Inject secrets (with reserved name checks)
    const RESERVED_VARS: &[&str] = &["profiles", "os", "arch", "distro", "hostname", "username"];

    for (name, value) in secrets {
        if RESERVED_VARS.contains(&name.as_str()) {
            warn!(
                name = %name,
                "Cannot insert secret variable: name is reserved"
            );
            continue;
        }
        context.insert(name, value);
    }

    // Inject profiles
    context.insert("profiles", profiles);

    // Render with Tera
    let mut tera = Tera::default();
    let filename = source_path.to_string_lossy();

    tera.add_raw_template(&filename, content)
        .map_err(|e| BombadilError::TemplateRender {
            path: source_path.to_path_buf(),
            cause: e,
        })?;

    tera.render(&filename, &context)
        .map_err(|e| BombadilError::TemplateRender {
            path: source_path.to_path_buf(),
            cause: e,
        })
}

/// Check if a file contains Tera template syntax.
pub fn is_template(content: &str) -> bool {
    content.contains("{{") || content.contains("{%") || content.contains("{#")
}

/// Inject platform context variables into the template context.
fn inject_platform_context(context: &mut Context) {
    // OS detection
    context.insert("os", std::env::consts::OS);
    context.insert("arch", std::env::consts::ARCH);

    // Hostname
    if let Ok(hostname) = hostname::get() {
        context.insert("hostname", &hostname.to_string_lossy().to_string());
    }

    // Username
    if let Ok(username) = std::env::var("USER") {
        context.insert("username", &username);
    }

    // Home directory
    if let Some(home) = dirs::home_dir() {
        context.insert("home", &home.to_string_lossy().to_string());
    }

    // Linux distro detection
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(id) = line.strip_prefix("ID=") {
                    context.insert("distro", id.trim_matches('"'));
                    break;
                }
            }
        }
    }

    debug!("Platform context injected");
}

/// Build a Tera Context from variables and secrets.
pub fn build_context(
    vars: &HashMap<String, String>,
    secrets: &HashMap<String, String>,
    profiles: &[String],
) -> Context {
    let mut context = Context::new();

    inject_platform_context(&mut context);

    for (name, value) in vars {
        context.insert(name, value);
    }

    for (name, value) in secrets {
        context.insert(name, value);
    }

    context.insert("profiles", profiles);

    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_simple_template() {
        let content = "Hello {{ name }}!";
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Tom".to_string());

        let result =
            render_string(content, Path::new("test"), &vars, &HashMap::new(), &[]).unwrap();
        assert_eq!(result, "Hello Tom!");
    }

    #[test]
    fn render_with_platform_vars() {
        let content = "OS: {{ os }}, Arch: {{ arch }}";
        let result = render_string(
            content,
            Path::new("test"),
            &HashMap::new(),
            &HashMap::new(),
            &[],
        )
        .unwrap();

        assert!(result.contains("OS:"));
        assert!(result.contains("Arch:"));
    }

    #[test]
    fn render_with_profiles() {
        let content = "{% for p in profiles %}{{ p }}{% endfor %}";
        let profiles = vec!["work".to_string(), "laptop".to_string()];
        let result = render_string(
            content,
            Path::new("test"),
            &HashMap::new(),
            &HashMap::new(),
            &profiles,
        )
        .unwrap();

        assert_eq!(result, "worklaptop");
    }

    #[test]
    fn is_template_detects_vars() {
        assert!(is_template("Hello {{ name }}"));
        assert!(is_template("{% if true %}{% endif %}"));
        assert!(is_template("{# comment #}"));
        assert!(!is_template("plain text"));
    }

    #[test]
    fn user_vars_override_platform() {
        let content = "{{ os }}";
        let mut vars = HashMap::new();
        vars.insert("os".to_string(), "custom_os".to_string());

        let result =
            render_string(content, Path::new("test"), &vars, &HashMap::new(), &[]).unwrap();
        assert_eq!(result, "custom_os");
    }
}

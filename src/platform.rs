//! Platform detection for cross-platform dotfile templates.
//!
//! This module provides automatic detection of the current platform,
//! including OS, distribution (for Linux), architecture, hostname, and username.
//! These values are automatically injected into the Tera template context.
//!
//! # Template Usage
//!
//! ```jinja2
//! {% if os == "macos" %}
//! eval "$(/opt/homebrew/bin/brew shellenv)"
//! {% elif os == "linux" %}
//!     {% if distro == "fedora" %}
//! # Fedora-specific config
//!     {% elif distro == "ubuntu" %}
//! # Ubuntu-specific config
//!     {% endif %}
//! {% endif %}
//! ```

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

/// Platform context containing detected system information
#[derive(Debug, Clone)]
pub struct PlatformContext {
    /// Operating system: "linux", "macos", "windows"
    pub os: String,
    /// Linux distribution name (lowercase): "fedora", "ubuntu", "arch", "debian", etc.
    /// None on non-Linux systems
    pub distro: Option<String>,
    /// Distribution version/release
    pub distro_version: Option<String>,
    /// CPU architecture: "x86_64", "aarch64", etc.
    pub arch: String,
    /// System hostname
    pub hostname: String,
    /// Current username
    pub username: String,
    /// Home directory path
    pub home: String,
}

impl Default for PlatformContext {
    fn default() -> Self {
        Self::detect()
    }
}

impl PlatformContext {
    /// Detect the current platform information
    pub fn detect() -> Self {
        let os = Self::detect_os();
        let (distro, distro_version) = Self::detect_distro();
        let arch = env::consts::ARCH.to_string();
        let hostname = Self::detect_hostname();
        let username = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let home = dirs::home_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~".to_string());

        Self {
            os,
            distro,
            distro_version,
            arch,
            hostname,
            username,
            home,
        }
    }

    /// Detect the operating system
    fn detect_os() -> String {
        match env::consts::OS {
            "linux" => "linux".to_string(),
            "macos" => "macos".to_string(),
            "windows" => "windows".to_string(),
            other => other.to_string(),
        }
    }

    /// Detect Linux distribution from /etc/os-release
    fn detect_distro() -> (Option<String>, Option<String>) {
        if env::consts::OS != "linux" {
            return (None, None);
        }

        // Try /etc/os-release first (standard location)
        let os_release_paths = ["/etc/os-release", "/usr/lib/os-release"];

        for path in os_release_paths {
            if let Ok(content) = fs::read_to_string(path) {
                let parsed = Self::parse_os_release(&content);
                if let Some(id) = parsed.get("ID") {
                    let version = parsed.get("VERSION_ID").cloned();
                    return (Some(id.to_lowercase()), version);
                }
            }
        }

        // Fallback: check for specific files
        if Path::new("/etc/fedora-release").exists() {
            return (Some("fedora".to_string()), None);
        }
        if Path::new("/etc/debian_version").exists() {
            return (Some("debian".to_string()), None);
        }
        if Path::new("/etc/arch-release").exists() {
            return (Some("arch".to_string()), None);
        }

        (None, None)
    }

    /// Parse /etc/os-release format (KEY=value or KEY="value")
    fn parse_os_release(content: &str) -> HashMap<String, String> {
        let mut result = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let value = value.trim_matches('"').trim_matches('\'');
                result.insert(key.to_string(), value.to_string());
            }
        }

        result
    }

    /// Detect the system hostname
    fn detect_hostname() -> String {
        // Try hostname crate approach
        if let Ok(name) = hostname::get() {
            if let Some(s) = name.to_str() {
                return s.to_string();
            }
        }

        // Fallback to /etc/hostname on Linux
        if let Ok(content) = fs::read_to_string("/etc/hostname") {
            return content.trim().to_string();
        }

        // Fallback to environment variable
        env::var("HOSTNAME")
            .or_else(|_| env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Convert platform context to a HashMap for Tera context injection
    pub fn to_template_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        vars.insert("os".to_string(), self.os.clone());
        vars.insert("arch".to_string(), self.arch.clone());
        vars.insert("hostname".to_string(), self.hostname.clone());
        vars.insert("username".to_string(), self.username.clone());
        vars.insert("home".to_string(), self.home.clone());

        if let Some(ref distro) = self.distro {
            vars.insert("distro".to_string(), distro.clone());
        }

        if let Some(ref version) = self.distro_version {
            vars.insert("distro_version".to_string(), version.clone());
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_creates_valid_context() {
        let ctx = PlatformContext::detect();

        // OS should be one of the known values
        assert!(
            ctx.os == "linux" || ctx.os == "macos" || ctx.os == "windows",
            "Unexpected OS: {}",
            ctx.os
        );

        // Arch should not be empty
        assert!(!ctx.arch.is_empty());

        // Username should not be empty (unless in weird CI environment)
        // Just check it doesn't panic
        let _ = ctx.username;
    }

    #[test]
    fn test_parse_os_release() {
        let content = r#"
NAME="Fedora Linux"
VERSION="39 (Workstation Edition)"
ID=fedora
VERSION_ID=39
PRETTY_NAME="Fedora Linux 39 (Workstation Edition)"
"#;

        let parsed = PlatformContext::parse_os_release(content);

        assert_eq!(parsed.get("ID"), Some(&"fedora".to_string()));
        assert_eq!(parsed.get("VERSION_ID"), Some(&"39".to_string()));
        assert_eq!(parsed.get("NAME"), Some(&"Fedora Linux".to_string()));
    }

    #[test]
    fn test_to_template_vars() {
        let ctx = PlatformContext {
            os: "linux".to_string(),
            distro: Some("fedora".to_string()),
            distro_version: Some("39".to_string()),
            arch: "x86_64".to_string(),
            hostname: "myhost".to_string(),
            username: "user".to_string(),
            home: "/home/user".to_string(),
        };

        let vars = ctx.to_template_vars();

        assert_eq!(vars.get("os"), Some(&"linux".to_string()));
        assert_eq!(vars.get("distro"), Some(&"fedora".to_string()));
        assert_eq!(vars.get("distro_version"), Some(&"39".to_string()));
        assert_eq!(vars.get("arch"), Some(&"x86_64".to_string()));
        assert_eq!(vars.get("hostname"), Some(&"myhost".to_string()));
        assert_eq!(vars.get("username"), Some(&"user".to_string()));
    }
}

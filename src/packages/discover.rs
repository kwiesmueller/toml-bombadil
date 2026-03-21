//! Package discovery (v3 legacy stub).
//!
//! Will be replaced/removed in Phase 4 along with the rest of the v3
//! package management code.

use crate::platform::PlatformContext;
use crate::settings::packages::{InstallMethods, Package};

/// Available install methods discovered for a package.
pub struct DiscoveryMethods {
    pub dnf: Option<String>,
    pub apt: Option<String>,
    pub brew: Option<String>,
    pub pacman: Option<String>,
}

impl DiscoveryMethods {
    pub fn has_any(&self) -> bool {
        self.dnf.is_some()
            || self.apt.is_some()
            || self.brew.is_some()
            || self.pacman.is_some()
    }
}

/// Result of discovering installation methods for a package.
pub struct DiscoveryResult {
    pub messages: Vec<String>,
    pub methods: DiscoveryMethods,
}

impl DiscoveryResult {
    /// Convert the discovery result into a configured Package.
    pub fn to_package(&self, tags: Vec<String>) -> Package {
        use crate::settings::packages::PackageManagerConfig;

        let mut install = InstallMethods::default();

        if let Some(ref name) = self.methods.dnf {
            install.dnf = Some(PackageManagerConfig::Simple(name.clone()));
        }
        if let Some(ref name) = self.methods.apt {
            install.apt = Some(PackageManagerConfig::Simple(name.clone()));
        }
        if let Some(ref name) = self.methods.brew {
            install.brew = Some(PackageManagerConfig::Simple(name.clone()));
        }
        if let Some(ref name) = self.methods.pacman {
            install.pacman = Some(PackageManagerConfig::Simple(name.clone()));
        }

        Package {
            tags,
            install,
            enabled: true,
        }
    }
}

/// Attempt to discover installation methods for a package on the current platform.
#[allow(unused_variables)]
pub fn discover_package(name: &str, platform: &PlatformContext) -> DiscoveryResult {
    // Stub: no automatic discovery implemented yet.
    DiscoveryResult {
        messages: vec![format!(
            "Auto-discovery not yet implemented. Configure '{}' manually.",
            name
        )],
        methods: DiscoveryMethods {
            dnf: None,
            apt: None,
            brew: None,
            pacman: None,
        },
    }
}

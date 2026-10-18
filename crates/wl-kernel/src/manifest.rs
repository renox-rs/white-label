//! The module manifest: which modules a deployment runs (wl-kernel §3).
//!
//! Read once at boot from `client.toml`, never changed at runtime. A module
//! whose dependency is not enabled stops the boot with a message naming both.

use crate::config::ConfigError;

/// One module the platform has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleSpec {
    /// The name in `client.toml`, without the `wl-` prefix.
    pub name: &'static str,
    /// The modules it cannot run without: it reads their records or calls
    /// one of their capabilities that has no no-op fallback.
    pub requires: &'static [&'static str],
}

/// Every client-side module ([ARCHITECTURE.md] §2). The kernel is always on
/// and is not listed.
///
/// [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md
pub const MODULES: &[ModuleSpec] = &[
    ModuleSpec {
        name: "parties",
        requires: &[],
    },
    ModuleSpec {
        name: "documents",
        requires: &[],
    },
    ModuleSpec {
        name: "notify",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "portal",
        requires: &["parties", "notify"],
    },
    ModuleSpec {
        name: "invoicing",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "payments",
        requires: &["invoicing", "parties"],
    },
    ModuleSpec {
        name: "sales",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "ticketing",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "delivery",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "scheduling",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "hr",
        requires: &["parties"],
    },
    ModuleSpec {
        name: "attendance",
        requires: &["hr"],
    },
    ModuleSpec {
        name: "inventory",
        requires: &[],
    },
];

/// The enabled modules, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    enabled: Vec<&'static str>,
}

impl Manifest {
    /// Checks `enabled` (names from `client.toml`): each is known, listed
    /// once, and has what it requires.
    pub fn new(enabled: &[String]) -> Result<Self, ConfigError> {
        let mut names: Vec<&'static str> = Vec::new();
        for name in enabled {
            let spec = spec(name).ok_or_else(|| {
                ConfigError(format!(
                    "[modules] `{name}` is not a module; known: {}",
                    MODULES
                        .iter()
                        .map(|m| m.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            if names.contains(&spec.name) {
                return Err(ConfigError(format!("[modules] `{name}` is listed twice")));
            }
            names.push(spec.name);
        }
        for name in &names {
            let spec = spec(name).expect("checked above");
            for required in spec.requires {
                if !names.contains(required) {
                    return Err(ConfigError(format!(
                        "[modules] `{name}` needs `{required}`; enable it too"
                    )));
                }
            }
        }
        Ok(Manifest { enabled: names })
    }

    /// Whether the module is on. The kernel always is.
    pub fn is_enabled(&self, name: &str) -> bool {
        name == "kernel" || self.enabled.contains(&name)
    }

    /// The enabled modules, in the order of `client.toml`.
    pub fn enabled(&self) -> &[&'static str] {
        &self.enabled
    }
}

fn spec(name: &str) -> Option<&'static ModuleSpec> {
    let name = name.strip_prefix("wl-").unwrap_or(name);
    MODULES.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_f1_property_set_is_complete() {
        let f1 = names(&[
            "parties",
            "documents",
            "notify",
            "portal",
            "invoicing",
            "payments",
            "ticketing",
            "sales",
            "hr",
        ]);
        let manifest = Manifest::new(&f1).unwrap();
        assert!(manifest.is_enabled("payments"));
        assert!(manifest.is_enabled("kernel"));
        assert!(!manifest.is_enabled("inventory"));
    }

    #[test]
    fn a_missing_dependency_names_both_modules() {
        let err = Manifest::new(&names(&["payments", "parties"])).unwrap_err();
        assert_eq!(
            err.0,
            "[modules] `payments` needs `invoicing`; enable it too"
        );
    }

    #[test]
    fn unknown_and_repeated_names_are_refused() {
        assert!(
            Manifest::new(&names(&["billing"]))
                .unwrap_err()
                .0
                .contains("not a module")
        );
        let twice = Manifest::new(&names(&["parties", "wl-parties"])).unwrap_err();
        assert!(twice.0.contains("listed twice"));
    }

    #[test]
    fn every_requirement_is_a_known_module() {
        for module in MODULES {
            for required in module.requires {
                assert!(
                    spec(required).is_some(),
                    "{} requires unknown {required}",
                    module.name
                );
            }
        }
    }
}

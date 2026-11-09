//! `machines.toml`: the builders and what each one does (plan 04.4).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The name of the file at the root of the repository.
pub const FILE_NAME: &str = "machines.toml";

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    /// The machines by name.
    pub machine: BTreeMap<String, Machine>,
}

/// One machine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// `linux` or `macos`.
    pub os: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
    /// The number of cores.
    pub cores: u32,
    /// The accelerator QEMU uses: `kvm`, `hvf` or `tcg`.
    pub accel: String,
    /// The jobs the machine runs.
    pub roles: Vec<String>,
}

impl Machines {
    /// Read `machines.toml` from the repository root.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(FILE_NAME);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse the text of the file and check each machine.
    pub fn parse(text: &str) -> Result<Self, String> {
        let machines: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        for (name, m) in &machines.machine {
            if !matches!(m.os.as_str(), "linux" | "macos") {
                return Err(format!("machine {name}: os {} is not linux or macos", m.os));
            }
            if !matches!(m.arch.as_str(), "x86_64" | "aarch64") {
                return Err(format!(
                    "machine {name}: arch {} is not x86_64 or aarch64",
                    m.arch
                ));
            }
            if !matches!(m.accel.as_str(), "kvm" | "hvf" | "tcg") {
                return Err(format!(
                    "machine {name}: accel {} is not kvm, hvf or tcg",
                    m.accel
                ));
            }
            if m.accel == "hvf" && m.os != "macos" {
                return Err(format!("machine {name}: hvf is only on macos"));
            }
        }
        Ok(machines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_are_checked() {
        let good = r#"
[machine.m4]
os = "macos"
arch = "aarch64"
cores = 10
accel = "hvf"
roles = ["arm64 Q", "9h GUI"]
"#;
        assert_eq!(Machines::parse(good).unwrap().machine["m4"].cores, 10);
        assert!(Machines::parse(&good.replace("\"macos\"", "\"linux\"")).is_err());
        assert!(Machines::parse(&good.replace("\"hvf\"", "\"xen\"")).is_err());
    }
}

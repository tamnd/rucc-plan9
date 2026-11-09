//! `rows.toml`: the architecture rows and the QEMU machine of each (plan 04.4 and 09.4).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The name of the file at the root of the repository.
pub const FILE_NAME: &str = "rows.toml";

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rows {
    /// The rows by `objtype`.
    pub row: BTreeMap<String, Row>,
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// The milestone that adds the row.
    pub from: String,
    /// rucc's name for the architecture.
    pub rucc_arch: String,
    /// The letter of the tree's compiler, such as `6` for `6c`.
    pub letter: String,
    /// The release image to boot, as `objtype.kind` in `sets.toml`.
    pub image: String,
    /// The kernel the image boots, such as `9pc64`.
    pub kernel: String,
    /// The QEMU program.
    pub qemu: String,
    /// The machine, with its options.
    pub machine: String,
    /// The CPU with KVM or HVF.
    pub cpu_hw: String,
    /// The CPU with TCG.
    pub cpu_tcg: String,
    /// The number of virtual CPUs.
    pub cpus: u32,
    /// The memory, in GiB.
    pub memory_gb: u32,
    /// The firmware: `bios` for the QEMU default, or `u-boot`.
    pub firmware: String,
    /// The disk interface, such as `virtio` or `ahci`.
    pub disk: String,
    /// The limit of one test run, in hours (12.2).
    pub run_limit_hours: u32,
}

impl Rows {
    /// Read `rows.toml` from the repository root.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(FILE_NAME);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse the text of the file and check each row.
    pub fn parse(text: &str) -> Result<Self, String> {
        let rows: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        for (name, row) in &rows.row {
            if !row.image.starts_with(&format!("{name}.")) {
                return Err(format!("row {name}: image {} is not of {name}", row.image));
            }
            if row.firmware != "bios" && row.firmware != "u-boot" {
                return Err(format!(
                    "row {name}: firmware {} is not bios or u-boot",
                    row.firmware
                ));
            }
            if row.cpus == 0 || row.memory_gb == 0 {
                return Err(format!("row {name}: no CPUs or no memory"));
            }
        }
        Ok(rows)
    }

    /// The row of `objtype`.
    pub fn get(&self, objtype: &str) -> Result<&Row, String> {
        self.row
            .get(objtype)
            .ok_or_else(|| format!("{FILE_NAME} has no row {objtype}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[row.amd64]
from = "P0"
rucc_arch = "x86_64"
letter = "6"
image = "amd64.qcow2"
kernel = "9pc64"
qemu = "qemu-system-x86_64"
machine = "q35"
cpu_hw = "host"
cpu_tcg = "max"
cpus = 4
memory_gb = 4
firmware = "bios"
disk = "virtio"
run_limit_hours = 4
"#;

    #[test]
    fn a_row_parses_and_is_checked() {
        let rows = Rows::parse(SAMPLE).unwrap();
        assert_eq!(rows.get("amd64").unwrap().letter, "6");
        assert!(rows.get("arm64").is_err());
        let bad = SAMPLE.replace("\"bios\"", "\"uefi\"");
        assert!(Rows::parse(&bad).unwrap_err().contains("uefi"));
        let bad = SAMPLE.replace("amd64.qcow2", "arm64.qcow2");
        assert!(Rows::parse(&bad).is_err());
    }
}

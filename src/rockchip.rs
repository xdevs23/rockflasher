use std::path::PathBuf;
use gpt::partition_types;
use crate::layout::{PartitionDefinition, Placement, KIB, LBA_SIZE};

// https://opensource.rock-chips.com/wiki_Boot_option#The_Pre-bootloader.28IDBLoader.29
const IDBLOADER_START: u64 = 0x40 * LBA_SIZE;
const IDBLOADER_SIZE_ALIGNMENT: u64 = IDBLOADER_START;

const IDBLOADER_PARTNAME: &str = "idbloader";

#[derive(clap::Args, Debug)]
#[group(id = "rockchip")]
pub struct Options {
    // Without this, clap requires the argument even when the optional group is absent.
    #[arg(short, long, required = false, help = format!(
        "Path to the idbloader image of a Rockchip board, written to a partition at sector {} ({} KiB)",
        IDBLOADER_START / LBA_SIZE, IDBLOADER_START / KIB
    ))]
    idbloader: PathBuf,
}

impl Options {
    pub fn board_partitions(&self) -> Result<Vec<Placement>, String> {
        let def = PartitionDefinition::from_file(
            IDBLOADER_PARTNAME, &self.idbloader, partition_types::ANDROID_BOOTLOADER, IDBLOADER_SIZE_ALIGNMENT
        )?;
        Ok(vec![Placement { def, start: IDBLOADER_START }])
    }
}

#[cfg(test)]
mod tests {
    use gpt::partition_types;
    use crate::fixtures::{layout, ScratchDir};

    const UUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

    #[test]
    fn idbloader_takes_settings() {
        let scratch = ScratchDir::new("rockchip-idbloader");
        let idbloader = scratch.file("idbloader.img", 1000);
        let layout = layout(&[
            "--idbloader", &idbloader.display().to_string(),
            "--partition-type", "idbloader:linux",
            "--partition-uuid", &format!("idbloader:{UUID}"),
        ]).unwrap();
        let idbloader = &layout.board_partitions[0].def;
        assert_eq!(idbloader.part_type, partition_types::LINUX_FS);
        assert_eq!(idbloader.part_guid, Some(UUID.parse().unwrap()));
    }
}

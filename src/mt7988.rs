use std::path::{Path, PathBuf};
use gpt::partition_types;
use crate::boards::Board;
use crate::layout::{GptAttributes, PartitionDefinition, Placement, SourceFile, LBA_SIZE, MIB, PART_ALIGNMENT};

const BL2_PARTNAME: &str = "bl2";
const FIP_PARTNAME: &str = "fip";

const BL2_START: u64 = 34 * LBA_SIZE;
const FIP_START: u64 = 4 * MIB;

#[derive(clap::Args, Debug)]
#[group(id = "mt7988", requires = "bl2")]
pub struct Options {
    // Without this, clap requires the argument even when the optional group is absent.
    #[arg(long, required = false, help = format!(
        "Path to the BL2 image for an MT7988 board with its SD card header, \
        written to a bootable partition named bl2 from sector {} to {} MiB",
        BL2_START / LBA_SIZE, FIP_START / MIB
    ))]
    bl2: PathBuf,

    #[arg(long, help = format!(
        "Path to the FIP image for an MT7988 board, written to a partition named fip at {} MiB",
        FIP_START / MIB
    ))]
    fip: Option<PathBuf>,
}

impl Board for Options {
    fn partitions(&self) -> Result<Vec<Placement>, String> {
        let bl2 = bl2(&self.bl2)?;
        let fip = self.fip.as_deref().map(fip).transpose()?;
        Ok([Some(bl2), fip].into_iter().flatten().collect())
    }
}

fn bl2(source_file: &Path) -> Result<Placement, String> {
    let source = SourceFile::read(BL2_PARTNAME, source_file)?;
    let size = FIP_START - BL2_START;
    let def = PartitionDefinition::new(BL2_PARTNAME, Some(source), size, partition_types::LINUX_FS, None)
        .with_attributes(GptAttributes { required: true, legacy_bios_bootable: true });
    Ok(Placement { def, start: BL2_START })
}

fn fip(source_file: &Path) -> Result<Placement, String> {
    let def = PartitionDefinition::from_file(FIP_PARTNAME, source_file, partition_types::LINUX_FS, PART_ALIGNMENT)?
        .with_attributes(GptAttributes { required: true, legacy_bios_bootable: false });
    Ok(Placement { def, start: FIP_START })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{layout, ScratchDir};
    use crate::layout::{Layout, FIRST_PART_ALIGNMENT, KIB};

    fn bl2_layout(bl2_size: u64) -> Result<Layout, String> {
        let scratch = ScratchDir::new(&format!("mt7988-bl2-{bl2_size}"));
        let bl2 = scratch.file("bl2.img", bl2_size);
        layout(&["--bl2", &bl2.display().to_string(), "--no-fill"])
    }

    fn bl2_fip_root_layout(scratch: &ScratchDir, fip_size: u64) -> Layout {
        let bl2 = scratch.file("bl2.img", 12343);
        let fip = scratch.file("fip.bin", fip_size);
        layout(&[
            "--bl2", &bl2.display().to_string(),
            "--fip", &fip.display().to_string(),
            "--blank-partition", "root:3MiB",
            "--no-fill",
        ]).unwrap()
    }

    #[test]
    fn bl2_runs_from_its_start_to_the_fip_and_the_other_partitions_follow() {
        let scratch = ScratchDir::new("mt7988-place");
        let layout = bl2_fip_root_layout(&scratch, 575_120);
        let placements = layout.place(17 * KIB..64 * MIB).unwrap();
        let extents: Vec<_> = placements.iter()
            .map(|placement| (placement.def.partition_name.as_str(), placement.start, placement.end()))
            .collect();
        assert_eq!(extents, [
            ("bl2", BL2_START, FIP_START),
            ("fip", FIP_START, FIP_START + MIB),
            ("root", FIRST_PART_ALIGNMENT, FIRST_PART_ALIGNMENT + 3 * MIB),
        ]);
    }

    #[test]
    fn bl2_is_the_only_legacy_bios_bootable_partition() {
        let scratch = ScratchDir::new("mt7988-bootable");
        let layout = bl2_fip_root_layout(&scratch, 1000);
        let bootable: Vec<_> = layout.definitions()
            .map(|def| (def.partition_name.as_str(), def.attributes.legacy_bios_bootable))
            .collect();
        assert_eq!(bootable, [("bl2", true), ("fip", false), ("root", false)]);
    }

    #[test]
    fn bl2_that_fills_its_partition_is_accepted() {
        assert!(bl2_layout(FIP_START - BL2_START).is_ok());
    }

    #[test]
    fn bl2_larger_than_its_partition_is_refused() {
        let err = bl2_layout(FIP_START - BL2_START + 1).err().unwrap();
        assert!(err.contains("is larger than partition bl2"), "{err}");
    }

    #[test]
    fn empty_bl2_is_refused() {
        let err = bl2_layout(0).err().unwrap();
        assert!(err.contains("for partition bl2 is empty"), "{err}");
    }
}

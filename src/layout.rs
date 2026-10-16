use std::fs::metadata;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use gpt::disk::LogicalBlockSize;
use gpt::partition_types;
use uuid::Uuid;
use crate::values::{ByteSize, PartitionSetting, PartitionType};

pub const LBA: LogicalBlockSize = LogicalBlockSize::Lb512;

pub const LBA_SIZE: u64 = match LBA {
    LogicalBlockSize::Lb512 => 512,
    LogicalBlockSize::Lb4096 => 4096
};

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * KIB;

pub const PART_ALIGNMENT: u64 = MIB;
pub const FIRST_PART_ALIGNMENT: u64 = 8 * MIB;

const USERDATA_PARTNAME: &str = "userdata";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Loader,
    Userdata,
    Other,
}

impl Role {
    fn for_name(partition_name: &str) -> Self {
        match partition_name {
            "uboot" | "bootloader" | "loader" | "trust" => Role::Loader,
            USERDATA_PARTNAME => Role::Userdata,
            _ => Role::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub len: u64,
}

impl SourceFile {
    pub fn stat(path: &Path) -> io::Result<Self> {
        metadata(path).map(|source_metadata| Self { path: path.into(), len: source_metadata.len() })
    }

    pub fn read(partition_name: &str, path: &Path) -> Result<Self, String> {
        Self::stat(path).map_err(|err| format!(
            "Could not read source file {} for partition {}: {}. Check the path.",
            path.display(), partition_name, err
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GptAttributes {
    pub required: bool,
    pub legacy_bios_bootable: bool,
}

#[derive(Clone, Debug)]
pub struct PartitionDefinition {
    pub partition_name: String,
    pub source: Option<SourceFile>,
    pub size: u64,
    pub part_type: partition_types::Type,
    pub part_guid: Option<Uuid>,
    pub attributes: GptAttributes,
}

impl PartitionDefinition {
    pub fn new(
        partition_name: &str,
        source: Option<SourceFile>,
        size: u64,
        part_type: partition_types::Type,
        part_guid: Option<Uuid>,
    ) -> Self {
        Self {
            partition_name: partition_name.into(),
            source,
            size,
            part_type,
            part_guid,
            attributes: GptAttributes::default(),
        }
    }

    fn role(&self) -> Role {
        Role::for_name(&self.partition_name)
    }

    fn named(partition_name: &str, source: Option<SourceFile>, size: u64) -> Self {
        Self::new(partition_name, source, size, partition_name_to_type(partition_name), None)
    }

    pub fn from_file(
        partition_name: &str,
        source_file: &Path,
        part_type: partition_types::Type,
        size_alignment: u64,
    ) -> Result<Self, String> {
        let source = SourceFile::read(partition_name, source_file)?;
        let size = source.len.next_multiple_of(size_alignment);
        Ok(Self::new(partition_name, Some(source), size, part_type, None))
    }

    pub fn with_attributes(self, attributes: GptAttributes) -> Self {
        Self { attributes, ..self }
    }

    fn userdata(size: u64) -> Self {
        Self::named(USERDATA_PARTNAME, None, size)
    }

    fn with_settings(self, settings: Settings) -> Self {
        Self {
            part_type: setting_for(settings.types, &self.partition_name)
                .map_or(self.part_type, |part_type| part_type.0.clone()),
            part_guid: setting_for(settings.uuids, &self.partition_name).copied().or(self.part_guid),
            ..self
        }
    }
}

#[derive(Clone, Copy)]
struct Settings<'a> {
    types: &'a [PartitionSetting<PartitionType>],
    uuids: &'a [PartitionSetting<Uuid>],
}

pub struct LayoutRequest {
    pub board_partitions: Vec<Placement>,
    pub file_partitions: Vec<PartitionSetting<PathBuf>>,
    pub blank_partitions: Vec<PartitionSetting<ByteSize>>,
    pub repart_partitions: Vec<PartitionDefinition>,
    pub types: Vec<PartitionSetting<PartitionType>>,
    pub uuids: Vec<PartitionSetting<Uuid>>,
    pub disk_guid: Option<Uuid>,
    pub fill_userdata: bool,
}

pub struct Layout {
    /// Partitions at the places where the boot chain of a board reads them, in the order of their starts.
    pub board_partitions: Vec<Placement>,
    pub partitions: Vec<PartitionDefinition>,
    pub disk_guid: Option<Uuid>,
    pub fill_userdata: bool,
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub def: PartitionDefinition,
    pub start: u64,
}

impl Placement {
    pub fn end(&self) -> u64 {
        self.start + self.def.size.next_multiple_of(LBA_SIZE)
    }

    pub fn sectors(&self) -> Range<u64> {
        self.start / LBA_SIZE..self.end() / LBA_SIZE
    }

    fn with_settings(self, settings: Settings) -> Self {
        Self { def: self.def.with_settings(settings), ..self }
    }
}

impl Layout {
    pub fn new(request: LayoutRequest) -> Result<Self, String> {
        let settings = Settings { types: &request.types, uuids: &request.uuids };
        let board_partitions = request.board_partitions.into_iter()
            .map(|placement| placement.with_settings(settings))
            .collect();
        let files = request.file_partitions.iter().map(|setting| PartitionDefinition::from_file(
            &setting.partition_name, &setting.value, partition_name_to_type(&setting.partition_name), FIRST_PART_ALIGNMENT
        ));
        let blanks = request.blank_partitions.iter().map(|setting|
            Ok(PartitionDefinition::named(&setting.partition_name, None, setting.value.bytes()))
        );
        let partitions: Vec<_> = files.chain(blanks).collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .chain(request.repart_partitions)
            .map(|def| def.with_settings(settings))
            .collect();

        let fill_userdata = request.fill_userdata && !partitions.iter().any(|def| def.role() == Role::Userdata);
        let layout = Self {
            board_partitions,
            partitions: loaders_first(partitions),
            disk_guid: request.disk_guid,
            fill_userdata,
        };
        layout.check(settings)?;
        Ok(layout)
    }

    fn check(&self, settings: Settings) -> Result<(), String> {
        let definitions: Vec<_> = self.definitions().collect();
        check_partition_names(&definitions)?;
        check_settings("type", settings.types, &definitions)?;
        check_settings("UUID", settings.uuids, &definitions)?;
        check_uuids(self.disk_guid, &definitions)?;
        check_source_files(&definitions)?;
        check_legacy_bios_bootable(&definitions)?;
        check_partition_sizes(&definitions)
    }

    pub fn is_empty(&self) -> bool {
        self.board_partitions.is_empty() && self.partitions.is_empty()
    }

    pub fn definitions(&self) -> impl Iterator<Item = &PartitionDefinition> {
        self.board_partitions.iter().map(|placement| &placement.def).chain(&self.partitions)
    }

    pub fn place(&self, usable: Range<u64>) -> Result<Vec<Placement>, String> {
        let next_free = |placements: &[Placement]| placements.last().map_or(usable.start, Placement::end);
        let mut placements: Vec<Placement> = Vec::new();
        for placement in &self.board_partitions {
            let free = next_free(&placements);
            if placement.start < free {
                return Err(format!(
                    "Partition {} has to start at byte {}, but the partition table or the partition before it \
                    uses the space up to byte {}. This board layout does not fit the partition table. \
                    Report it to the developers of rockflasher.",
                    placement.def.partition_name, placement.start, free
                ))
            }
            placements.push(placement.clone());
        }
        for (index, def) in self.partitions.iter().enumerate() {
            let alignment = if index == 0 { FIRST_PART_ALIGNMENT } else { PART_ALIGNMENT };
            let start = next_free(&placements).next_multiple_of(alignment);
            placements.push(Placement { def: def.clone(), start });
        }

        if self.fill_userdata {
            let start = next_free(&placements).next_multiple_of(PART_ALIGNMENT);
            if start < usable.end {
                placements.push(Placement { def: PartitionDefinition::userdata(usable.end - start), start });
            }
        }

        match placements.iter().find(|placement| placement.end() > usable.end) {
            Some(placement) => Err(format!(
                "Partition {} ends at byte {}, after the end of the usable space at byte {}. \
                Use a larger destination, or smaller partitions.",
                placement.def.partition_name, placement.end(), usable.end
            )),
            None => Ok(placements),
        }
    }
}

fn setting_for<'a, T>(settings: &'a [PartitionSetting<T>], partition_name: &str) -> Option<&'a T> {
    settings.iter()
        .find(|setting| setting.partition_name == partition_name)
        .map(|setting| &setting.value)
}

fn check_settings<T>(
    setting_kind: &str,
    settings: &[PartitionSetting<T>],
    partitions: &[&PartitionDefinition],
) -> Result<(), String> {
    for (index, setting) in settings.iter().enumerate() {
        if !partitions.iter().any(|def| def.partition_name == setting.partition_name) {
            return Err(format!(
                "A partition {0} is set for {1}, which is not in the layout. \
                Add a partition named {1}, or set the {0} for a partition in the layout.",
                setting_kind, setting.partition_name
            ))
        }
        if settings[..index].iter().any(|earlier| earlier.partition_name == setting.partition_name) {
            return Err(format!(
                "The partition {} of {} is set twice. Set it once.",
                setting_kind, setting.partition_name
            ))
        }
    }
    Ok(())
}

fn check_partition_names(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    for (index, def) in partitions.iter().enumerate() {
        if partitions[..index].iter().any(|earlier| earlier.partition_name == def.partition_name) {
            return Err(format!(
                "Two partitions are named {}. Give each partition its own name.",
                def.partition_name
            ))
        }
    }
    Ok(())
}

fn check_uuids(disk_guid: Option<Uuid>, partitions: &[&PartitionDefinition]) -> Result<(), String> {
    for (index, def) in partitions.iter().enumerate() {
        let Some(part_guid) = def.part_guid else { continue };
        if disk_guid == Some(part_guid) {
            return Err(format!(
                "Partition {} has the UUID {} of the disk. Give the disk and each partition its own UUID.",
                def.partition_name, part_guid
            ))
        }
        if let Some(earlier) = partitions[..index].iter().find(|earlier| earlier.part_guid == Some(part_guid)) {
            return Err(format!(
                "Partitions {} and {} have the same UUID {}. Give each partition its own UUID.",
                earlier.partition_name, def.partition_name, part_guid
            ))
        }
    }
    Ok(())
}

fn check_source_files(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    for def in partitions {
        let Some(source) = &def.source else { continue };
        if source.len == 0 {
            return Err(format!(
                "Source file {} for partition {} is empty. Give partition {} a source file that is not empty.",
                source.path.display(), def.partition_name, def.partition_name
            ))
        }
        if source.len > def.size {
            return Err(format!(
                "Source file {} ({} bytes) is larger than partition {} ({} bytes). \
                Give partition {} a source file of at most {} bytes.",
                source.path.display(), source.len, def.partition_name, def.size, def.partition_name, def.size
            ))
        }
    }
    Ok(())
}

fn check_legacy_bios_bootable(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    let mut bootable = partitions.iter().filter(|def| def.attributes.legacy_bios_bootable);
    match (bootable.next(), bootable.next()) {
        (Some(first), Some(second)) => Err(format!(
            "Partitions {} and {} are both legacy BIOS bootable, and the MBR marks one partition active. \
            Mark one of them.",
            first.partition_name, second.partition_name
        )),
        _ => Ok(()),
    }
}

fn check_partition_sizes(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    match partitions.iter().find(|def| def.size == 0) {
        Some(def) => Err(format!(
            "Partition {} would be empty. Give it a size above zero.",
            def.partition_name
        )),
        None => Ok(()),
    }
}

fn loaders_first(partitions: Vec<PartitionDefinition>) -> Vec<PartitionDefinition> {
    let (loaders, others): (Vec<_>, Vec<_>) = partitions.into_iter()
        .partition(|def| def.role() == Role::Loader);
    loaders.into_iter().chain(others).collect()
}

fn partition_name_to_type(name: &str) -> partition_types::Type {
    match Role::for_name(name) {
        Role::Loader => partition_types::ANDROID_BOOTLOADER,
        Role::Userdata => partition_types::ANDROID_DATA,
        Role::Other => match name {
            "system" | "vendor" | "super" | "product" | "odm" => partition_types::ANDROID_SYSTEM,
            "cache" => partition_types::ANDROID_CACHE,
            "boot" | "vendor_boot" | "system_dlkm" | "vendor_dlkm" | "odm_dlkm" |
            "dtb" | "dtbo" | "vbmeta" | "security" | "init_boot" => partition_types::ANDROID_BOOT,
            "recovery" => partition_types::ANDROID_RECOVERY,
            "misc" => partition_types::ANDROID_MISC,
            "metadata" => partition_types::ANDROID_META,
            "factory" | "backup" => partition_types::ANDROID_FACTORY,
            "stage2" | "bootloader2" | "loader2" => partition_types::ANDROID_BOOTLOADER2,
            "fastboot" => partition_types::ANDROID_FASTBOOT,
            "oem" => partition_types::ANDROID_OEM,
            "persist" => partition_types::ANDROID_PERSISTENT,
            _ => partition_types::BASIC
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{layout, ScratchDir};

    const UUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

    fn names(partitions: &[PartitionDefinition]) -> Vec<&str> {
        partitions.iter().map(|def| def.partition_name.as_str()).collect()
    }

    fn with_board_partition(mut layout: Layout, name: &str, start: u64, size: u64) -> Layout {
        let def = PartitionDefinition::new(name, None, size, partition_types::BASIC, None);
        layout.board_partitions.push(Placement { def, start });
        layout
    }

    #[test]
    fn loaders_come_first_whatever_their_type() {
        let layout = layout(&[
            "--blank-partition", "a:1MiB",
            "--blank-partition", "uboot:1MiB",
            "--partition-type", "uboot:linux",
        ]).unwrap();
        assert_eq!(names(&layout.partitions), ["uboot", "a"]);
        assert_eq!(layout.partitions[0].part_type, partition_types::LINUX_FS);
    }

    #[test]
    fn type_follows_the_name_unless_set() {
        let layout = layout(&[
            "--blank-partition", "cache:1MiB",
            "--blank-partition", "esp:1MiB",
            "--partition-type", "esp:esp",
        ]).unwrap();
        assert_eq!(layout.partitions[0].part_type, partition_types::ANDROID_CACHE);
        assert_eq!(layout.partitions[1].part_type, partition_types::EFI);
    }

    #[test]
    fn each_partition_starts_after_the_one_before_it() {
        let scratch = ScratchDir::new("layout-place");
        let uboot = scratch.file("u-boot.itb", MIB);
        let layout = layout(&[
            "--blank-partition", "small:4MiB",
            "--blank-partition", "odd:1000",
            "--blank-partition", "last:1MiB",
            "--partition", &format!("uboot:{}", uboot.display()),
        ]).unwrap();
        let layout = with_board_partition(layout, "loader", 32 * KIB, 224 * KIB);
        let placements = layout.place(17 * KIB..64 * MIB).unwrap();
        let starts: Vec<_> = placements.iter()
            .map(|placement| (placement.def.partition_name.as_str(), placement.start, placement.end()))
            .collect();
        assert_eq!(starts, [
            ("loader", 32 * KIB, 256 * KIB),
            ("uboot", 8 * MIB, 16 * MIB),
            ("small", 16 * MIB, 20 * MIB),
            ("odd", 20 * MIB, 20 * MIB + 1024),
            ("last", 21 * MIB, 22 * MIB),
            ("userdata", 22 * MIB, 64 * MIB),
        ]);
    }

    #[test]
    fn board_partition_before_the_usable_space_is_refused() {
        let layout = with_board_partition(layout(&["--no-fill"]).unwrap(), "loader", 32 * KIB, KIB);
        let err = layout.place(64 * KIB..64 * MIB).err().unwrap();
        assert!(err.contains("loader has to start at byte 32768"), "{err}");
    }

    #[test]
    fn board_partition_that_overlaps_the_one_before_it_is_refused() {
        let layout = with_board_partition(layout(&["--no-fill"]).unwrap(), "first", 32 * KIB, 64 * KIB);
        let layout = with_board_partition(layout, "second", 64 * KIB, KIB);
        let err = layout.place(17 * KIB..64 * MIB).err().unwrap();
        assert!(err.contains("second has to start at byte 65536"), "{err}");
    }

    #[test]
    fn first_partition_starts_at_8_mib_without_idbloader() {
        let layout = layout(&["--blank-partition", "a:1MiB", "--no-fill"]).unwrap();
        let placements = layout.place(17 * KIB..64 * MIB).unwrap();
        assert_eq!(placements[0].start, 8 * MIB);
    }

    #[test]
    fn partition_past_the_usable_space_is_refused() {
        let layout = layout(&["--blank-partition", "a:1MiB", "--no-fill"]).unwrap();
        assert!(layout.place(17 * KIB..9 * MIB - 1).is_err());
    }

    #[test]
    fn userdata_fills_the_disk_unless_given_or_no_fill() {
        let fill = |options: &[&str]| layout(options).unwrap().fill_userdata;
        assert!(fill(&["--blank-partition", "a:1MiB"]));
        assert!(!fill(&["--blank-partition", "a:1MiB", "--no-fill"]));
        assert!(!fill(&["--blank-partition", "userdata:1MiB", "--partition-type", "userdata:linux"]));
    }

    #[test]
    fn setting_for_an_unknown_partition_is_refused() {
        let result = layout(&["--blank-partition", "a:1MiB", "--partition-type", "b:linux"]);
        assert!(result.is_err());
    }

    #[test]
    fn setting_given_twice_is_refused() {
        let result = layout(&[
            "--blank-partition", "a:1MiB",
            "--partition-type", "a:linux",
            "--partition-type", "a:esp",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn one_name_on_two_partitions_is_refused() {
        let scratch = ScratchDir::new("layout-names");
        let idbloader = scratch.file("idbloader.img", 1000);
        let repart = scratch.path().join("repart.json");
        let esp = serde_json::json!({ "type": "esp", "label": "esp", "uuid": UUID, "raw_size": 4096 });
        std::fs::write(&repart, serde_json::Value::from([esp]).to_string()).unwrap();
        let idbloader = idbloader.display().to_string();
        let repart = repart.display().to_string();
        let refused = |options: &[&str]| layout(options).err().is_some_and(|err| err.contains("Two partitions are named"));
        assert!(refused(&["--blank-partition", "a:1MiB", "--blank-partition", "a:2MiB"]));
        assert!(refused(&["--partition", &format!("a:{idbloader}"), "--blank-partition", "a:1MiB"]));
        assert!(refused(&["--repart", &repart, "--blank-partition", "esp:1MiB"]));
        assert!(refused(&["--idbloader", &idbloader, "--blank-partition", "idbloader:1MiB"]));
    }

    #[test]
    fn one_uuid_on_two_partitions_is_refused() {
        let result = layout(&[
            "--blank-partition", "a:1MiB",
            "--blank-partition", "b:1MiB",
            "--partition-uuid", &format!("a:{UUID}"),
            "--partition-uuid", &format!("b:{UUID}"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn partition_uuid_equal_to_the_disk_uuid_is_refused() {
        let err = layout(&[
            "--blank-partition", "a:1MiB",
            "--disk-uuid", UUID,
            "--partition-uuid", &format!("a:{UUID}"),
        ]).err().unwrap();
        assert!(err.contains("Partition a has the UUID"), "{err}");
    }

    #[test]
    fn repart_partitions_follow_the_loaders_and_take_settings() {
        let scratch = ScratchDir::new("layout-repart");
        let uboot = scratch.file("u-boot.itb", 1000);
        let repart = scratch.path().join("repart.json");
        let esp = serde_json::json!({ "type": "esp", "label": "ESP", "uuid": UUID, "raw_size": 4096 });
        std::fs::write(&repart, serde_json::Value::from([esp]).to_string()).unwrap();
        let layout = layout(&[
            "--repart", &repart.display().to_string(),
            "--blank-partition", "a:1MiB",
            "--partition", &format!("uboot:{}", uboot.display()),
            "--partition-type", "ESP:linux",
        ]).unwrap();
        assert_eq!(names(&layout.partitions), ["uboot", "a", "ESP"]);
        assert_eq!(layout.partitions[2].part_type, partition_types::LINUX_FS);
        assert_eq!(layout.partitions[2].part_guid, Some(UUID.parse().unwrap()));
    }

    #[test]
    fn empty_source_file_is_refused() {
        let scratch = ScratchDir::new("layout-empty");
        let empty = scratch.file("empty.img", 0);
        let err = layout(&["--partition", &format!("a:{}", empty.display())]).err().unwrap();
        assert!(err.contains("for partition a is empty"), "{err}");
    }

    #[test]
    fn source_file_larger_than_its_partition_is_refused() {
        let scratch = ScratchDir::new("layout-large-source");
        scratch.file("disk.img.esp", 8192);
        let repart = scratch.path().join("repart.json");
        let esp = serde_json::json!({
            "type": "esp", "label": "ESP", "uuid": UUID, "raw_size": 4096, "split_path": "disk.img.esp",
        });
        std::fs::write(&repart, serde_json::Value::from([esp]).to_string()).unwrap();
        let err = layout(&["--repart", &repart.display().to_string()]).err().unwrap();
        assert!(err.contains("(8192 bytes) is larger than partition ESP (4096 bytes)"), "{err}");
    }

    #[test]
    fn two_legacy_bios_bootable_partitions_are_refused() {
        let mut layout = layout(&["--blank-partition", "a:1MiB", "--blank-partition", "b:1MiB"]).unwrap();
        let settings = Settings { types: &[], uuids: &[] };
        assert!(layout.check(settings).is_ok());
        for def in &mut layout.partitions {
            def.attributes.legacy_bios_bootable = true;
        }
        let err = layout.check(settings).err().unwrap();
        assert!(err.contains("Partitions a and b are both legacy BIOS bootable"), "{err}");
    }
}

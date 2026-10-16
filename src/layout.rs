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

const PART_ALIGNMENT: u64 = 1024 * 1024;
pub const FIRST_PART_ALIGNMENT: u64 = 8 * 1024 * 1024;

// https://opensource.rock-chips.com/wiki_Boot_option#The_Pre-bootloader.28IDBLoader.29
const IDBLOADER_ALIGNMENT_LBA: u64 = 0x40;
const IDBLOADER_ALIGNMENT: u64 = IDBLOADER_ALIGNMENT_LBA * LBA_SIZE;

const IDBLOADER_PARTNAME: &str = "idbloader";
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
            "uboot" | "bootloader" | "loader" | "trust" | IDBLOADER_PARTNAME => Role::Loader,
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

#[derive(Clone, Debug)]
pub struct PartitionDefinition {
    pub partition_name: String,
    pub source: Option<SourceFile>,
    pub size: u64,
    pub part_type: partition_types::Type,
    pub part_guid: Option<Uuid>,
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
        }
    }

    fn role(&self) -> Role {
        Role::for_name(&self.partition_name)
    }

    fn named(partition_name: &str, source: Option<SourceFile>, size: u64) -> Self {
        Self::new(partition_name, source, size, partition_name_to_type(partition_name), None)
    }

    fn from_file(partition_name: &str, source_file: &Path, alignment: u64) -> Result<Self, String> {
        let source = SourceFile::read(partition_name, source_file)?;
        let size = source.len.next_multiple_of(alignment);
        Ok(Self::named(partition_name, Some(source), size))
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
    pub idbloader: Option<PathBuf>,
    pub file_partitions: Vec<PartitionSetting<PathBuf>>,
    pub blank_partitions: Vec<PartitionSetting<ByteSize>>,
    pub repart_partitions: Vec<PartitionDefinition>,
    pub types: Vec<PartitionSetting<PartitionType>>,
    pub uuids: Vec<PartitionSetting<Uuid>>,
    pub fill_userdata: bool,
}

pub struct Layout {
    pub idbloader: Option<PartitionDefinition>,
    pub partitions: Vec<PartitionDefinition>,
    pub fill_userdata: bool,
}

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
}

impl Layout {
    pub fn new(request: LayoutRequest) -> Result<Self, String> {
        let settings = Settings { types: &request.types, uuids: &request.uuids };
        let idbloader = request.idbloader.as_deref()
            .map(|idbloader| PartitionDefinition::from_file(IDBLOADER_PARTNAME, idbloader, IDBLOADER_ALIGNMENT))
            .transpose()?
            .map(|idbloader| idbloader.with_settings(settings));
        let files = request.file_partitions.iter().map(|setting|
            PartitionDefinition::from_file(&setting.partition_name, &setting.value, FIRST_PART_ALIGNMENT)
        );
        let blanks = request.blank_partitions.iter().map(|setting|
            Ok(PartitionDefinition::named(&setting.partition_name, None, setting.value.bytes()))
        );
        let partitions: Vec<_> = files.chain(blanks).collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .chain(request.repart_partitions)
            .map(|def| def.with_settings(settings))
            .collect();

        let definitions: Vec<_> = idbloader.iter().chain(&partitions).collect();
        check_partition_names(&definitions)?;
        check_settings("type", settings.types, &definitions)?;
        check_settings("UUID", settings.uuids, &definitions)?;
        check_partition_uuids(&definitions)?;
        check_source_files(&definitions)?;
        check_partition_sizes(&definitions)?;

        let fill_userdata = request.fill_userdata && !partitions.iter().any(|def| def.role() == Role::Userdata);
        Ok(Self { idbloader, partitions: loaders_first(partitions), fill_userdata })
    }

    pub fn is_empty(&self) -> bool {
        self.idbloader.is_none() && self.partitions.is_empty()
    }

    pub fn place(&self, usable: Range<u64>) -> Result<Vec<Placement>, String> {
        let idbloader = self.idbloader.iter().map(|def| (def.clone(), IDBLOADER_ALIGNMENT));
        let partitions = self.partitions.iter().enumerate().map(|(index, def)| {
            (def.clone(), if index == 0 { FIRST_PART_ALIGNMENT } else { PART_ALIGNMENT })
        });
        let mut placements: Vec<Placement> = Vec::new();
        for (def, alignment) in idbloader.chain(partitions) {
            let start = placements.last().map_or(usable.start, Placement::end).next_multiple_of(alignment);
            placements.push(Placement { def, start });
        }

        if self.fill_userdata {
            let start = placements.last().map_or(usable.start, Placement::end).next_multiple_of(PART_ALIGNMENT);
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
                "A partition {} is set for {}, which is not in the layout. \
                Set it for a file partition, a blank partition, a systemd-repart partition or {}.",
                setting_kind, setting.partition_name, IDBLOADER_PARTNAME
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

fn check_partition_uuids(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    for (index, def) in partitions.iter().enumerate() {
        let Some(part_guid) = def.part_guid else { continue };
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
    const MIB: u64 = 1024 * 1024;

    fn names(partitions: &[PartitionDefinition]) -> Vec<&str> {
        partitions.iter().map(|def| def.partition_name.as_str()).collect()
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
        let idbloader = scratch.file("idbloader.img", 200 * 1024);
        let uboot = scratch.file("u-boot.itb", 1024 * 1024);
        let layout = layout(&[
            "--idbloader", &idbloader.display().to_string(),
            "--blank-partition", "small:4MiB",
            "--blank-partition", "odd:1000",
            "--blank-partition", "last:1MiB",
            "--partition", &format!("uboot:{}", uboot.display()),
        ]).unwrap();
        let placements = layout.place(17 * 1024..64 * MIB).unwrap();
        let starts: Vec<_> = placements.iter()
            .map(|placement| (placement.def.partition_name.as_str(), placement.start, placement.end()))
            .collect();
        assert_eq!(starts, [
            ("idbloader", 32 * 1024, 32 * 1024 + 224 * 1024),
            ("uboot", 8 * MIB, 16 * MIB),
            ("small", 16 * MIB, 20 * MIB),
            ("odd", 20 * MIB, 20 * MIB + 1024),
            ("last", 21 * MIB, 22 * MIB),
            ("userdata", 22 * MIB, 64 * MIB),
        ]);
    }

    #[test]
    fn first_partition_starts_at_8_mib_without_idbloader() {
        let layout = layout(&["--blank-partition", "a:1MiB", "--no-fill"]).unwrap();
        let placements = layout.place(17 * 1024..64 * MIB).unwrap();
        assert_eq!(placements[0].start, 8 * MIB);
    }

    #[test]
    fn partition_past_the_usable_space_is_refused() {
        let layout = layout(&["--blank-partition", "a:1MiB", "--no-fill"]).unwrap();
        assert!(layout.place(17 * 1024..9 * MIB - 1).is_err());
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
    fn idbloader_takes_settings() {
        let scratch = ScratchDir::new("layout-idbloader");
        let idbloader = scratch.file("idbloader.img", 1000);
        let layout = layout(&[
            "--idbloader", &idbloader.display().to_string(),
            "--partition-type", "idbloader:linux",
            "--partition-uuid", &format!("idbloader:{UUID}"),
        ]).unwrap();
        let idbloader = layout.idbloader.unwrap();
        assert_eq!(idbloader.part_type, partition_types::LINUX_FS);
        assert_eq!(idbloader.part_guid, Some(UUID.parse().unwrap()));
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
}

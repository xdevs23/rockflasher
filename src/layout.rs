use std::fs::metadata;
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
pub struct PartitionDefinition {
    pub partition_name: String,
    pub source_file: Option<PathBuf>,
    pub size: u64,
    pub part_type: partition_types::Type,
    pub part_guid: Option<Uuid>,
}

impl PartitionDefinition {
    pub fn new(
        partition_name: &str,
        source_file: Option<PathBuf>,
        size: u64,
        part_type: partition_types::Type,
        part_guid: Option<Uuid>,
    ) -> Self {
        Self {
            partition_name: partition_name.into(),
            source_file,
            size,
            part_type,
            part_guid,
        }
    }

    fn role(&self) -> Role {
        Role::for_name(&self.partition_name)
    }

    fn named(partition_name: &str, source_file: Option<PathBuf>, size: u64) -> Self {
        Self::new(partition_name, source_file, size, partition_name_to_type(partition_name), None)
    }

    fn from_file(partition_name: &str, source_file: &Path, alignment: u64) -> Result<Self, String> {
        let size = metadata(source_file)
            .map(|source_metadata| source_metadata.len().next_multiple_of(alignment))
            .map_err(|err| format!(
                "Could not read source file {} of partition {}: {}. Check the path.",
                source_file.display(), partition_name, err
            ))?;
        Ok(Self::named(partition_name, Some(source_file.into()), size))
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

fn check_partition_sizes(partitions: &[&PartitionDefinition]) -> Result<(), String> {
    match partitions.iter().find(|def| def.size == 0) {
        Some(def) => Err(format!(
            "Partition {} would be empty. Give it a source file that is not empty, or a size above zero.",
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


use std::fmt::Display;
use std::num::NonZeroU64;
use std::str::FromStr;
use gpt::partition_types;
use parse_size::parse_size;

// linux, and the names systemd-repart prints for the discoverable partition types that the gpt crate defines
const TYPE_NAMES: [(&str, partition_types::Type); 11] = [
    ("linux", partition_types::LINUX_FS),
    ("esp", partition_types::EFI),
    ("xbootldr", partition_types::FREEDESK_BOOT),
    ("linux-generic", partition_types::LINUX_FS),
    ("swap", partition_types::LINUX_SWAP),
    ("home", partition_types::LINUX_HOME),
    ("srv", partition_types::LINUX_SRV),
    ("root-x86", partition_types::LINUX_ROOT_X86),
    ("root-x86-64", partition_types::LINUX_ROOT_X64),
    ("root-arm", partition_types::LINUX_ROOT_ARM_32),
    ("root-arm64", partition_types::LINUX_ROOT_ARM_64),
];

#[derive(Clone, Debug)]
pub struct PartitionType(pub partition_types::Type);

impl FromStr for PartitionType {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        TYPE_NAMES.iter()
            .find(|(name, _)| *name == text)
            .map_or_else(|| partition_types::Type::from_str(text), |(_, part_type)| Ok(part_type.clone()))
            .map(PartitionType)
            .map_err(|_| format!(
                "Partition type {text} is not supported. Use {}, a type name of the gpt crate or a type GUID.",
                TYPE_NAMES.map(|(name, _)| name).join(", ")
            ))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ByteSize(NonZeroU64);

impl ByteSize {
    pub fn bytes(self) -> u64 {
        self.0.get()
    }
}

impl FromStr for ByteSize {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let size = parse_size(text).map_err(|err| err.to_string())?;
        NonZeroU64::new(size).map(ByteSize).ok_or_else(|| "A size of zero is not possible. Give a size above zero.".into())
    }
}

#[derive(Clone, Debug)]
pub struct Filesystem(String);

impl Filesystem {
    pub fn mkfs_program(&self) -> String {
        format!("mkfs.{}", self.0)
    }
}

impl Display for Filesystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Filesystem {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if !text.is_empty() && text.chars().all(|char| char.is_ascii_alphanumeric()) {
            Ok(Self(text.into()))
        } else {
            Err(format!(
                "{text} is not a filesystem name. Use letters and digits only, such as ext4 or vfat."
            ))
        }
    }
}

#[derive(Clone, Debug)]
pub struct PartitionSetting<T> {
    pub partition_name: String,
    pub value: T,
}

impl<T: FromStr> FromStr for PartitionSetting<T> where T::Err: Display {
    type Err = String;

    fn from_str(arg: &str) -> Result<Self, String> {
        let (partition_name, value) = arg.split_once(':')
            .ok_or_else(|| format!("Invalid argument {arg}, expected <name>:<value>"))?;
        let value = value.parse()
            .map_err(|err| format!("Invalid value in {arg}: {err}"))?;
        Ok(Self { partition_name: partition_name.into(), value })
    }
}


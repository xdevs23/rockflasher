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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use uuid::Uuid;
    use super::*;

    #[test]
    fn setting_splits_at_the_first_colon() {
        let setting: PartitionSetting<PathBuf> = "boot:images/a:b.img".parse().unwrap();
        assert_eq!(setting.partition_name, "boot");
        assert_eq!(setting.value, PathBuf::from("images/a:b.img"));
    }

    #[test]
    fn setting_parses_its_value() {
        let setting: PartitionSetting<ByteSize> = "cache:384MiB".parse().unwrap();
        assert_eq!(setting.partition_name, "cache");
        assert_eq!(setting.value.bytes(), 384 * 1024 * 1024);
    }

    #[test]
    fn setting_without_colon_is_refused() {
        assert!("cache".parse::<PartitionSetting<ByteSize>>().is_err());
    }

    #[test]
    fn setting_with_invalid_value_is_refused() {
        assert!("esp:not-a-uuid".parse::<PartitionSetting<Uuid>>().is_err());
        assert!("cache:12XB".parse::<PartitionSetting<ByteSize>>().is_err());
    }

    #[test]
    fn size_of_zero_is_refused() {
        assert!("0".parse::<ByteSize>().is_err());
        assert!("0MiB".parse::<ByteSize>().is_err());
    }

    #[test]
    fn filesystem_is_a_name_of_letters_and_digits() {
        assert_eq!("ext4".parse::<Filesystem>().unwrap().mkfs_program(), "mkfs.ext4");
        assert!("".parse::<Filesystem>().is_err());
        assert!("../ext4".parse::<Filesystem>().is_err());
        assert!("ext4 -F".parse::<Filesystem>().is_err());
    }

    #[test]
    fn partition_type_reads_table_names_crate_names_and_guids() {
        assert_eq!("esp".parse::<PartitionType>().unwrap().0, partition_types::EFI);
        assert_eq!("linux".parse::<PartitionType>().unwrap().0, partition_types::LINUX_FS);
        assert_eq!("linux-generic".parse::<PartitionType>().unwrap().0, partition_types::LINUX_FS);
        assert_eq!("root-arm64".parse::<PartitionType>().unwrap().0, partition_types::LINUX_ROOT_ARM_64);
        assert_eq!("linux_swap".parse::<PartitionType>().unwrap().0, partition_types::LINUX_SWAP);
        assert_eq!(
            "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".parse::<PartitionType>().unwrap().0,
            partition_types::EFI
        );
        assert!("no-such-type".parse::<PartitionType>().unwrap_err().contains("is not supported"));
    }
}

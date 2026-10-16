use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;
use crate::layout::{PartitionDefinition, SourceFile};
use crate::values::PartitionType;

#[derive(Deserialize)]
struct RepartPartition {
    label: String,
    #[serde(rename = "type", deserialize_with = "partition_type")]
    part_type: PartitionType,
    uuid: Uuid,
    raw_size: u64,
    #[serde(default, deserialize_with = "split_path")]
    split_path: Option<PathBuf>,
}

impl RepartPartition {
    fn into_definition(self, directory: &Path) -> Result<PartitionDefinition, String> {
        let source = self.split_path.as_deref()
            .map(|split_path| self.split_file(&directory.join(split_path)))
            .transpose()?;
        Ok(PartitionDefinition::new(&self.label, source, self.raw_size, self.part_type.0, Some(self.uuid)))
    }

    fn split_file(&self, path: &Path) -> Result<SourceFile, String> {
        SourceFile::stat(path).map_err(|err| format!(
            "Could not read split file {} for partition {}: {}. \
            Pass the output of the systemd-repart run that wrote the split files.",
            path.display(), self.label, err
        ))
    }
}

fn partition_type<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PartitionType, D::Error> {
    String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
}

// systemd-repart prints "-" for a partition without a split file
fn split_path<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<PathBuf>, D::Error> {
    let split_path = Option::<String>::deserialize(deserializer)?;
    Ok(split_path.filter(|split_path| split_path != "-").map(PathBuf::from))
}

pub fn read_partitions(path: &Path) -> Result<Vec<PartitionDefinition>, String> {
    let partitions: Vec<RepartPartition> = File::open(path)
        .map_err(|err| err.to_string())
        .and_then(|file| serde_json::from_reader(BufReader::new(file)).map_err(|err| err.to_string()))
        .map_err(|err| format!(
            "Could not read systemd-repart output {}: {}. \
            Pass the JSON that systemd-repart prints with --json=short or --json=pretty.",
            path.display(), err
        ))?;
    let directory = path.parent().unwrap_or(Path::new(""));
    partitions.into_iter().map(|partition| partition.into_definition(directory)).collect()
}

#[cfg(test)]
mod tests {
    use gpt::partition_types;
    use serde_json::{json, Value};
    use super::*;
    use crate::fixtures::ScratchDir;

    const ESP_UUID: &str = "e4c58bcb-adc5-4abb-aab4-d7ceca5bb687";
    const MAIN_UUID: &str = "cd8d7b01-ea83-431e-868f-25319dc3267e";
    const SWAP_UUID: &str = "717b2fb7-2aca-4080-b495-15fd8c9a46fa";
    const CUSTOM_TYPE: &str = "a2a0d0eb-e5b9-3344-87c0-68b6b72699c7";

    fn write_output(scratch: &ScratchDir, partitions: &[Value]) -> PathBuf {
        let path = scratch.path().join("repart.json");
        std::fs::write(&path, Value::from(partitions).to_string()).unwrap();
        path
    }

    fn partition(part_type: &str, label: &str, uuid: &str, raw_size: u64, split_path: &str) -> Value {
        json!({
            "type": part_type, "label": label, "uuid": uuid, "partno": 0, "offset": 1_048_576,
            "raw_size": raw_size, "activity": "create", "split_path": split_path,
        })
    }

    #[test]
    fn partitions_take_label_type_uuid_size_and_split_file() {
        let scratch = ScratchDir::new("repart-read");
        let esp = scratch.file("disk.img.esp", 4096);
        let main = scratch.file("disk.img.linux-generic", 8192);
        let path = write_output(&scratch, &[
            partition("esp", "ESP", ESP_UUID, 4096, "disk.img.esp"),
            partition("linux-generic", "main", MAIN_UUID, 16384, &main.display().to_string()),
            partition(CUSTOM_TYPE, "swap", SWAP_UUID, 2048, "-"),
        ]);

        let partitions = read_partitions(&path).unwrap();

        let fields: Vec<_> = partitions.iter()
            .map(|def| (def.partition_name.as_str(), def.part_type.guid, def.part_guid, def.size, def.source.as_ref().map(|source| source.path.clone())))
            .collect();
        let uuid = |text: &str| text.parse::<Uuid>().unwrap();
        assert_eq!(fields, [
            ("ESP", partition_types::EFI.guid, Some(uuid(ESP_UUID)), 4096, Some(esp)),
            ("main", partition_types::LINUX_FS.guid, Some(uuid(MAIN_UUID)), 16384, Some(main)),
            ("swap", uuid(CUSTOM_TYPE), Some(uuid(SWAP_UUID)), 2048, None),
        ]);
    }

    #[test]
    fn unknown_type_name_is_refused() {
        let scratch = ScratchDir::new("repart-unknown-type");
        let path = write_output(&scratch, &[partition("var", "var", MAIN_UUID, 4096, "-")]);
        assert!(read_partitions(&path).unwrap_err().contains("Partition type var is not supported"));
    }

    #[test]
    fn missing_split_file_is_refused() {
        let scratch = ScratchDir::new("repart-missing-split");
        let path = write_output(&scratch, &[partition("esp", "ESP", ESP_UUID, 4096, "disk.img.esp")]);
        assert!(read_partitions(&path).unwrap_err().contains("Could not read split file"));
    }
}

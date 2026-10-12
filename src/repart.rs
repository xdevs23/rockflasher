use std::fs::{metadata, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;
use crate::layout::PartitionDefinition;
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
        let split_file = self.split_path.as_deref().map(|split_path| directory.join(split_path));
        if let Some(split_file) = &split_file {
            self.check_split_file(split_file)?;
        }
        Ok(PartitionDefinition::new(&self.label, split_file, self.raw_size, self.part_type.0, Some(self.uuid)))
    }

    fn check_split_file(&self, split_file: &Path) -> Result<(), String> {
        let length = metadata(split_file)
            .map_err(|err| format!(
                "Could not read split file {} of partition {}: {}. \
                Pass the output of the systemd-repart run that wrote the split files.",
                split_file.display(), self.label, err
            ))?
            .len();
        if length > self.raw_size {
            return Err(format!(
                "Split file {} ({} bytes) is larger than partition {} ({} bytes). \
                Pass the output of the systemd-repart run that wrote the split files.",
                split_file.display(), length, self.label, self.raw_size
            ))
        }
        Ok(())
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


use std::path::PathBuf;
use clap::Parser;
use uuid::Uuid;
use crate::boards::Boards;
use crate::layout::LayoutRequest;
use crate::repart;
use crate::values::{ByteSize, Filesystem, PartitionSetting, PartitionType};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Add a partition with the contents of a file (name:file). The first partition starts at 8 MiB, and loader partitions such as uboot come first
    #[arg(short, long)]
    pub partition: Vec<PartitionSetting<PathBuf>>,

    /// Set the GPT type of a partition (name:type): linux, a partition type name of systemd-repart such as esp or linux-generic, a type name of the gpt crate or a type GUID
    #[arg(long)]
    pub partition_type: Vec<PartitionSetting<PartitionType>>,

    /// Set the partition UUID of a partition (name:uuid)
    #[arg(long)]
    pub partition_uuid: Vec<PartitionSetting<Uuid>>,

    /// Add an empty partition (name:size)
    #[arg(short, long)]
    pub blank_partition: Vec<PartitionSetting<ByteSize>>,

    /// Add the partitions listed in the JSON output of systemd-repart (--json=short or pretty), each named by its label, with its type, UUID and size, and its split file as contents
    #[arg(long)]
    pub repart: Option<PathBuf>,

    /// Disk or image file to write to
    #[arg(short, long)]
    pub destination: PathBuf,

    /// Format a partition (name:filesystem), for use with --blank-partition
    #[arg(short, long)]
    pub format_partition: Vec<PartitionSetting<Filesystem>>,

    /// Size of an image file destination. Without it, the image file ends after its last partition, which needs --no-fill
    #[arg(short, long)]
    pub size: Option<ByteSize>,

    #[command(flatten)]
    boards: Boards,

    /// Leave the space after the last partition free instead of adding a userdata partition
    #[arg(long)]
    pub no_fill: bool,
}

impl Args {
    pub fn layout_request(&self) -> Result<LayoutRequest, String> {
        Ok(LayoutRequest {
            board_partitions: self.boards.partitions()?,
            file_partitions: self.partition.clone(),
            blank_partitions: self.blank_partition.clone(),
            repart_partitions: self.repart.as_deref().map_or(Ok(Vec::new()), repart::read_partitions)?,
            types: self.partition_type.clone(),
            uuids: self.partition_uuid.clone(),
            fill_userdata: !self.no_fill,
        })
    }
}

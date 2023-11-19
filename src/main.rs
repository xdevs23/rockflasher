use std::collections::BTreeMap;
use std::fs::{File, metadata, OpenOptions};
use std::io;
use std::io::{copy, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use block_utils::{BlockResult, get_device_info, is_block_device};
use clap::Parser;
use gpt::disk::LogicalBlockSize;
use gpt::partition::Partition;
use gpt::partition_types;
use parse_size::parse_size;
use sizes::BinarySize;
use spinner::SpinnerBuilder;
use crate::alignment::align_up;

pub mod alignment;

const LBA: LogicalBlockSize = LogicalBlockSize::Lb512;

const LBA_SIZE: u64 = match LBA {
    LogicalBlockSize::Lb512 => 512,
    LogicalBlockSize::Lb4096 => 4096
};

const PART_ALIGNMENT: u64 = 1 * 1024 * 1024;
const FIRST_PART_ALIGNMENT: u64 = 8 * 1024 * 1024;

// https://opensource.rock-chips.com/wiki_Boot_option#The_Pre-bootloader.28IDBLoader.29
const IDBLOADER_ALIGNMENT_LBA: u64 = 0x40;
const IDBLOADER_ALIGNMENT: u64 = 0x40 * LBA_SIZE;

const IDBLOADER_PARTNAME: &'static str = "idbloader";

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Add a partition to the disk
    #[arg(short, long)]
    partition: Vec<String>,

    /// Add empty partition to the disk
    #[arg(short, long)]
    blank_partition: Vec<String>,

    /// Disk or image file to write to
    #[arg(short, long)]
    destination: PathBuf,

    /// Format partition (use in combination with --blank-partition)
    #[arg(short, long)]
    format_partition: Vec<String>,

    /// Image file size (only if destination is not a device)
    #[arg(short, long, default_value="0")]
    size: String,

    /// Path to IDBloader
    #[arg(short, long)]
    idbloader: Option<PathBuf>
}

fn check_args(opt: &Args) -> Result<(), String> {
    match opt.destination.try_exists() {
        Err(err) => Err(format!(
            "Could not access file {}: {}",
            opt.destination.to_str().unwrap_or("<invalid path>"), err
        )),
        _ => Ok(())
    }?;

    if opt.destination.is_dir() {
        return Err(format!(
            "Destination {} is a directory",
            opt.destination.to_str().unwrap_or("<invalid path>")
        ))
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct PartitionDefinition {
    partition_name: String,
    source_file: Option<PathBuf>,
    size: u64,
}

#[derive(Clone, Debug)]
struct FormatPartitionDefinition {
    partition_name: String,
    format_as: String,
}

#[derive(Clone, Debug)]
struct CreatedPartition {
    def: Option<PartitionDefinition>,
    partition: Partition,
}

fn parse_partition(part_arg: &String) -> Result<PartitionDefinition, String> {
    let split = match part_arg.split_once(":") {
        None => Err(format!("Invalid partition argument: {}", part_arg)),
        Some(split) => Ok(split)
    }?;
    let source_filename = split.1;
    let source_file: PathBuf = source_filename.into();
    match source_file.try_exists() {
        Err(err) => Err(
            format!("Source file {} is inaccessible: {}", source_filename, err)
        ),
        Ok(false) => Err(format!("Source file {} does not exist", source_filename)),
        _ => Ok(())
    }?;
    let part_size =
        metadata(source_file.clone())
            .map_err(|err| format!(
                "Failed to get metadata for source file {}: {}",
                source_file.to_str().unwrap(), err
            ))
            .and_then(|source_metadata|
                Ok(align_up(source_metadata.len(), FIRST_PART_ALIGNMENT))
            )?;

    Ok(PartitionDefinition {
        partition_name: split.0.into(),
        source_file: Some(source_file),
        size: part_size,
    })
}

fn parse_empty_partition(part_arg: &String) -> Result<PartitionDefinition, String> {
    let split = match part_arg.split_once(":") {
        None => Err(format!("Invalid empty partition argument: {}", part_arg)),
        Some(split) => Ok(split)
    }?;
    let size_string = split.1;
    let size = parse_size(size_string)
        .map_err(|e| format!("Invalid size for empty partition ({}): {}", size_string, e))?;

    Ok(PartitionDefinition {
        partition_name: split.0.into(),
        source_file: None,
        size,
    })
}

fn parse_format_partition(part_arg: &String) -> Result<FormatPartitionDefinition, String> {
    let split = match part_arg.split_once(":") {
        None => Err(format!("Invalid partition argument (missing fs): {}", part_arg)),
        Some(split) => Ok(split)
    }?;
    let partition_name = split.0.into();
    let format_as = split.1.into();

    Ok(FormatPartitionDefinition { partition_name, format_as })
}

fn parse_partitions(opt: &Args) -> Result<Vec<PartitionDefinition>, String> {
    opt.partition.iter()
        .map(|part_arg| parse_partition(part_arg))
        .chain(
            opt.blank_partition.iter()
                .map(|part_arg| parse_empty_partition(part_arg))
        )
        .collect()
}

fn parse_format_partitions(opt: &Args) -> Result<Vec<FormatPartitionDefinition>, String> {
    opt.format_partition.iter()
        .map(|part_arg| parse_format_partition(part_arg))
        .collect()
}

fn reorder_partitions(partitions: Vec<PartitionDefinition>) -> Vec<PartitionDefinition> {
    let bootloader_partitions = partitions.clone().into_iter()
        .filter(|part|
            partition_name_to_type(
                part.partition_name.clone()
            ) == partition_types::ANDROID_BOOTLOADER
        );

    let all_other_partitions = partitions.into_iter()
        .filter(|part|
            partition_name_to_type(
                part.partition_name.clone()
            ) != partition_types::ANDROID_BOOTLOADER
        );

    bootloader_partitions.chain(all_other_partitions).collect()
}

fn main() -> Result<(), String> {
    let opt = Args::parse();

    let size = parse_size(opt.size.clone())
        .map_err(|e| format!("Invalid size ({}): {}", opt.size, e))?;

    check_args(&opt)?;

    let partitions = parse_partitions(&opt)?;
    let partitions = reorder_partitions(partitions);
    let partitions_to_format = parse_format_partitions(&opt)?;

    flash(opt.destination.clone(), size, partitions, opt.idbloader)?;
    format_partitions(opt.destination.clone(), partitions_to_format)?;

    Ok(())
}

fn flash(
    destination: PathBuf,
    size: u64,
    partitions: Vec<PartitionDefinition>,
    idbloader: Option<PathBuf>,
) -> Result<(), String> {
    if partitions.is_empty() && idbloader.is_none() {
        eprintln!("No partitions specified, nothing to flash, skipping.");
        return Ok(())
    }

    let (size, is_block_device) = match is_block_device(destination.clone()) {
        Ok(true) => match get_device_size(destination.clone()) {
            Ok(size) => Ok((size, true)),
            Err(_) => Err(format!(
                "Failed to determine device size: {}",
                destination.to_str().unwrap_or("<invalid path>")
            ))
        },
        _ => Ok((size, false)),
    }?;

    eprintln!(
        "Destination: {} ({})", destination.to_str().unwrap(),
        BinarySize::from(size).rounded()
    );

    if !is_block_device {
        create_sparse_file(destination.clone(), size)?;
    } else {
        erase_beginning(destination.clone())?;
    }

    let created_partitions =
        create_partition_table(destination.clone(), partitions, idbloader)?;

    write_images(destination, created_partitions)?;

    eprintln!("Flash complete.");

    Ok(())
}

fn open_write_sync(path: PathBuf) -> io::Result<File> {
    OpenOptions::new()
        .read(true).write(true)
        .custom_flags(
            if cfg!(unix) {
                libc::O_SYNC
            } else {
                0
            }
        )
        .open(path)
}

fn create_protective_mbr(path: PathBuf) -> Result<(), String> {
    let mut file = open_write_sync(path.clone())
        .map_err(|err| format!("Could not open file: {}", err))?;

    let device_size = get_device_size(path.clone()).unwrap();

    let mbr = gpt::mbr::ProtectiveMBR::with_lb_size(
        u32::try_from((device_size / LBA_SIZE) - 1).unwrap_or(0xFF_FF_FF_FF));
    mbr.overwrite_lba0(&mut file)
        .map_err(|err| format!("Failed to write MBR to {}: {}", path.to_str().unwrap(), err))?;

    Ok(())
}

fn create_partition_table(
    destination: PathBuf,
    partitions: Vec<PartitionDefinition>,
    idbloader: Option<PathBuf>,
) -> Result<Vec<CreatedPartition>, String> {
    let mut created_partitions = vec![];

    eprintln!("Creating protective MBR…");
    create_protective_mbr(destination.clone())?;

    let cfg = gpt::GptConfig::new()
        .initialized(false)
        .writable(true)
        .logical_block_size(LBA);


    eprintln!("Opening {}…", destination.to_str().unwrap());
    let mut disk = cfg.open(destination.clone())
        .map_err(|err| format!(
            "Failed to open file {} for creating a partition table: {}",
            destination.to_str().unwrap(), err
        ))?;

    // Make sure there are no partitions
    disk.update_partitions(BTreeMap::<u32, Partition>::new())
        .map_err(|err| format!("Failed to clear partition table: {}", err))?;

    if let Some(idbloader) = idbloader {
        let loader_size = metadata(idbloader.clone())
            .map_err(|err| format!(
                "Failed to get metadata for file {}: {}",
                idbloader.to_str().unwrap(), err
            ))
            .and_then(|source_metadata|
                Ok(align_up(source_metadata.len(), IDBLOADER_ALIGNMENT))
            )?;
        eprintln!(
            "Adding partition for pre-bootloader, size {}",
            BinarySize::from(loader_size).rounded()
        );
        let part_id = disk.add_partition(
            IDBLOADER_PARTNAME,
            loader_size,
            partition_types::ANDROID_BOOTLOADER,
            0,
            Some(IDBLOADER_ALIGNMENT_LBA)
        ).map_err(|err| format!(
            "Could not add pre-bootloader partition, size {}: {}",
            BinarySize::from(loader_size).rounded(), err
        ))?;

        let partition = disk.partitions().get(&part_id)
            .ok_or(format!("Can't find created partition with ID {}", part_id))?;

        created_partitions.push(
            CreatedPartition {
                def: Some(PartitionDefinition {
                    partition_name: IDBLOADER_PARTNAME.into(),
                    source_file: Some(idbloader.clone()),
                    size: loader_size,
                }),
                partition: partition.clone(),
            }
        );
    }

    for (index, partition_def) in partitions.iter().enumerate() {
        let part_alignment = if index == 0 { FIRST_PART_ALIGNMENT } else { PART_ALIGNMENT };
        let part_size = partition_def.size;

        eprintln!(
            "Adding partition {}, size {}",
            partition_def.partition_name, BinarySize::from(part_size).rounded()
        );

        let part_id = disk.add_partition(
            partition_def.partition_name.as_str(),
            part_size,
            partition_name_to_type(partition_def.partition_name.clone()),
            partition_name_to_flags(partition_def.partition_name.clone()),
            // Align on 1 MiB boundary
            Some(part_alignment / LBA_SIZE)
        ).map_err(|err| format!(
            "Could not add partition name {}, size {}: {}",
            partition_def.partition_name, BinarySize::from(part_size).rounded(), err
        ))?;

        let partition = disk.partitions().get(&part_id)
            .ok_or(format!("Can't find created partition with ID {}", part_id))?;
        created_partitions.push(
            CreatedPartition {
                def: Some(partition_def.clone()),
                partition: partition.clone(),
            }
        );
    }

    let has_created_userdata = partitions.iter()
        .any(|def|
            partition_name_to_type(def.partition_name.clone()) == partition_types::ANDROID_DATA
        );
    if !has_created_userdata {
        // For the remaining space, we'll create an userdata partition
        if let Some(last_free_sectors) = disk.find_free_sectors().last() {
            let last_free_sectors = last_free_sectors.clone();
            let part_size = last_free_sectors.1 * LBA_SIZE;
            eprintln!(
                "Creating userdata partition, size {}", BinarySize::from(part_size).rounded()
            );
            let part_id = disk.add_partition(
                "userdata",
                part_size,
                partition_types::ANDROID_DATA,
                0,
                Some(PART_ALIGNMENT / LBA_SIZE)
            ).map_err(|err| format!(
                "Could not add userdata partition size {}: {}",
                BinarySize::from(part_size).rounded(), err
            ))?;
            let partition = disk.partitions().get(&part_id)
                .ok_or(format!("Can't find created partition with ID {}", part_id))?;
            created_partitions.push(
                CreatedPartition {
                    def: None,
                    partition: partition.clone(),
                }
            );
        }
    }

    eprintln!("Writing partition table…");
    disk.write().map_err(|err| format!("Failed to write partition table: {}", err))?;

    Ok(created_partitions)
}

fn get_device_size(device_path: impl AsRef<Path>) -> BlockResult<u64> {
    match get_device_info(device_path) {
        Ok(device) => Ok(device.capacity),
        Err(e) => Err(e),
    }
}

fn create_sparse_file(path: impl AsRef<Path>, size: u64) -> Result<(), String> {
    let mut open_options = OpenOptions::new();
    open_options.read(true).write(true).create(true).truncate(true);

    let mut file = open_options.open(path)
        .map_err(|err| format!("Could not create and open file: {}", err))?;

    // Make sure the file is actually 16GB in size
    file.seek(SeekFrom::Start(size - 1))
        .map_err(|err| format!("Could not seek into sparse file: {}", err))?;
    file.write(&[0x00])
        .map_err(|err| format!("Could not finalize sparse file: {}", err))?;

    Ok(())
}

fn erase_beginning(path: PathBuf) -> Result<(), String> {
    let sp = SpinnerBuilder::new("Erasing beginning of disk".into()).start();
    let file = open_write_sync(path)
        .map_err(|err| format!("Could not open file: {}", err))?;


    // First we'll erase the first 8 MiB to make sure there are no leftovers of old loaders
    file.write_at(vec![0_u8; FIRST_PART_ALIGNMENT as usize].as_slice(), 0)
        .map_err(|err| format!("Failed to erase beginning of disk: {}", err))?;

    sp.message("Erased beginning of disk".into());
    sp.close();
    Ok(())
}

fn partition_name_to_type(name: String) -> partition_types::Type {
    match name.as_str() {
        "system" | "vendor" | "super" | "product" | "odm" => partition_types::ANDROID_SYSTEM,
        "cache" => partition_types::ANDROID_CACHE,
        "userdata" => partition_types::ANDROID_DATA,
        "boot" | "vendor_boot" | "system_dlkm" | "vendor_dlkm" |
        "dtb" | "dtbo" | "vbmeta" | "security" => partition_types::ANDROID_BOOT,
        "recovery" => partition_types::ANDROID_RECOVERY,
        "misc" => partition_types::ANDROID_MISC,
        "metadata" => partition_types::ANDROID_META,
        "factory" | "backup" => partition_types::ANDROID_FACTORY,
        "uboot" | "bootloader" | "loader" | "trust" | "idbloader" =>
            partition_types::ANDROID_BOOTLOADER,
        "stage2" | "bootloader2" | "loader2" => partition_types::ANDROID_BOOTLOADER2,
        "fastboot" => partition_types::ANDROID_FASTBOOT,
        "oem" => partition_types::ANDROID_OEM,
        "persist" => partition_types::ANDROID_PERSISTENT,
        _ => partition_types::BASIC
    }
}

fn partition_name_to_flags(name: String) -> u64 {
    match name.as_str() {
        // it looks like we don't need to set any flags, but maybe we should set 0 and 1 accordingly
        _ => 0
    }
}

fn write_images(
    destination: PathBuf,
    partitions: Vec<CreatedPartition>
) -> Result<(), String> {
    eprintln!("Opening {} to write images…", destination.to_str().unwrap());
    let mut file = OpenOptions::new().read(true).write(true)
        .custom_flags(
            if cfg!(unix) {
                libc::O_SYNC
            } else {
                0
            }
        )
        .open(destination.clone())
        .map_err(|err| format!(
            "Could not open destination file {} for writing images: {}",
            destination.to_str().unwrap(), err
        ))?;

    const CLEAR_BYTES: [u8; 1024] = [0; 1024];
    const BIG_CLEAR_BYTES: [u8; 1024*32] = [0; 1024*32];

    for partition in partitions {
        let sp = SpinnerBuilder::new(
            format!("Preparing partition {}", partition.partition.name)
        ).start();
        let partition_start = partition.partition.first_lba * LBA_SIZE;

        // First, clear the first KiB to make sure there is no file system
        file.write_at(&CLEAR_BYTES, partition_start)
            .map_err(|err| format!(
                "Failed to clear filesystem signatures on partition {} at offset {}: {}",
                partition.partition.name, partition_start, err
            ))?;

        // Both def and def.source_file must be Some, otherwise there's no point
        // in writing anything. This if statement matches both at the same time.
        if let Some((def, Some(source_file))) = partition.def.and_then(
            |def| Some((def.clone(), def.source_file))
        ) {
            file.seek(SeekFrom::Start(partition_start))
                .map_err(|err| format!(
                    "Could not seek to start of partition {}: {}",
                    partition.partition.name, err
                ))?;

            sp.update(format!(
                "Writing partition {} ({})",
                partition.partition.name, BinarySize::from(def.size).rounded()
            ));

            let mut input_file = OpenOptions::new().read(true).open(source_file.clone())
                .map_err(|err| format!(
                    "Could not open source file {} to write to {}: {}",
                    source_file.to_str().unwrap(), partition.partition.name, err
                ))?;

            let bytes_copied = copy(&mut input_file, &mut file)
                .map_err(|err| format!(
                    "Failed to write image {} to {} on {}: {}",
                    source_file.to_str().unwrap(), partition.partition.name,
                    destination.to_str().unwrap(), err
                ))?;

            let remaining_bytes = partition.partition.bytes_len(LBA)
                .map_err(|err| format!(
                    "Unable to calculate remaining bytes for {}: {}",
                    partition.partition.name, err
                ))? - bytes_copied;

            if remaining_bytes > 0 {
                sp.update(format!(
                    "Clearing rest of partition {} ({})…",
                    partition.partition.name, BinarySize::from(remaining_bytes).rounded()
                ));

                let clear_bytes_size = BIG_CLEAR_BYTES.len();
                let mut clear_bytes: Vec<u8> = BIG_CLEAR_BYTES.into();
                for offset in (0..remaining_bytes).step_by(clear_bytes_size) {
                    // This will only actually truncate when the last step is reached
                    clear_bytes.truncate((remaining_bytes - offset) as usize);
                    file.write(clear_bytes.as_slice()).map_err(|err| format!(
                        "Failed to write clear bytes to {} on {}: {}",
                        partition.partition.name,
                        destination.to_str().unwrap(), err
                    ))?;
                }
            }

            sp.message(format!(
                "Successfully wrote {} ({} at {:#x})",
                partition.partition.name, BinarySize::from(def.size).rounded(),
                partition_start,
            ));
        } else {
            sp.message(format!("Cleared {}, nothing else to do.", partition.partition.name));
        }
        sp.close();
    }

    eprintln!("Finished writing all partitions");

    Ok(())
}

fn format_partitions(
    destination: PathBuf,
    partitions_to_format: Vec<FormatPartitionDefinition>
) -> Result<(), String>  {
    if partitions_to_format.is_empty() {
        return Ok(())
    }
    if !cfg!(target_os = "linux") {
        return Err(format!("Creating filesystems is unsupported on {}", cfg!(target_os)));
    }

    eprintln!("Probing partitions");
    let output = Command::new("partprobe")
        .output()
        .or_else(|e| {
            eprintln!("Failed to run partprobe: {}", e);
            Err(e)
        })
        .ok();
    if let Some(output) = output {
        if !output.status.success() {
            eprintln!(
                "WARNING: partprobe failed:\n{}\n{}",
                String::from_utf8_lossy(output.stdout.as_slice()),
                String::from_utf8_lossy(output.stderr.as_slice())
            )
        }
    }

    eprintln!("Starting format, partition count: {}", partitions_to_format.len());

    let cfg = gpt::GptConfig::new()
        .initialized(true)
        .writable(false)
        .logical_block_size(LBA);

    eprintln!("Opening {}…", destination.to_str().unwrap());
    let disk = cfg.open(destination.clone())
        .map_err(|err| format!(
            "Failed to open file {} for reading partition table: {}",
            destination.to_str().unwrap(), err
        ))?;

    for partition_to_format in partitions_to_format {
        let (_, gpt_part) = disk.partitions().iter().find(
            |(_, part)| part.name == partition_to_format.partition_name
        ).ok_or_else(|| format!(
            "Could not find partition {} to format as {}",
            partition_to_format.partition_name, partition_to_format.format_as
        ))?;
        let part_uuid = gpt_part.part_guid;
        eprintln!(
            "Formatting {} as {} (PARTUUID={})",
            gpt_part.name,
            partition_to_format.format_as,
            part_uuid
        );
        let device = format!("/dev/disk/by-partuuid/{}", part_uuid.to_string());
        wait_for_device(
            PathBuf::from(device.clone()),
            20, Duration::from_millis(250)
        )?;
        let output = Command::new(format!("mkfs.{}", partition_to_format.format_as))
            .arg(device)
            .output()
            .map_err(|e| format!(
                "Failed to run mkfs.{} on partition {} (PARTUUID={}): {}",
                partition_to_format.format_as,
                gpt_part.name,
                part_uuid.to_string(),
                e
            ))?;
        if !output.status.success() {
            eprintln!(
                "mkfs.{} exited with status code {}. Output:",
                partition_to_format.format_as,
                output.status.code().unwrap_or(-1)
            );
            eprintln!("{}", String::from_utf8_lossy(output.stdout.as_slice()));
            eprintln!("{}", String::from_utf8_lossy(output.stderr.as_slice()));
            return Err(format!(
                "Failed to format partition {} (PARTUUID={}) using mkfs.{}:\n{}\n{}",
                gpt_part.name,
                part_uuid.to_string(),
                partition_to_format.format_as,
                String::from_utf8_lossy(output.stdout.as_slice()),
                String::from_utf8_lossy(output.stderr.as_slice()),
            ))
        }
    }

    Ok(())
}

fn wait_for_device(device: PathBuf, retries: u32, retry_interval: Duration) -> Result<(), String> {
    let mut tried = 0;
    while !(device.exists() && (device.is_file() || device.is_symlink())) {
        if retries == tried {
            return Err(format!(
                "Timed out waiting for device {}, retries: {}",
                device.to_string_lossy(),
                tried
            ))
        }
        if tried == 0 {
            eprintln!("Waiting for device {}…", device.to_string_lossy())
        }
        tried += 1;
        sleep(retry_interval)
    }
    Ok(())
}

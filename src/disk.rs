use std::fs::{remove_file, File, OpenOptions};
use std::io;
use std::io::{copy, Read, Seek, SeekFrom};
use std::ops::Range;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;
use gpt::GptDisk;
use gpt::header::Header;
use gpt::partition::Partition;
use sizes::BinarySize;
use uuid::Uuid;
use crate::destination::Destination;
use crate::layout::{Layout, PartitionDefinition, Placement, FIRST_PART_ALIGNMENT, LBA, LBA_SIZE};
use crate::values::ByteSize;

const CLEAR_BYTES: [u8; 1024] = [0; 1024];

#[derive(Clone, Debug)]
struct CreatedPartition {
    def: PartitionDefinition,
    partition: Partition,
}

pub fn flash(destination: &Destination, layout: &Layout) -> Result<(), String> {
    if layout.is_empty() {
        eprintln!("No partitions specified, nothing to flash, skipping.");
        return Ok(())
    }

    // Mounting a partition of a block device fails while the device is open exclusively.
    let mut exclusive_device = None;
    let size = match destination {
        Destination::BlockDevice(device) => exclusive_device.insert(open_exclusive(device)?)
            .seek(SeekFrom::End(0))
            .map_err(|err| format!("Could not read the size of device {}: {}.", device.display(), err)),
        Destination::ImageFile { path, size } => match size.map(ByteSize::bytes) {
            Some(size) => create_sparse_file(path, size).map(|()| size),
            None if layout.fill_userdata => Err(format!(
                "Image file {} needs a size that leaves space for the userdata partition. \
                Pass --size, or pass --no-fill to leave the userdata partition out.",
                path.display()
            )),
            None => create_fitted_image(path, layout),
        },
    }?;

    eprintln!(
        "Destination: {} ({})", destination.path().display(),
        BinarySize::from(size).rounded()
    );

    let created_partitions = create_partition_table(destination, size, layout)?;

    write_images(destination, created_partitions)?;

    eprintln!("Flash complete.");

    Ok(())
}

fn open_read_write(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

fn create_protective_mbr(path: &Path, device_size: u64) -> Result<(), String> {
    let mut file = open_read_write(path)
        .map_err(|err| format!("Could not open file: {err}"))?;

    let mbr = gpt::mbr::ProtectiveMBR::with_lb_size(
        u32::try_from((device_size / LBA_SIZE) - 1).unwrap_or(0xFF_FF_FF_FF));
    mbr.overwrite_lba0(&mut file)
        .map_err(|err| format!("Failed to write MBR to {}: {}", path.display(), err))?;

    Ok(())
}

fn create_partition_table(
    destination: &Destination,
    size: u64,
    layout: &Layout,
) -> Result<Vec<CreatedPartition>, String> {
    let path = destination.path();
    eprintln!("Opening {}…", path.display());
    let (disk, created_partitions) = lay_out_partitions(path, layout)?;

    for created in &created_partitions {
        eprintln!(
            "Partition {} at sector {}, size {}",
            created.def.partition_name, created.partition.first_lba,
            BinarySize::from(created.def.size).rounded()
        );
    }

    if let Destination::BlockDevice(device) = destination {
        erase_beginning(device)?;
    }

    eprintln!("Creating protective MBR…");
    create_protective_mbr(path, size)?;

    eprintln!("Writing partition table…");
    disk.write().map_err(|err| format!("Failed to write partition table: {err}"))?;

    Ok(created_partitions)
}

fn lay_out_partitions(
    destination: &Path,
    layout: &Layout,
) -> Result<(GptDisk<File>, Vec<CreatedPartition>), String> {
    let mut disk = new_partition_table(destination, layout.disk_guid)?;
    let created_partitions = layout.place(usable_bytes(disk.header()))?.into_iter()
        .map(|placement| add_partition(&mut disk, placement))
        .collect::<Result<Vec<_>, String>>()?;

    Ok((disk, created_partitions))
}

fn new_partition_table(destination: &Path, disk_guid: Option<Uuid>) -> Result<GptDisk<File>, String> {
    let cfg = gpt::GptConfig::new()
        .writable(true)
        .logical_block_size(LBA);

    open_read_write(destination)
        .map_err(gpt::GptError::from)
        .and_then(|file| cfg.create_from_device(file, disk_guid))
        .map_err(|err| format!(
            "Failed to open file {} for creating a partition table: {}",
            destination.display(), err
        ))
}

fn usable_bytes(header: &Header) -> Range<u64> {
    header.first_usable * LBA_SIZE..(header.last_usable + 1) * LBA_SIZE
}

fn add_partition(disk: &mut GptDisk<File>, placement: Placement) -> Result<CreatedPartition, String> {
    let sectors = placement.sectors();
    let Placement { def, .. } = placement;
    let part_id = disk.find_next_partition_id()
        .ok_or(format!(
            "The partition table has no free entry for partition {}. Use fewer partitions.",
            def.partition_name
        ))?;
    disk.add_partition_at(
        def.partition_name.as_str(),
        part_id,
        sectors.start,
        sectors.end - sectors.start,
        def.part_type.clone(),
        0,
    ).map_err(|err| format!(
        "Could not add partition {}, size {}: {}",
        def.partition_name, BinarySize::from(def.size).rounded(), err
    ))?;

    if let Some(part_guid) = def.part_guid {
        let mut table = disk.take_partitions();
        table.get_mut(&part_id)
            .ok_or(format!("Can't find created partition with ID {part_id}"))?
            .part_guid = part_guid;
        disk.update_partitions(table)
            .map_err(|err| format!("Could not set UUID of partition {}: {}", def.partition_name, err))?;
    }

    let partition = disk.partitions().get(&part_id).cloned()
        .ok_or(format!("Can't find created partition with ID {part_id}"))?;
    Ok(CreatedPartition { def, partition })
}

fn create_fitted_image(path: &Path, layout: &Layout) -> Result<u64, String> {
    let upper_bound = layout.definitions()
        .map(|def| def.size + FIRST_PART_ALIGNMENT)
        .sum::<u64>() + FIRST_PART_ALIGNMENT;
    create_sparse_file(path, upper_bound)
        .and_then(|()| fitted_size(path, layout))
        .and_then(|size| create_sparse_file(path, size).map(|()| size))
        .map_err(|err| match remove_file(path) {
            Ok(()) => err,
            Err(remove_err) if remove_err.kind() == io::ErrorKind::NotFound => err,
            Err(remove_err) => format!("{err} Could not remove image file {}: {remove_err}", path.display()),
        })
}

fn fitted_size(path: &Path, layout: &Layout) -> Result<u64, String> {
    let disk = new_partition_table(path, layout.disk_guid)?;
    let header = disk.header();
    let usable = usable_bytes(header);
    let partitions_end = layout.place(usable.clone())?.iter()
        .map(Placement::end)
        .max()
        .unwrap_or(usable.start);
    let backup_table = (header.backup_lba - header.last_usable) * LBA_SIZE;
    Ok(partitions_end + backup_table)
}

fn open_exclusive(device: &Path) -> Result<File, String> {
    OpenOptions::new().read(true).custom_flags(libc::O_EXCL).open(device)
        .map_err(|err| match err.kind() {
            io::ErrorKind::ResourceBusy => format!(
                "Device {} is in use, for example by a mounted partition or by the running system. \
                Unmount every partition on it, or give another device.",
                device.display()
            ),
            _ => format!(
                "Could not open device {}: {}. \
                Check that the device is connected and that you can open it, for example as root.",
                device.display(), err
            ),
        })
}

fn create_sparse_file(path: impl AsRef<Path>, size: u64) -> Result<(), String> {
    let mut open_options = OpenOptions::new();
    open_options.read(true).write(true).create(true).truncate(true);

    open_options.open(path)
        .and_then(|file| file.set_len(size))
        .map_err(|err| format!("Could not create a sparse file of size {size}: {err}"))
}

fn erase_beginning(path: &Path) -> Result<(), String> {
    eprintln!("Erasing beginning of disk…");
    let mut file = open_read_write(path)
        .map_err(|err| format!("Could not open file: {err}"))?;

    // First we'll erase the first 8 MiB to make sure there are no leftovers of old loaders
    copy(&mut io::repeat(0).take(FIRST_PART_ALIGNMENT), &mut file)
        .map_err(|err| format!("Failed to erase beginning of disk: {err}"))?;

    Ok(())
}

fn write_images(
    destination: &Destination,
    partitions: Vec<CreatedPartition>
) -> Result<(), String> {
    let path = destination.path();
    eprintln!("Opening {} to write images…", path.display());
    let mut file = open_read_write(path)
        .map_err(|err| format!(
            "Could not open destination file {} for writing images: {}",
            path.display(), err
        ))?;

    for partition in partitions {
        eprintln!("Preparing partition {}…", partition.partition.name);
        let partition_start = partition.partition.first_lba * LBA_SIZE;

        // First, clear the first KiB to make sure there is no file system
        file.write_at(&CLEAR_BYTES, partition_start)
            .map_err(|err| format!(
                "Failed to clear filesystem signatures on partition {} at offset {}: {}",
                partition.partition.name, partition_start, err
            ))?;

        if let Some(source) = &partition.def.source {
            file.seek(SeekFrom::Start(partition_start))
                .map_err(|err| format!(
                    "Could not seek to start of partition {}: {}",
                    partition.partition.name, err
                ))?;

            eprintln!(
                "Writing partition {} ({})…",
                partition.partition.name, BinarySize::from(partition.def.size).rounded()
            );

            let input_file = OpenOptions::new().read(true).open(&source.path)
                .map_err(|err| format!(
                    "Could not open source file {} to write to {}: {}",
                    source.path.display(), partition.partition.name, err
                ))?;

            let bytes_copied = copy(&mut input_file.take(source.len), &mut file)
                .map_err(|err| format!(
                    "Failed to write image {} to {} on {}: {}",
                    source.path.display(), partition.partition.name,
                    path.display(), err
                ))?;
            if bytes_copied != source.len {
                return Err(format!(
                    "Source file {} for partition {} ended after {} of its {} bytes while it was written. \
                    Leave the source files unchanged while rockflasher runs.",
                    source.path.display(), partition.partition.name, bytes_copied, source.len
                ))
            }

            let remaining_bytes = partition.partition.bytes_len(LBA)
                .map_err(|err| format!(
                    "Unable to calculate remaining bytes for {}: {}",
                    partition.partition.name, err
                ))? - bytes_copied;

            // An image file is created empty, so the rest of its partitions is zero already.
            if remaining_bytes > 0 && matches!(destination, Destination::BlockDevice(_)) {
                eprintln!(
                    "Clearing rest of partition {} ({})…",
                    partition.partition.name, BinarySize::from(remaining_bytes).rounded()
                );

                copy(&mut io::repeat(0).take(remaining_bytes), &mut file)
                    .map_err(|err| format!(
                        "Failed to write clear bytes to {} on {}: {}",
                        partition.partition.name,
                        path.display(), err
                    ))?;
            }

            eprintln!(
                "Successfully wrote {} ({} at {:#x})",
                partition.partition.name, BinarySize::from(partition.def.size).rounded(),
                partition_start,
            );
        } else {
            eprintln!("Cleared {}, nothing else to do.", partition.partition.name);
        }
    }

    eprintln!("Finished writing all partitions, syncing {}…", path.display());
    file.sync_all().map_err(|err| format!(
        "Failed to sync {}: {}", path.display(), err
    ))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{layout, ScratchDir};

    fn flash_fitted_image(scratch: &ScratchDir, options: &[&str]) -> GptDisk<File> {
        let image = scratch.path().join("disk.img");
        let layout = layout(options).unwrap();
        flash(&Destination::ImageFile { path: image.clone(), size: None }, &layout).unwrap();
        gpt::GptConfig::new().logical_block_size(LBA).open(&image).unwrap()
    }

    fn assert_fitted(disk: &GptDisk<File>) {
        let header = disk.header();
        let last_lba = disk.partitions().values().map(|partition| partition.last_lba).max();
        assert_eq!(last_lba, Some(header.last_usable));
        assert!(disk.backup_header().is_ok());
        let image_size = disk.device_ref().metadata().unwrap().len();
        assert_eq!(image_size, (header.backup_lba + 1) * LBA_SIZE);
    }

    #[test]
    fn image_file_ends_with_the_backup_table_after_the_last_partition() {
        let scratch = ScratchDir::new("disk-fitted");
        let disk = flash_fitted_image(&scratch, &[
            "--blank-partition", "a:1000",
            "--blank-partition", "b:3MiB",
            "--no-fill",
        ]);
        assert_fitted(&disk);
    }

    #[test]
    fn image_file_with_idbloader_keeps_the_loader_offsets() {
        let scratch = ScratchDir::new("disk-fitted-idbloader");
        let idbloader = scratch.file("idbloader.img", 200 * 1024);
        let uboot = scratch.file("u-boot.itb", 1024 * 1024);
        let disk = flash_fitted_image(&scratch, &[
            "--idbloader", &idbloader.display().to_string(),
            "--blank-partition", "main:5MiB",
            "--partition", &format!("uboot:{}", uboot.display()),
            "--no-fill",
        ]);
        assert_fitted(&disk);
        let start_of = |name: &str| disk.partitions().values()
            .find(|partition| partition.name == name)
            .map(|partition| partition.first_lba * LBA_SIZE);
        assert_eq!(start_of("idbloader"), Some(32 * 1024));
        assert_eq!(start_of("uboot"), Some(8 * 1024 * 1024));
        assert_eq!(start_of("main"), Some(16 * 1024 * 1024));
    }

    #[test]
    fn image_file_takes_the_given_disk_guid() {
        let scratch = ScratchDir::new("disk-guid");
        let guid = "5452574f-2211-4433-5566-778899aabb00";
        let disk = flash_fitted_image(&scratch, &["--blank-partition", "a:1MiB", "--disk-uuid", guid, "--no-fill"]);
        assert_eq!(*disk.guid(), guid.parse::<Uuid>().unwrap());
        assert_eq!(disk.backup_header().unwrap().disk_guid, *disk.guid());
    }

    #[test]
    fn image_file_that_does_not_fit_is_removed() {
        let scratch = ScratchDir::new("disk-fitted-removed");
        let image = scratch.path().join("disk.img");
        let mut layout = layout(&["--blank-partition", "a:1MiB", "--no-fill"]).unwrap();
        layout.partitions[0].size = u64::try_from(i64::MAX).unwrap();
        assert!(flash(&Destination::ImageFile { path: image.clone(), size: None }, &layout).is_err());
        assert!(!image.exists());
    }

    #[test]
    fn flash_writes_each_source_at_its_partition_start_and_zeros_after_it() {
        let scratch = ScratchDir::new("disk-contents");
        let idbloader = scratch.patterned_file("idbloader.img", 200 * 1024, 1);
        let uboot = scratch.patterned_file("u-boot.itb", 1000 * 1024, 2);
        let esp = scratch.patterned_file("disk.img.esp", 3 * 1024 * 1024 + 512, 3);
        let repart = scratch.path().join("repart.json");
        let partitions = serde_json::json!([
            { "type": "esp", "label": "esp", "uuid": "8f3c2a6e-4b1d-4e7a-9c05-d2b7e1f04a93",
              "raw_size": 8 * 1024 * 1024, "split_path": "disk.img.esp" },
            { "type": "linux-generic", "label": "var", "uuid": "0b6e3c52-1d8f-4a27-b9e4-6f1c2d3a4b5c",
              "raw_size": 2 * 1024 * 1024, "split_path": "-" },
        ]);
        std::fs::write(&repart, partitions.to_string()).unwrap();
        let image = scratch.path().join("disk.img");
        let layout = layout(&[
            "--idbloader", &idbloader.display().to_string(),
            "--partition", &format!("uboot:{}", uboot.display()),
            "--repart", &repart.display().to_string(),
            "--no-fill",
        ]).unwrap();

        flash(&Destination::ImageFile { path: image.clone(), size: None }, &layout).unwrap();

        let disk = gpt::GptConfig::new().logical_block_size(LBA).open(&image).unwrap();
        let contents = std::fs::read(&image).unwrap();
        let sources = [
            ("idbloader", Some(idbloader)),
            ("uboot", Some(uboot)),
            ("esp", Some(esp)),
            ("var", None),
        ];
        for (name, source) in sources {
            let partition = disk.partitions().values().find(|partition| partition.name == name).unwrap();
            let start = usize::try_from(partition.first_lba * LBA_SIZE).unwrap();
            let end = usize::try_from((partition.last_lba + 1) * LBA_SIZE).unwrap();
            let source = source.map(|source| std::fs::read(source).unwrap()).unwrap_or_default();
            let (written, rest) = contents[start..end].split_at(source.len());
            assert!(written == source, "contents of {name}");
            assert!(rest.iter().all(|&byte| byte == 0), "zeros after the contents of {name}");
        }
    }

    #[test]
    fn flash_writes_a_source_up_to_its_length_when_the_layout_was_built() {
        let scratch = ScratchDir::new("disk-source-length");
        let source = scratch.patterned_file("a.img", 4096, 1);
        let image = scratch.path().join("disk.img");
        let mut layout = layout(&["--partition", &format!("a:{}", source.display()), "--no-fill"]).unwrap();
        layout.partitions[0].source.as_mut().unwrap().len = 1024;

        flash(&Destination::ImageFile { path: image.clone(), size: None }, &layout).unwrap();

        let disk = gpt::GptConfig::new().logical_block_size(LBA).open(&image).unwrap();
        let start = usize::try_from(disk.partitions().values().next().unwrap().first_lba * LBA_SIZE).unwrap();
        let contents = std::fs::read(&image).unwrap();
        assert_eq!(contents[start..start + 1024], std::fs::read(&source).unwrap()[..1024]);
        assert!(contents[start + 1024..start + 4096].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn source_that_ends_before_its_length_is_refused() {
        let scratch = ScratchDir::new("disk-source-short");
        let source = scratch.file("a.img", 4096);
        let image = scratch.path().join("disk.img");
        let mut layout = layout(&["--partition", &format!("a:{}", source.display()), "--no-fill"]).unwrap();
        layout.partitions[0].source.as_mut().unwrap().len = 8192;
        let err = flash(&Destination::ImageFile { path: image, size: None }, &layout).err().unwrap();
        assert!(err.contains("ended after 4096 of its 8192 bytes"), "{err}");
    }

    #[test]
    fn image_file_without_size_needs_no_fill() {
        let scratch = ScratchDir::new("disk-fill");
        let image = scratch.path().join("disk.img");
        let layout = layout(&["--blank-partition", "a:1MiB"]).unwrap();
        assert!(flash(&Destination::ImageFile { path: image, size: None }, &layout).is_err());
    }
}

use std::io;
use std::path::Path;
use std::process::{Command, Output};
use std::thread::sleep;
use std::time::Duration;
use crate::layout::LBA;
use crate::values::{Filesystem, PartitionSetting};

pub fn format_partitions(
    destination: &Path,
    partitions_to_format: &[PartitionSetting<Filesystem>]
) -> Result<(), String>  {
    if partitions_to_format.is_empty() {
        return Ok(())
    }
    if !cfg!(target_os = "linux") {
        return Err(format!("Creating filesystems is unsupported on {}", std::env::consts::OS));
    }

    eprintln!("Probing partitions");
    let output = Command::new("partprobe")
        .output()
        .map_err(|e| {
            eprintln!("Failed to run partprobe: {e}");
            e
        })
        .ok();
    if let Some(output) = output {
        if !output.status.success() {
            eprintln!(
                "WARNING: partprobe failed:\n{}\n{}",
                String::from_utf8_lossy(output.stdout.as_slice()),
                String::from_utf8_lossy(output.stderr.as_slice())
            );
        }
    }
    sleep(Duration::from_millis(500));

    eprintln!("Starting format, partition count: {}", partitions_to_format.len());

    let cfg = gpt::GptConfig::new()
        .writable(false)
        .logical_block_size(LBA);

    eprintln!("Opening {}…", destination.display());
    let disk = cfg.open(destination)
        .map_err(|err| format!(
            "Failed to open file {} for reading partition table: {}",
            destination.display(), err
        ))?;

    for partition_to_format in partitions_to_format {
        let (_, gpt_part) = disk.partitions().iter().find(
            |(_, part)| part.name == partition_to_format.partition_name
        ).ok_or_else(|| format!(
            "Could not find partition {} to format as {}",
            partition_to_format.partition_name, partition_to_format.value
        ))?;
        let part_uuid = gpt_part.part_guid;
        eprintln!(
            "Formatting {} as {} (PARTUUID={})",
            gpt_part.name,
            partition_to_format.value,
            part_uuid
        );
        let device = format!("/dev/disk/by-partuuid/{part_uuid}");
        wait_for_device(Path::new(&device), 20, Duration::from_millis(250))?;
        let output = run_mkfs(&device, &partition_to_format.value)
            .map_err(|e| format!(
                "Failed to run {} on partition {} (PARTUUID={}): {}",
                partition_to_format.value.mkfs_program(),
                gpt_part.name,
                part_uuid,
                e
            ))?;
        if !output.status.success() {
            eprintln!(
                "{} exited with status code {}. Output:",
                partition_to_format.value.mkfs_program(),
                output.status.code().unwrap_or(-1)
            );
            eprintln!("{}", String::from_utf8_lossy(output.stdout.as_slice()));
            eprintln!("{}", String::from_utf8_lossy(output.stderr.as_slice()));
            return Err(format!(
                "Failed to format partition {} (PARTUUID={}) using {}:\n{}\n{}",
                gpt_part.name,
                part_uuid,
                partition_to_format.value.mkfs_program(),
                String::from_utf8_lossy(output.stdout.as_slice()),
                String::from_utf8_lossy(output.stderr.as_slice()),
            ))
        }
    }

    Ok(())
}

fn wait_for_device(device: &Path, retries: u32, retry_interval: Duration) -> Result<(), String> {
    let mut tried = 0;
    while !(device.exists() &&
        (device.is_file() || device.is_symlink()) && device.read_link().is_ok()) {
        if retries == tried {
            return Err(format!(
                "Timed out waiting for device {}, retries: {}",
                device.to_string_lossy(),
                tried
            ))
        }
        if tried == 0 {
            eprintln!("Waiting for device {}…", device.to_string_lossy());
        }
        tried += 1;
        sleep(retry_interval);
    }
    Ok(())
}

fn run_mkfs(device: &str, filesystem: &Filesystem) -> io::Result<Output> {
    Command::new(filesystem.mkfs_program())
        .arg(device)
        .output()
}

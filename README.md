# rockflasher
A tool for flashing emmc/microSD cards for use with Rockchip boards.
It writes AOSP builds, U-Boot on its own or Linux systems, to a card or to an image file.

## Usage

```
cargo build --profile release
target/release/rockflasher --help
```

### Examples

#### Install AOSP

This will flash the specified images to the card `/dev/sdd`.
In addition, the tool will create two cleared blank partitions as well
as an userdata partition that fills the rest of the available space.

```
sudo target/release/rockflasher \
    --idbloader idbloader.img \
    --partition boot:boot.img \
    --partition dtbo:dtbo.img \
    --partition misc:misc.img \
    --partition recovery:recovery.img \
    --partition super:super.img \
    --partition uboot:u-boot.itb \
    --partition vbmeta:vbmeta.img \
    --blank-partition cache:384MiB \
    --blank-partition metadata:16MiB \
    --format-partition cache:ext4 \
    --format-partition userdata:ext4 \
    --format-partition metadata:ext4 \
    --destination /dev/sdX
```

#### Install U-Boot

```
sudo target/release/rockflasher --idbloader idbloader.img --partition uboot:u-boot.itb --destination /dev/sdX
```

#### Install a Linux system

```
sudo target/release/rockflasher \
    --idbloader idbloader.img \
    --partition uboot:u-boot.itb \
    --partition esp:esp.raw \
    --partition-type esp:esp \
    --partition-uuid esp:ea41e62a-2a38-46c9-bb9d-3e0f807c338f \
    --partition main:main.raw \
    --partition-type main:linux \
    --no-fill \
    --destination /dev/sdX
```

The idbloader starts at sector 64 (32 KiB).
The first partition starts at 8 MiB, and loader partitions (`uboot`, `bootloader`, `loader`, `trust`) come first, so `--partition uboot:u-boot.itb` places U-Boot at 8 MiB.
Every other partition starts at the end of the partition before it, rounded up to 1 MiB.

The partition name is the GPT partition name.
`--partition-type name:type` sets the GPT type: `linux`, a systemd-repart type name (`esp`, `xbootldr`, `linux-generic`, `swap`, `home`, `srv`, `root-x86`, `root-x86-64`, `root-arm`, `root-arm64`), a type name of the gpt crate (such as `linux_fs`) or a type GUID.
`--partition-uuid name:uuid` sets the partition UUID.

`--no-fill` leaves the space after the last partition free. Without it, a userdata partition fills that space.
With `--no-fill`, an image file as `--destination` needs no `--size`: it gets the size of its partitions.

Writing to a block device fails before anything is written while a partition on it is mounted or otherwise in use.

## License

This project is licensed under the MIT License. See `LICENSE` for details.

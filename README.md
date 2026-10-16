# rockflasher
A tool for flashing emmc/microSD cards for use with Rockchip boards, and microSD cards for MediaTek MT7988 boards such as the Banana Pi BPI-R4.
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

#### Install a Linux system from systemd-repart output

Run systemd-repart with `--split=yes` and `--json=short`, save its output, and pass that file with `--repart`:

```
sudo target/release/rockflasher \
    --idbloader idbloader.img \
    --partition uboot:u-boot.itb \
    --repart repart.json \
    --no-fill \
    --destination /dev/sdX
```

Each partition in the file becomes a partition named by its label, with its type, its UUID and its size.
Its split file is written into it. A partition without a split file is created without contents.
A relative split file path is read from the directory of the JSON file.
Its type is read like a `--partition-type` value.
The partitions of `--repart` come after those of `--partition` and `--blank-partition`, and loader partitions still come first.

#### Install a Linux system on a MediaTek MT7988 board

`--bl2` takes the BL2 image with the SD card header, as written by mkimage with `-T mtk_image` and `media=sdmmc`. `--fip` takes the FIP image:

```
sudo target/release/rockflasher \
    --bl2 bl2.img \
    --fip fip.bin \
    --repart repart.json \
    --no-fill \
    --destination /dev/sdX
```

### Layout

A card is written for one board, so `--idbloader` and `--bl2` exclude each other.
On a Rockchip board, the idbloader is written to a partition named `idbloader` at sector 64 (32 KiB).
On a MediaTek MT7988 board, BL2 is written to a partition named `bl2` from sector 34 (17 KiB) to 4 MiB.
That partition has the GPT attributes required and legacy BIOS bootable, and it is the active partition in the MBR.
The FIP is written to a partition named `fip` at 4 MiB, where BL2 finds it by its name.

The first partition from `--partition`, `--blank-partition` or `--repart` starts at the first 8 MiB boundary after the board partitions, and loader partitions (`uboot`, `bootloader`, `loader`, `trust`) come first, so `--partition uboot:u-boot.itb` places U-Boot at 8 MiB.
Every other partition, and the userdata partition that fills the disk, starts at the end of the partition before it, rounded up to 1 MiB.

The partition name is the GPT partition name.
`--partition-type name:type` sets the GPT type: `linux`, a systemd-repart type name (`esp`, `xbootldr`, `linux-generic`, `swap`, `home`, `srv`, `root-x86`, `root-x86-64`, `root-arm`, `root-arm64`), a type name of the gpt crate (such as `linux_fs`) or a type GUID.
`--partition-uuid name:uuid` sets the partition UUID.
`--disk-uuid uuid` sets the disk GUID. With it, `--no-fill` and a `--partition-uuid` for every partition, the same inputs give the same image file.

`--no-fill` leaves the space after the last partition free. Without it, a userdata partition fills that space.
With `--no-fill`, an image file as `--destination` needs no `--size`: it gets the size of its partitions.

Writing to a block device fails before anything is written while a partition on it is mounted or otherwise in use.

## License

This project is licensed under the MIT License. See `LICENSE` for details.

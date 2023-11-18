# rockflasher
A tool for flashing emmc/microSD cards for use with Rockchip boards.
This is meant to be used after building AOSP, but can also be used to install u-boot without anything else.

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

#### Install some Linux OS

Note that this tool is currently not meant to be used for anything other than installing AOSP or U-Boot so the usefulness will be limited.

```
sudo target/release/rockflasher \
    --idbloader idbloader.img \
    --partition uboot:u-boot.itb \
    --partition boot:boot.img \
    --partition super:rootfs.img \
    --blank-partition swap:2GiB \
    --destination /dev/sdX
```

## License

This project is licensed under the MIT License. See `LICENSE` for details.

# rockflash
A tool for flashing emmc/microSD cards for use with Rockchip boards.
This is meant to be used after building AOSP.

## Usage

```
cargo build --profile release
target/release/rockflasher --help
```

### Example

This will flash the specified images to the card `/dev/sdd`.
In addition, the tool will create two cleared blank partitions as well
as an userdata partition that fills the rest of the available space.

```
sudo target/release/rockflasher \
    --partition boot:boot.img \
    --partition dtbo:dtbo.img \
    --partition misc:misc.img \
    --partition recovery:recovery.img \
    --partition super:super.img \
    --partition uboot:uboot.img \
    --partition vbmeta:vbmeta.img \
    --blank-partition cache:384MiB \
    --blank-partition metadata:16MiB \
    --destination /dev/sdd
```

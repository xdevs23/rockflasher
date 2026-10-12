use std::fs::metadata;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use crate::values::ByteSize;

pub enum Destination {
    BlockDevice(PathBuf),
    ImageFile { path: PathBuf, size: Option<ByteSize> },
}

impl Destination {
    pub fn resolve(path: &Path, image_size: Option<ByteSize>) -> Result<Self, String> {
        let image_file = || Destination::ImageFile { path: path.into(), size: image_size };
        match metadata(path) {
            Ok(meta) if meta.is_dir() => Err(format!(
                "Destination {} is a directory. Give a block device or the path of an image file.",
                path.display()
            )),
            Ok(meta) if meta.file_type().is_block_device() => match image_size {
                None => Ok(Destination::BlockDevice(path.into())),
                Some(_) => Err(format!(
                    "Destination {} is a block device, which has a size of its own. \
                    Leave out the image size, or give the path of an image file.",
                    path.display()
                )),
            },
            Ok(_) => Ok(image_file()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(image_file()),
            Err(err) => Err(format!("Could not access destination {}: {}", path.display(), err)),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Destination::BlockDevice(path) | Destination::ImageFile { path, .. } => path,
        }
    }
}


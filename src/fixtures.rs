use std::path::{Path, PathBuf};
use std::{env, fs, process};
use clap::Parser;
use crate::cli::Args;
use crate::layout::Layout;

pub fn layout(options: &[&str]) -> Result<Layout, String> {
    let args = Args::try_parse_from(["rockflasher", "--destination", "disk.img"].iter().chain(options)).unwrap();
    Layout::new(args.layout_request()?)
}

pub struct ScratchDir(PathBuf);

impl ScratchDir {
    pub fn new(name: &str) -> Self {
        let path = env::temp_dir().join(format!("rockflasher-{}-{name}", process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn file(&self, name: &str, size: u64) -> PathBuf {
        let path = self.0.join(name);
        fs::File::create(&path).and_then(|file| file.set_len(size)).unwrap();
        path
    }

    pub fn patterned_file(&self, name: &str, size: u64, seed: u64) -> PathBuf {
        let path = self.0.join(name);
        let contents: Vec<u8> = (0..size)
            .map(|index| u8::try_from((index * 7 + seed) % 251 + 1).unwrap())
            .collect();
        fs::write(&path, contents).unwrap();
        path
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_dir_all(&self.0) {
            eprintln!("Could not remove {}: {err}", self.0.display());
        }
    }
}

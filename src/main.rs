use clap::Parser;
use crate::cli::Args;
use crate::destination::Destination;
use crate::layout::Layout;

mod boards;
mod cli;
mod destination;
mod disk;
#[cfg(test)]
mod fixtures;
mod format;
mod layout;
mod mt7988;
mod repart;
mod rockchip;
mod values;

fn main() -> Result<(), String> {
    let opt = Args::parse();
    let layout = Layout::new(opt.layout_request()?)?;
    let destination = Destination::resolve(&opt.destination, opt.size)?;

    disk::flash(&destination, &layout)?;
    format::format_partitions(destination.path(), &opt.format_partition)
}

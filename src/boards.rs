use clap::Args;
use crate::layout::Placement;
use crate::rockchip;

#[derive(Args, Debug)]
pub struct Boards {
    #[command(flatten)]
    rockchip: Option<rockchip::Options>,
}

impl Boards {
    pub fn partitions(&self) -> Result<Vec<Placement>, String> {
        self.rockchip.as_ref().map_or(Ok(Vec::new()), rockchip::Options::board_partitions)
    }
}

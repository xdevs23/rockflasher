use clap::Args;
use crate::layout::Placement;
use crate::{mt7988, rockchip};

/// A board whose boot chain reads partitions at fixed places.
pub trait Board {
    fn partitions(&self) -> Result<Vec<Placement>, String>;
}

#[derive(Args, Debug)]
pub struct Boards {
    #[command(flatten)]
    rockchip: Option<rockchip::Options>,

    #[command(flatten)]
    mt7988: Option<mt7988::Options>,
}

impl Boards {
    pub fn partitions(&self) -> Result<Vec<Placement>, String> {
        let given: Vec<&dyn Board> = [
            self.rockchip.as_ref().map(|options| options as &dyn Board),
            self.mt7988.as_ref().map(|options| options as &dyn Board),
        ].into_iter().flatten().collect();
        match given.as_slice() {
            [] => Ok(Vec::new()),
            [board] => board.partitions(),
            _ => Err("The options of more than one board are given. Give the options of one board.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{layout, parse};

    #[test]
    fn fip_needs_bl2() {
        assert!(parse(&["--fip", "fip.bin"]).is_err());
        assert!(parse(&["--bl2", "bl2.img", "--fip", "fip.bin"]).is_ok());
    }

    #[test]
    fn options_of_two_boards_are_refused() {
        let err = layout(&["--idbloader", "idbloader.img", "--bl2", "bl2.img"]).err().unwrap();
        assert!(err.contains("more than one board"), "{err}");
    }
}

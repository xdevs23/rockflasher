use core::fmt;

pub const B: u64 = 1;
pub const KB: u64 = 1000 * B;
pub const MB: u64 = 1000 * KB;
pub const GB: u64 = 1000 * MB;
pub const TB: u64 = 1000 * GB;
pub const PB: u64 = 1000 * TB;
pub const EB: u64 = 1000 * PB;
pub const ZB: u128 = 1000 * EB as u128;
pub const YB: u128 = 1000 * ZB;

pub const KIB: u64 = 1024 * B;
pub const MIB: u64 = 1024 * KIB;
pub const GIB: u64 = 1024 * MIB;
pub const TIB: u64 = 1024 * GIB;
pub const PIB: u64 = 1024 * TIB;
pub const EIB: u64 = 1024 * PIB;
pub const ZIB: u128 = 1024 * EIB as u128;
pub const YIB: u128 = 1024 * ZIB;


pub struct BinarySize(pub u128);
pub struct RoundedBinarySize(pub u128, pub u8);

impl fmt::Display for BinarySize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            s if s.0 as u128 >= YIB => write!(f, "{} YiB", s.0 as u128 / YIB),
            s if s.0 as u128 >= ZIB => write!(f, "{} ZiB", s.0 as u128 / ZIB),
            s if s.0 as u128 >= EIB as u128 => write!(f, "{} EiB", s.0 as u128 / EIB as u128),
            s if s.0 as u64 >= PIB => write!(f, "{} PiB", s.0 as u64 / PIB),
            s if s.0 as u64 >= TIB => write!(f, "{} TiB", s.0 as u64 / TIB),
            s if s.0 as u64 >= GIB => write!(f, "{} GiB", s.0 as u64 / GIB),
            s if s.0 as u64 >= MIB => write!(f, "{} MiB", s.0 as u64 / MIB),
            s if s.0 as u64 >= KIB => write!(f, "{} KiB", s.0 as u64 / KIB),
            _ => write!(f, "{} B", self.0)
        }
    }
}

impl BinarySize {
    pub fn rounded_to(self, decimal_places: u8) -> RoundedBinarySize {
        RoundedBinarySize(self.0, decimal_places)
    }

    /// Rounds to 2 decimal places
    pub fn rounded(self) -> RoundedBinarySize {
        self.rounded_to(2)
    }
}

impl From<u128> for BinarySize {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<u64> for BinarySize {
    fn from(value: u64) -> Self {
        Self(value as u128)
    }
}

impl From<u32> for BinarySize {
    fn from(value: u32) -> Self {
        Self(value as u128)
    }
}

impl From<u16> for BinarySize {
    fn from(value: u16) -> Self {
        Self(value as u128)
    }
}

impl From<u8> for BinarySize {
    fn from(value: u8) -> Self {
        Self(value as u128)
    }
}

impl From<usize> for BinarySize {
    fn from(value: usize) -> Self {
        Self(value as u128)
    }
}

impl fmt::Display for RoundedBinarySize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            s if s.0 as u64 >= PIB => write!(f, "{:.dp$}", s.0 as f64 / PIB as f64, dp=s.1 as usize),
            s if s.0 as u64 >= TIB => write!(f, "{:.dp$} TiB", s.0 as f64 / TIB as f64, dp=s.1 as usize),
            s if s.0 as u64 >= GIB => write!(f, "{:.dp$} GiB", s.0 as f64 / GIB as f64, dp=s.1 as usize),
            s if s.0 as u64 >= MIB => write!(f, "{:.dp$} MiB", s.0 as f64 / MIB as f64, dp=s.1 as usize),
            s if s.0 as u64 >= KIB => write!(f, "{:.dp$} KiB", s.0 as f64 / KIB as f64, dp=s.1 as usize),
            _ => write!(f, "{} B", self.0)
        }
    }
}

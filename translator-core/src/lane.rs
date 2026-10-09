/// One of the two people facing each other across the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    One,
    Two,
}

impl Lane {
    pub fn other(self) -> Lane {
        match self {
            Lane::One => Lane::Two,
            Lane::Two => Lane::One,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_swaps_lanes() {
        assert_eq!(Lane::One.other(), Lane::Two);
        assert_eq!(Lane::Two.other(), Lane::One);
    }
}

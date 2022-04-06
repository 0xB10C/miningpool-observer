use std::convert::TryFrom;

#[derive(Clone, Copy)]
pub enum Sanctioneer {
    // the value is important for database backwards compatibilty
    // make sure to add new tag to the try_from fn below!
    OFAC = 1,
}

impl TryFrom<i32> for Sanctioneer {
    type Error = ();

    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            x if x == Sanctioneer::OFAC as i32 => Ok(Sanctioneer::OFAC),
            // FIXME: add new sanctioneers here
            _ => Err(()),
        }
    }
}

impl Sanctioneer {
    pub fn name(&self) -> &str {
        match self {
            Sanctioneer::OFAC => "OFAC",
        }
    }
}


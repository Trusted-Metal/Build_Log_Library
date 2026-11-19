//! The stages of a laser's data (decision D27-21), so that a result can say which of them it went through.
//!
//! * **Layer:** Processing. Plain values, nothing else.
//! * **Requirement:** BL-PR-03 (the stages that produced a value are named).

/// A stage that a laser's data has been through. The stages are in order, and a result names the last one that was applied.
// open-domain: the stages of the pipeline grow with its plans (the corrections will add to this list), and a consumer matches on the ones it knows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Stage {
    /// As stored in the file (the read selection of IP-27 phase 27C).
    Raw,
    /// In physical units: bits converted to millimetres ([`crate::z_conversion`], phase 27D). Only Z is built.
    Physical,
    /// Moved in time by the scanner card's timing offsets ([`crate::time_shift`], phase 27F).
    Shifted,
    /// Corrected with the correction grids. Not built yet: it comes when the corrector is ported over.
    Corrected,
}

impl Stage {
    /// The name of the stage, as it is written in documents and messages.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Raw => "raw",
            Stage::Physical => "physical",
            Stage::Shifted => "shifted",
            Stage::Corrected => "corrected",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stages_are_in_the_order_the_data_goes_through_them() {
        assert!(Stage::Raw < Stage::Physical);
        assert!(Stage::Physical < Stage::Shifted);
        assert!(Stage::Shifted < Stage::Corrected);
        let names: Vec<&str> = [
            Stage::Raw,
            Stage::Physical,
            Stage::Shifted,
            Stage::Corrected,
        ]
        .into_iter()
        .map(Stage::name)
        .collect();
        assert_eq!(names, ["raw", "physical", "shifted", "corrected"]);
    }
}

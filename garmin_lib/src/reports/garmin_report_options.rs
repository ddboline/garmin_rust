use crate::utils::sport_types::SportTypes;

#[derive(Debug, Clone, Copy)]
pub enum GarminReportAgg {
    Year,
    Month,
    Week,
    Day,
    File,
}

#[derive(Debug, Clone, Default)]
pub struct GarminReportOptions {
    pub agg: Option<GarminReportAgg>,
    pub do_sport: Option<SportTypes>,
}

impl GarminReportOptions {
    pub fn new() -> GarminReportOptions {
        GarminReportOptions {
            agg: None,
            do_sport: None,
        }
    }
}

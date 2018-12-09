use crate::utils::sport_types::SportTypes;

#[derive(Debug, Clone)]
pub struct GarminReportOptions {
    pub do_year: bool,
    pub do_month: bool,
    pub do_week: bool,
    pub do_day: bool,
    pub do_file: bool,
    pub do_sport: Option<SportTypes>,
}

impl GarminReportOptions {
    pub fn new() -> GarminReportOptions {
        GarminReportOptions {
            do_year: false,
            do_month: false,
            do_week: false,
            do_day: false,
            do_file: false,
            do_sport: None,
        }
    }
}

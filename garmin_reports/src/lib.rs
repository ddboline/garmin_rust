#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

pub mod garmin_constraints;
pub mod garmin_file_report_html;
pub mod garmin_file_report_txt;
pub mod garmin_report_options;
pub mod garmin_summary_report_html;
pub mod garmin_summary_report_txt;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

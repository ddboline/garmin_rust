#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::unsafe_derive_deserialize)]

pub mod garmin_util;
pub mod pgpool;
pub mod plot_graph;
pub mod plot_opts;
pub mod sport_types;

#[cfg(test)]
mod tests {
    use crate::garmin_util::{convert_time_string, convert_xml_local_time_to_utc, titlecase};
    use garmin_lib::date_time_wrapper::iso8601::convert_datetime_to_str;

    #[test]
    fn test_convert_time_string() {
        assert_eq!(convert_time_string("07:03:12.2").unwrap(), 25392.2);
        assert_eq!(
            format!("{}", convert_time_string("07:AB:12.2").err().unwrap()),
            "ParseIntError invalid digit found in string"
        );
    }

    #[test]
    fn test_convert_xml_local_time_to_utc() {
        assert_eq!(
            convert_datetime_to_str(
                convert_xml_local_time_to_utc("2011-05-07T15:43:07-04:00").unwrap()
            ),
            "2011-05-07T19:43:07Z"
        );
    }

    #[test]
    fn test_titlecase() {
        let input = "running";
        assert_eq!(titlecase(input), "Running");
    }
}

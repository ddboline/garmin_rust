use garmin_lib::utils::{
    date_time_wrapper::iso8601::convert_datetime_to_str,
    garmin_util::{convert_time_string, convert_xml_local_time_to_utc, titlecase},
    plot_graph, plot_opts,
};

#[test]
fn test_convert_time_string() {
    assert_eq!(convert_time_string("07:03:12.2").unwrap(), 25392.2);
    assert_eq!(
        format!("{}", convert_time_string("07:AB:12.2").err().unwrap()),
        "invalid digit found in string"
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
fn plot_graph() {
    let test_data = vec![(0.1, 0.2), (1.0, 2.0), (3.0, 4.0)];

    let plot_opts = plot_opts::PlotOpts::new()
        .with_labels("Test X label", "Test Y label")
        .with_marker("o")
        .with_name("test_plot")
        .with_title("test title")
        .with_data(&test_data);

    assert!(plot_graph::generate_d3_plot(&plot_opts)
        .unwrap()
        .contains(r#".text("test title")"#));
}

#[test]
fn test_titlecase() {
    let input = "running";
    assert_eq!(titlecase(input), "Running");
}

use stack_string::format_sstr;

use garmin_models::garmin_file::GarminFile;
use garmin_utils::{garmin_util::METERS_PER_MILE, plot_opts::{PlotOpts, DataPoint}};

use garmin_reports::garmin_file_report_txt::get_splits;

#[derive(Default, PartialEq, Clone)]
pub struct ReportObjects {
    pub avg_hr: f64,
    pub sum_time: f64,
    pub max_hr: f64,

    pub hr_vals: Vec<f64>,
    pub hr_values: Vec<DataPoint>,
    pub alt_vals: Vec<f64>,
    pub alt_values: Vec<DataPoint>,
    pub mph_speed_values: Vec<DataPoint>,
    pub avg_speed_values: Vec<DataPoint>,
    pub avg_mph_speed_values: Vec<DataPoint>,
    pub lat_vals: Vec<f64>,
    pub lon_vals: Vec<f64>,
    pub mile_split_vals: Vec<DataPoint>,
    pub speed_values: Vec<DataPoint>,
    pub heart_rate_speed: Vec<DataPoint>,
}

#[must_use]
pub fn extract_report_objects_from_file(gfile: &GarminFile) -> ReportObjects {
    let speed_values = get_splits(gfile, 400., "lap", true);
    let mut heart_rate_speed: Vec<_> = speed_values
        .iter()
        .map(|v| {
            let t = v.time_value;
            let h = v.avg_heart_rate.unwrap_or(0.0);
            DataPoint { x: h, y: 4.0 * t / 60.}
        })
        .collect();
    heart_rate_speed.shrink_to_fit();

    let mut speed_values: Vec<_> = speed_values
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            DataPoint { x: d / 4., y: 4. * t / 60.}
        })
        .collect();
    speed_values.shrink_to_fit();

    let mut mile_split_vals: Vec<_> = get_splits(gfile, METERS_PER_MILE, "mi", false)
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            DataPoint {x: d, y: t / 60.}
        })
        .collect();
    mile_split_vals.shrink_to_fit();

    let mut report_objs = ReportObjects {
        mile_split_vals,
        speed_values,
        heart_rate_speed,
        ..ReportObjects::default()
    };

    for point in &gfile.points {
        if point.distance.is_none() {
            continue;
        }
        let xval = point.distance.unwrap_or(0.0) / METERS_PER_MILE;
        if xval > 0.0 {
            if let Some(hr) = point.heart_rate {
                if hr > 0.0 {
                    report_objs.avg_hr += hr * point.duration_from_last;
                    report_objs.sum_time += point.duration_from_last;
                    report_objs.hr_vals.push(hr);
                    report_objs.hr_values.push(DataPoint {x: xval, y: hr});
                }
            }
        };
        if let Some(alt) = point.altitude {
            if (alt > 0.0) & (alt < 10000.0) {
                report_objs.alt_vals.push(alt);
                report_objs.alt_values.push(DataPoint { x: xval, y: alt});
            }
        };
        if (point.speed_mph > 0.0) & (point.speed_mph < 20.0) {
            report_objs.mph_speed_values.push(DataPoint {x: xval, y: point.speed_mph});
        };
        if (point.avg_speed_value_permi > 0.0) & (point.avg_speed_value_permi < 20.0) {
            report_objs
                .avg_speed_values
                .push(DataPoint {x: xval, y: point.avg_speed_value_permi});
        };
        if point.avg_speed_value_mph > 0.0 {
            report_objs
                .avg_mph_speed_values
                .push(DataPoint { x: xval, y: point.avg_speed_value_mph});
        };
        if let Some(lat) = point.latitude {
            if let Some(lon) = point.longitude {
                report_objs.lat_vals.push(lat);
                report_objs.lon_vals.push(lon);
            }
        };
    }

    if report_objs.sum_time > 0.0 {
        report_objs.avg_hr /= report_objs.sum_time;
        report_objs.max_hr = *report_objs
            .hr_vals
            .iter()
            .max_by_key(|&h| *h as i64)
            .unwrap_or(&0.0);
    };

    report_objs
}

#[must_use]
pub fn get_plot_opts(report_objs: &ReportObjects) -> Vec<PlotOpts> {
    let mut plot_opts = Vec::new();

    if !report_objs.mile_split_vals.is_empty() {
        plot_opts.push(
            PlotOpts::new()
                .with_name("mile_splits")
                .with_title("Pace per Mile every mi")
                .with_data(&report_objs.mile_split_vals)
                .with_marker("o")
                .with_labels("mi", "min/mi"),
        );
    };

    if !report_objs.hr_values.is_empty() {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heart_rate")
                .with_title(&format_sstr!(
                    "Heart Rate {:2.2} avg {:2.2} max",
                    report_objs.avg_hr,
                    report_objs.max_hr
                ))
                .with_data(&report_objs.hr_values)
                .with_labels("mi", "bpm"),
        );
    };

    if !report_objs.alt_values.is_empty() {
        plot_opts.push(
            PlotOpts::new()
                .with_name("altitude")
                .with_title("Altitude")
                .with_data(&report_objs.alt_values)
                .with_labels("mi", "height [m]"),
        );
    };

    if !report_objs.speed_values.is_empty() {
        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_minpermi")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&report_objs.speed_values)
                .with_labels("mi", "min/mi"),
        );

        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_mph")
                .with_title("Speed mph")
                .with_data(&report_objs.mph_speed_values)
                .with_labels("mi", "mph"),
        );
    };

    if !report_objs.heart_rate_speed.is_empty() {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heartrate_vs_speed")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&report_objs.heart_rate_speed)
                .with_scatter()
                .with_labels("hrt", "min/mi"),
        );
    };

    if !report_objs.avg_speed_values.is_empty() {
        let avg_speed_value = report_objs.avg_speed_values.last().map_or(0f64, |d| d.y);
        let avg_speed_value_min = avg_speed_value as i32;
        let avg_speed_value_sec =
            ((avg_speed_value - f64::from(avg_speed_value_min)) * 60.0) as i32;

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_minpermi")
                .with_title(&format_sstr!(
                    "Avg Speed {}:{:02} min/mi",
                    avg_speed_value_min,
                    avg_speed_value_sec
                ))
                .with_data(&report_objs.heart_rate_speed)
                .with_scatter()
                .with_labels("mi", "min/mi"),
        );
    };

    if !report_objs.avg_mph_speed_values.is_empty() {
        let avg_mph_speed_value = report_objs
            .avg_mph_speed_values
            .last()
            .map_or(0f64, |d| d.y);

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_mph")
                .with_title(&format_sstr!("Avg Speed {avg_mph_speed_value:.2} mph"))
                .with_data(&report_objs.avg_mph_speed_values)
                .with_scatter()
                .with_labels("mi", "min/mi"),
        );
    };

    plot_opts
}

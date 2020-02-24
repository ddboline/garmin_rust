use anyhow::Error;
use chrono::{Date, Datelike, Local};
use log::debug;
use std::collections::HashSet;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::common::garmin_config::GarminConfig;
use crate::common::garmin_file::GarminFile;
use crate::common::garmin_lap::GarminLap;
use crate::common::pgpool::PgPool;
use crate::common::strava_sync::get_strava_id_from_begin_datetime;
use crate::reports::garmin_file_report_txt::get_splits;
use crate::reports::garmin_templates::{GARMIN_TEMPLATE, MAP_TEMPLATE};
use crate::utils::garmin_util::{print_h_m_s, titlecase, MARATHON_DISTANCE_MI, METERS_PER_MILE};
use crate::utils::plot_graph::generate_d3_plot;
use crate::utils::plot_opts::PlotOpts;
use crate::utils::sport_types::SportTypes;

pub fn generate_history_buttons(history_vec: &[String]) -> String {
    let local: Date<Local> = Local::today();
    let year = local.year();
    let month = local.month();
    let (prev_year, prev_month) = if month > 1 {
        (year, month - 1)
    } else {
        (year - 1, 12)
    };
    let default_string = format!(
        "{:04}-{:02},{:04}-{:02},week",
        prev_year, prev_month, year, month
    );
    let mut used_buttons: HashSet<String> = HashSet::new();

    let history_buttons: Vec<_> = vec![default_string]
        .iter()
        .chain(history_vec.iter())
        .filter_map(|most_recent| {
            if used_buttons.contains(most_recent) {
                None
            } else {
                used_buttons.insert(most_recent.clone());
                Some(format!(
                    "{}{}{}{}{}",
                    r#"<button type="submit" onclick="send_command('filter="#,
                    most_recent,
                    r#"');"> "#,
                    most_recent,
                    " </button>"
                ))
            }
        })
        .collect();

    history_buttons.join("\n")
}

#[derive(Default)]
struct ReportObjects {
    avg_hr: f64,
    sum_time: f64,
    max_hr: f64,

    hr_vals: Vec<f64>,
    hr_values: Vec<(f64, f64)>,
    alt_vals: Vec<f64>,
    alt_values: Vec<(f64, f64)>,
    mph_speed_values: Vec<(f64, f64)>,
    avg_speed_values: Vec<(f64, f64)>,
    avg_mph_speed_values: Vec<(f64, f64)>,
    lat_vals: Vec<f64>,
    lon_vals: Vec<f64>,
    mile_split_vals: Vec<(f64, f64)>,
    speed_values: Vec<(f64, f64)>,
    heart_rate_speed: Vec<(f64, f64)>,
}

pub async fn file_report_html(
    config: &GarminConfig,
    gfile: &GarminFile,
    history: &[String],
    pool: &PgPool,
) -> Result<String, Error> {
    let report_objs = extract_report_objects_from_file(&gfile)?;
    let plot_opts = get_plot_opts(&report_objs);
    let graphs = get_graphs(&plot_opts);

    get_html_string(
        config,
        &gfile,
        &report_objs,
        &graphs,
        gfile.sport,
        &history,
        pool,
    )
    .await
}

fn extract_report_objects_from_file(gfile: &GarminFile) -> Result<ReportObjects, Error> {
    let mut report_objs = ReportObjects::default();

    report_objs.avg_hr = 0.0;
    report_objs.sum_time = 0.0;
    report_objs.max_hr = 0.0;

    report_objs.hr_vals = Vec::new();
    report_objs.hr_values = Vec::new();
    report_objs.alt_vals = Vec::new();
    report_objs.alt_values = Vec::new();
    report_objs.mph_speed_values = Vec::new();
    report_objs.avg_speed_values = Vec::new();
    report_objs.avg_mph_speed_values = Vec::new();
    report_objs.lat_vals = Vec::new();
    report_objs.lon_vals = Vec::new();

    let speed_values = get_splits(&gfile, 400., "lap", true)?;
    report_objs.heart_rate_speed = speed_values
        .iter()
        .map(|v| {
            let t = v.time_value;
            let h = v.avg_heart_rate.unwrap_or(0.0);
            (h, 4.0 * t / 60.)
        })
        .collect();

    report_objs.speed_values = speed_values
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            (d / 4., 4. * t / 60.)
        })
        .collect();

    report_objs.mile_split_vals = get_splits(&gfile, METERS_PER_MILE, "mi", false)?
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            (d, t / 60.)
        })
        .collect();

    for point in &gfile.points {
        if point.distance == None {
            continue;
        }
        let xval = point.distance.unwrap_or(0.0) / METERS_PER_MILE;
        if xval > 0.0 {
            if let Some(hr) = point.heart_rate {
                if hr > 0.0 {
                    report_objs.avg_hr += hr * point.duration_from_last;
                    report_objs.sum_time += point.duration_from_last;
                    report_objs.hr_vals.push(hr);
                    report_objs.hr_values.push((xval, hr));
                }
            }
        };
        if let Some(alt) = point.altitude {
            if (alt > 0.0) & (alt < 10000.0) {
                report_objs.alt_vals.push(alt);
                report_objs.alt_values.push((xval, alt));
            }
        };
        if (point.speed_mph > 0.0) & (point.speed_mph < 20.0) {
            report_objs.mph_speed_values.push((xval, point.speed_mph));
        };
        if (point.avg_speed_value_permi > 0.0) & (point.avg_speed_value_permi < 20.0) {
            report_objs
                .avg_speed_values
                .push((xval, point.avg_speed_value_permi));
        };
        if point.avg_speed_value_mph > 0.0 {
            report_objs
                .avg_mph_speed_values
                .push((xval, point.avg_speed_value_mph));
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

    Ok(report_objs)
}

fn get_plot_opts<'a>(report_objs: &'a ReportObjects) -> Vec<PlotOpts<'a>> {
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
                .with_title(
                    format!(
                        "Heart Rate {:2.2} avg {:2.2} max",
                        report_objs.avg_hr, report_objs.max_hr
                    )
                    .as_str(),
                )
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
        let (_, avg_speed_value) = report_objs.avg_speed_values.last().unwrap_or(&(0.0, 0.0));
        let avg_speed_value_min = *avg_speed_value as i32;
        let avg_speed_value_sec =
            ((*avg_speed_value - f64::from(avg_speed_value_min)) * 60.0) as i32;

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_minpermi")
                .with_title(
                    format!(
                        "Avg Speed {}:{:02} min/mi",
                        avg_speed_value_min, avg_speed_value_sec
                    )
                    .as_str(),
                )
                .with_data(&report_objs.heart_rate_speed)
                .with_scatter()
                .with_labels("mi", "min/mi"),
        );
    };

    if !report_objs.avg_mph_speed_values.is_empty() {
        let (_, avg_mph_speed_value) = report_objs
            .avg_mph_speed_values
            .last()
            .unwrap_or(&(0.0, 0.0));

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_mph")
                .with_title(format!("Avg Speed {:.2} mph", avg_mph_speed_value).as_str())
                .with_data(&report_objs.avg_mph_speed_values)
                .with_scatter()
                .with_labels("mi", "min/mi"),
        );
    };

    plot_opts
}

fn get_graphs(plot_opts: &[PlotOpts]) -> Vec<String> {
    plot_opts
        .par_iter()
        .filter_map(|options| match generate_d3_plot(&options) {
            Ok(s) => Some(s),
            Err(e) => {
                debug!("{}", e);
                None
            }
        })
        .collect()
}

async fn get_html_string(
    config: &GarminConfig,
    gfile: &GarminFile,
    report_objs: &ReportObjects,
    graphs: &[String],
    sport: SportTypes,
    history: &[String],
    pool: &PgPool,
) -> Result<String, Error> {
    let strava_id_title = get_strava_id_from_begin_datetime(pool, gfile.begin_datetime).await?;

    let htmlvec = if !report_objs.lat_vals.is_empty()
        & !report_objs.lon_vals.is_empty()
        & (report_objs.lat_vals.len() == report_objs.lon_vals.len())
    {
        get_map_tempate_vec(
            report_objs,
            gfile,
            sport,
            &strava_id_title,
            history,
            graphs,
            config,
        )?
    } else {
        get_garmin_template_vec(&config.domain, gfile, sport, &strava_id_title, history)?
    };

    Ok(htmlvec.join("\n"))
}

fn get_garmin_template_vec(
    domain: &str,
    gfile: &GarminFile,
    sport: SportTypes,
    strava_id_title: &Option<(String, String)>,
    history: &[String],
) -> Result<Vec<String>, Error> {
    let mut htmlvec = Vec::new();

    for line in GARMIN_TEMPLATE.split('\n') {
        if line.contains("INSERTTEXTHERE") {
            htmlvec.push(format!("{}\n", get_file_html(&gfile)));
            htmlvec.push(format!(
                "<br><br>{}\n",
                get_html_splits(&gfile, METERS_PER_MILE, "mi")?
            ));
            htmlvec.push(format!(
                "<br><br>{}\n",
                get_html_splits(&gfile, 5000.0, "km")?
            ));
        } else if line.contains("SPORTTITLEDATE") {
            let newtitle = format!(
                "Garmin Event {} on {}",
                titlecase(&sport.to_string()),
                gfile.begin_datetime
            );
            htmlvec.push(line.replace("SPORTTITLEDATE", &newtitle).to_string());
        } else if line.contains("SPORTTITLELINK") {
            let newtitle = format!(
                "Garmin Event {} on {}",
                titlecase(&sport.to_string()),
                gfile.begin_datetime
            );
            let newtitle = match strava_id_title.as_ref() {
                Some((id, title)) => format!(
                    r#"<a href="https://www.strava.com/activities/{}">{} {}</a>"#,
                    id, title, gfile.begin_datetime
                ),
                None => newtitle,
            };
            htmlvec.push(line.replace("SPORTTITLELINK", &newtitle).to_string());
        } else if line.contains("HISTORYBUTTONS") {
            let history_button = generate_history_buttons(history);
            htmlvec.push(line.replace("HISTORYBUTTONS", &history_button).to_string());
        } else if line.contains("DOMAIN") {
            htmlvec.push(line.replace("DOMAIN", domain));
        } else {
            htmlvec.push(
                line.replace("<pre>", "<div>")
                    .replace("</pre>", "</div>")
                    .to_string(),
            );
        }
    }
    Ok(htmlvec)
}

fn get_map_tempate_vec(
    report_objs: &ReportObjects,
    gfile: &GarminFile,
    sport: SportTypes,
    strava_id_title: &Option<(String, String)>,
    history: &[String],
    graphs: &[String],
    config: &GarminConfig,
) -> Result<Vec<String>, Error> {
    let minlat = report_objs
        .lat_vals
        .iter()
        .min_by_key(|&v| (v * 1000.0) as i32)
        .unwrap_or(&0.0);
    let maxlat = report_objs
        .lat_vals
        .iter()
        .max_by_key(|&v| (v * 1000.0) as i32)
        .unwrap_or(&0.0);
    let minlon = report_objs
        .lon_vals
        .iter()
        .min_by_key(|&v| (v * 1000.0) as i32)
        .unwrap_or(&0.0);
    let maxlon = report_objs
        .lon_vals
        .iter()
        .max_by_key(|&v| (v * 1000.0) as i32)
        .unwrap_or(&0.0);
    let central_lat = (maxlat + minlat) / 2.0;
    let central_lon = (maxlon + minlon) / 2.0;
    let latlon_min = if (maxlat - minlat) > (maxlon - minlon) {
        maxlat - minlat
    } else {
        maxlon - minlon
    };
    let latlon_thresholds = vec![
        (15, 0.015),
        (14, 0.038),
        (13, 0.07),
        (12, 0.12),
        (11, 0.20),
        (10, 0.4),
    ];

    let mut htmlvec = Vec::new();

    for line in MAP_TEMPLATE.split('\n') {
        if line.contains("SPORTTITLEDATE") {
            let newtitle = format!(
                "Garmin Event {} on {}",
                titlecase(&sport.to_string()),
                gfile.begin_datetime
            );
            htmlvec.push(line.replace("SPORTTITLEDATE", &newtitle).to_string());
        } else if line.contains("SPORTTITLELINK") {
            let newtitle = format!(
                "Garmin Event {} on {}",
                titlecase(&sport.to_string()),
                gfile.begin_datetime
            );
            let newtitle = match strava_id_title.as_ref() {
                Some((id, title)) => format!(
                    r#"<a href="https://www.strava.com/activities/{}">{} {}</a>"#,
                    id, title, gfile.begin_datetime
                ),
                None => newtitle,
            };
            htmlvec.push(line.replace("SPORTTITLELINK", &newtitle).to_string());
        } else if line.contains("STRAVAUPLOADBUTTON") {
            if let Some((id, _)) = strava_id_title.as_ref() {
                htmlvec.push(line.replace("STRAVAUPLOADBUTTON", &format!(r#"
                    <form>
                    <input type="text" name="cmd" id="strava_upload"/>
                    <input type="button" name="submitSTRAVA" value="Title" onclick="processStravaUpdate('{}');"/>
                    </form>
                "#,
                id
                )));
            } else {
                htmlvec.push(line.replace("STRAVAUPLOADBUTTON", r#"
                    <form>
                    <input type="text" name="cmd" id="strava_upload"/>
                    <input type="button" name="submitSTRAVA" value="Title" onclick="processStravaData();"/>
                    </form>
                "#));
            }
        } else if line.contains("ZOOMVALUE") {
            for (zoom, thresh) in &latlon_thresholds {
                if (latlon_min < *thresh) | (*zoom == 10) {
                    htmlvec.push(line.replace("ZOOMVALUE", &format!("{}", zoom)).to_string());
                    break;
                }
            }
        } else if line.contains("INSERTTABLESHERE") {
            htmlvec.push(format!("{}\n", get_file_html(&gfile)));
            htmlvec.push(format!(
                "<br><br>{}\n",
                get_html_splits(&gfile, METERS_PER_MILE, "mi")?
            ));
            htmlvec.push(format!(
                "<br><br>{}\n",
                get_html_splits(&gfile, 5000.0, "km")?
            ));
        } else if line.contains("INSERTMAPSEGMENTSHERE") {
            for (latv, lonv) in report_objs.lat_vals.iter().zip(report_objs.lon_vals.iter()) {
                htmlvec.push(format!("new google.maps.LatLng({},{}),", latv, lonv));
            }
        } else if line.contains("MINLAT")
            | line.contains("MAXLAT")
            | line.contains("MINLON")
            | line.contains("MAXLON")
        {
            htmlvec.push(
                line.replace("MINLAT", &format!("{}", minlat))
                    .replace("MAXLAT", &format!("{}", maxlat))
                    .replace("MINLON", &format!("{}", minlon))
                    .replace("MAXLON", &format!("{}", maxlon))
                    .to_string(),
            );
        } else if line.contains("CENTRALLAT") | line.contains("CENTRALLON") {
            htmlvec.push(
                line.replace("CENTRALLAT", &central_lat.to_string())
                    .replace("CENTRALLON", &central_lon.to_string())
                    .to_string(),
            );
        } else if line.contains("INSERTOTHERIMAGESHERE") {
            htmlvec.extend_from_slice(graphs)
        } else if line.contains("MAPSAPIKEY") {
            htmlvec.push(line.replace("MAPSAPIKEY", &config.maps_api_key).to_string());
        } else if line.contains("HISTORYBUTTONS") {
            let history_button = generate_history_buttons(history);
            htmlvec.push(line.replace("HISTORYBUTTONS", &history_button).to_string());
        } else if line.contains("FILENAME") | line.contains("ACTIVITYTYPE") {
            let filename = format!("{}/{}", &config.gps_dir, &gfile.filename);
            let activity_type = gfile.sport.to_strava_activity();
            htmlvec.push(
                line.replace("FILENAME", &filename)
                    .replace("ACTIVITYTYPE", &activity_type),
            );
        } else if line.contains("DOMAIN") {
            htmlvec.push(line.replace("DOMAIN", &config.domain));
        } else {
            htmlvec.push(line.to_string());
        };
    }
    Ok(htmlvec)
}

fn get_file_html(gfile: &GarminFile) -> String {
    let mut retval = Vec::new();

    let sport = gfile.sport.to_string();

    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(
        r#"<thead><tr style="text-align: center;"><th>Start Time</th>
                   <th>Sport</th></tr></thead>"#
            .to_string(),
    );
    retval.push(format!(
        "<tbody><tr style={0}text-align: center;{0}><td>{1}</td><td>{2}</td></tr></tbody>",
        '"', gfile.begin_datetime, sport
    ));
    retval.push(r#"</table><br>"#.to_string());

    let labels = vec![
        "Sport",
        "Lap",
        "Distance",
        "Duration",
        "Calories",
        "Time",
        "Pace / mi",
        "Pace / km",
        "Heart Rate",
    ];
    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());
    for label in labels {
        retval.push(format!("<th>{}</th>", label));
    }
    retval.push("</tr></thead>".to_string());
    retval.push("<tbody>".to_string());
    for lap in &gfile.laps {
        retval.push(r#"<tr style="text-align: center;">"#.to_string());
        for lap_html in get_lap_html(&lap, &sport) {
            retval.push(lap_html);
        }
        retval.push("</tr>".to_string());
    }

    let min_mile = if gfile.total_distance > 0.0 {
        (gfile.total_duration / 60.) / (gfile.total_distance / METERS_PER_MILE)
    } else {
        0.0
    };

    let mi_per_hr = if gfile.total_duration > 0.0 {
        (gfile.total_distance / METERS_PER_MILE) / (gfile.total_duration / 3600.)
    } else {
        0.0
    };

    let (mut labels, mut values) = match sport.as_str() {
        "running" => (
            vec![
                "".to_string(),
                "Distance".to_string(),
                "Calories".to_string(),
                "Time".to_string(),
                "Pace / mi".to_string(),
                "Pace / km".to_string(),
            ],
            vec![
                "total".to_string(),
                format!("{:.2} mi", gfile.total_distance / METERS_PER_MILE),
                format!("{}", gfile.total_calories),
                print_h_m_s(gfile.total_duration, true).unwrap_or_else(|_| "".to_string()),
                print_h_m_s(min_mile * 60.0, false).unwrap_or_else(|_| "".to_string()),
                print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false)
                    .unwrap_or_else(|_| "".to_string()),
            ],
        ),
        _ => (
            vec![
                "total".to_string(),
                "Distance".to_string(),
                "Calories".to_string(),
                "Time".to_string(),
                "Pace mph".to_string(),
            ],
            vec![
                "".to_string(),
                format!("{:.2} mi", gfile.total_distance / METERS_PER_MILE),
                format!("{}", gfile.total_calories),
                print_h_m_s(gfile.total_duration, true).unwrap_or_else(|_| "".to_string()),
                format!("{}", mi_per_hr),
            ],
        ),
    };

    if (gfile.total_hr_dur > 0.0)
        & (gfile.total_hr_dis > 0.0)
        & (gfile.total_hr_dur > gfile.total_hr_dis)
    {
        labels.push("Heart Rate".to_string());
        values.push(format!(
            "{} bpm",
            (gfile.total_hr_dur / gfile.total_hr_dis) as i32
        ));
    };

    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());

    for label in labels {
        retval.push(format!("<th>{}</th>", label));
    }

    retval.push("</tr></thead>".to_string());
    retval.push(r#"<tbody><tr style="text-align: center;">"#.to_string());

    for value in values {
        retval.push(format!("<td>{}</td>", value));
    }

    retval.push("</tr></tbody></table>".to_string());

    retval.join("\n")
}

fn get_lap_html(glap: &GarminLap, sport: &str) -> Vec<String> {
    let mut values = vec![
        sport.to_string(),
        format!("{}", glap.lap_number),
        format!("{:.2} mi", glap.lap_distance / METERS_PER_MILE),
        print_h_m_s(glap.lap_duration, true).unwrap_or_else(|_| "".to_string()),
        format!("{}", glap.lap_calories),
        format!("{:.2} min", glap.lap_duration / 60.),
    ];
    if glap.lap_distance > 0.0 {
        values.push(format!(
            "{} / mi",
            print_h_m_s(
                glap.lap_duration / (glap.lap_distance / METERS_PER_MILE),
                false
            )
            .unwrap_or_else(|_| "".to_string())
        ));
        values.push(format!(
            "{} / km",
            print_h_m_s(glap.lap_duration / (glap.lap_distance / 1000.), false)
                .unwrap_or_else(|_| "".to_string())
        ));
    }
    if let Some(lap_avg_hr) = glap.lap_avg_hr {
        values.push(format!("{} bpm", lap_avg_hr));
    }
    values.iter().map(|v| format!("<td>{}</td>", v)).collect()
}

fn get_html_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
) -> Result<String, Error> {
    if gfile.points.is_empty() {
        Ok("".to_string())
    } else {
        let labels = vec![
            "Split",
            "Time",
            "Pace / mi",
            "Pace / km",
            "Marathon Time",
            "Heart Rate",
        ];

        let values: Vec<_> = get_splits(gfile, split_distance_in_meters, label, true)?
            .into_iter()
            .map(|val| {
                let dis = val.split_distance as i32;
                let tim = val.time_value;
                let hrt = val.avg_heart_rate.unwrap_or(0.0) as i32;
                vec![
                    format!("{} {}", dis, label),
                    print_h_m_s(tim, true).unwrap_or_else(|_| "".to_string()),
                    print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false)
                        .unwrap_or_else(|_| "".to_string()),
                    print_h_m_s(tim / (split_distance_in_meters / 1000.), false)
                        .unwrap_or_else(|_| "".to_string()),
                    print_h_m_s(
                        tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                        true,
                    )
                    .unwrap_or_else(|_| "".to_string()),
                    format!("{} bpm", hrt),
                ]
            })
            .collect();

        let mut retval = Vec::new();
        retval.push(r#"<table border="1" class="dataframe">"#.to_string());
        retval.push(r#"<thead><tr style="text-align: center;">"#.to_string());
        for label in labels {
            retval.push(format!("<th>{}</th>", label));
        }
        retval.push("</tr></thead>".to_string());
        retval.push("<tbody>".to_string());
        for line in values {
            retval.push(r#"<tr style="text-align: center;">"#.to_string());
            for val in line {
                retval.push(format!("<td>{}</td>", val));
            }
            retval.push("</tr>".to_string());
        }
        retval.push("</tbody></table>".to_string());
        Ok(retval.join("\n"))
    }
}

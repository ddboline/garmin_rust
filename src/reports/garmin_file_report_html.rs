extern crate rayon;

use failure::Error;

use rayon::prelude::*;

use crate::garmin_file::GarminFile;
use crate::garmin_lap::GarminLap;
use crate::garmin_sync::{get_s3_client, upload_file_acl};
use crate::reports::garmin_file_report_txt::get_splits;
use crate::reports::garmin_templates::{GARMIN_TEMPLATE, MAP_TEMPLATE};
use crate::utils::garmin_util::{print_h_m_s, titlecase, MARATHON_DISTANCE_MI, METERS_PER_MILE};
use crate::utils::plot_graph::plot_graph;
use crate::utils::plot_opts::PlotOpts;

pub fn file_report_html(
    gfile: &GarminFile,
    maps_api_key: &str,
    cache_dir: &str,
    http_bucket: &str,
) -> Result<String, Error> {
    let sport = match &gfile.sport {
        Some(s) => s.clone(),
        None => "none".to_string(),
    };

    let mut avg_hr = 0.0;
    let mut sum_time = 0.0;
    let mut max_hr = 0.0;

    let mut hr_vals = Vec::new();
    let mut hr_values = Vec::new();
    let mut alt_vals = Vec::new();
    let mut alt_values = Vec::new();
    let mut mph_speed_values = Vec::new();
    let mut avg_speed_values = Vec::new();
    let mut avg_mph_speed_values = Vec::new();
    let mut lat_vals = Vec::new();
    let mut lon_vals = Vec::new();

    let speed_values = get_splits(&gfile, 400., "lap", true)?;
    let heart_rate_speed: Vec<_> = speed_values
        .iter()
        .map(|v| {
            let t = v.get(1).unwrap();
            let h = v.get(2).unwrap();
            (*h, 4.0 * t / 60.)
        })
        .collect();
    let speed_values: Vec<_> = speed_values
        .into_iter()
        .map(|v| {
            let d = v.get(0).unwrap();
            let t = v.get(1).unwrap();
            (d / 4., 4. * t / 60.)
        })
        .collect();
    let mile_split_vals = get_splits(&gfile, METERS_PER_MILE, "mi", false)?;
    let mile_split_vals: Vec<_> = mile_split_vals
        .into_iter()
        .map(|v| {
            let d = v.get(0).unwrap();
            let t = v.get(1).unwrap();
            (*d, t / 60.)
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
                    avg_hr += hr * point.duration_from_last;
                    sum_time += point.duration_from_last;
                    hr_vals.push(hr);
                    hr_values.push((xval, hr));
                }
            }
        };
        if let Some(alt) = point.altitude {
            if (alt > 0.0) & (alt < 10000.0) {
                alt_vals.push(alt);
                alt_values.push((xval, alt));
            }
        };
        if (point.speed_mph > 0.0) & (point.speed_mph < 20.0) {
            mph_speed_values.push((xval, point.speed_mph));
        };
        if (point.avg_speed_value_permi > 0.0) & (point.avg_speed_value_permi < 20.0) {
            avg_speed_values.push((xval, point.avg_speed_value_permi));
        };
        if point.avg_speed_value_mph > 0.0 {
            avg_mph_speed_values.push((xval, point.avg_speed_value_mph));
        };
        if let Some(lat) = point.latitude {
            if let Some(lon) = point.longitude {
                lat_vals.push(lat);
                lon_vals.push(lon);
            }
        };
    }
    if sum_time > 0.0 {
        avg_hr /= sum_time;
        max_hr = *hr_vals.iter().max_by_key(|&h| *h as i64).unwrap();
    };

    let mut plot_opts = Vec::new();

    if mile_split_vals.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("mile_splits")
                .with_title("Pace per Mile every mi")
                .with_data(&mile_split_vals)
                .with_marker("o")
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if hr_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heart_rate")
                .with_title(format!("Heart Rate {:2.2} avg {:2.2} max", avg_hr, max_hr).as_str())
                .with_data(&hr_values)
                .with_labels("mi", "bpm")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if alt_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("altitude")
                .with_title("Altitude")
                .with_data(&alt_values)
                .with_labels("mi", "height [m]")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if speed_values.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_minpermi")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&speed_values)
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );

        plot_opts.push(
            PlotOpts::new()
                .with_name("speed_mph")
                .with_title("Speed mph")
                .with_data(&mph_speed_values)
                .with_labels("mi", "mph")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if heart_rate_speed.len() > 0 {
        plot_opts.push(
            PlotOpts::new()
                .with_name("heartrate_vs_speed")
                .with_title("Speed min/mi every 1/4 mi")
                .with_data(&heart_rate_speed)
                .with_scatter()
                .with_labels("hrt", "min/mi")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if avg_speed_values.len() > 0 {
        let (_, avg_speed_value) = avg_speed_values.last().unwrap();
        let avg_speed_value_min = *avg_speed_value as i32;
        let avg_speed_value_sec = ((*avg_speed_value - avg_speed_value_min as f64) * 60.0) as i32;

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
                .with_data(&heart_rate_speed)
                .with_scatter()
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    if avg_mph_speed_values.len() > 0 {
        let (_, avg_mph_speed_value) = avg_mph_speed_values.last().unwrap();

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_mph")
                .with_title(format!("Avg Speed {:.2} mph", avg_mph_speed_value).as_str())
                .with_data(&avg_mph_speed_values)
                .with_scatter()
                .with_labels("mi", "min/mi")
                .with_cache_dir(&cache_dir)
                .with_http_bucket(&http_bucket),
        );
    };

    let graphs: Vec<_> = plot_opts
        .par_iter()
        .filter_map(|options| match plot_graph(&options) {
            Ok(x) => {
                let gf = x.trim().to_string();
                let s3_client = get_s3_client();
                let local_file = format!("{}/html/{}", cache_dir, gf);
                upload_file_acl(
                    &local_file,
                    &http_bucket,
                    &gf,
                    &s3_client,
                    Some("public-read".to_string()),
                )
                .unwrap();
                let uri = format!("https://s3.amazonaws.com/{}/{}", &http_bucket, &gf);
                Some(uri)
            }
            Err(err) => {
                println!("{}", err);
                None
            }
        })
        .collect();

    let mut htmlvec: Vec<String> = Vec::new();

    if (lat_vals.len() > 0) & (lon_vals.len() > 0) & (lat_vals.len() == lon_vals.len()) {
        let minlat = lat_vals
            .iter()
            .min_by_key(|&v| (v * 1000.0) as i32)
            .unwrap();
        let maxlat = lat_vals
            .iter()
            .max_by_key(|&v| (v * 1000.0) as i32)
            .unwrap();
        let minlon = lon_vals
            .iter()
            .min_by_key(|&v| (v * 1000.0) as i32)
            .unwrap();
        let maxlon = lon_vals
            .iter()
            .max_by_key(|&v| (v * 1000.0) as i32)
            .unwrap();
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

        for line in MAP_TEMPLATE.split("\n") {
            if line.contains("SPORTTITLEDATE") {
                let newtitle = format!(
                    "Garmin Event {} on {}",
                    titlecase(&sport),
                    gfile.begin_datetime
                );
                htmlvec.push(format!("{}", line.replace("SPORTTITLEDATE", &newtitle)));
            } else if line.contains("ZOOMVALUE") {
                for (zoom, thresh) in &latlon_thresholds {
                    if (latlon_min < *thresh) | (*zoom == 10) {
                        htmlvec.push(format!(
                            "{}",
                            line.replace("ZOOMVALUE", &format!("{}", zoom))
                        ));
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
                for (latv, lonv) in lat_vals.iter().zip(lon_vals.iter()) {
                    htmlvec.push(format!("new google.maps.LatLng({},{}),\n", latv, lonv));
                }
            } else if line.contains("MINLAT")
                | line.contains("MAXLAT")
                | line.contains("MINLON")
                | line.contains("MAXLON")
            {
                htmlvec.push(format!(
                    "{}",
                    line.replace("MINLAT", &format!("{}", minlat))
                        .replace("MAXLAT", &format!("{}", maxlat))
                        .replace("MINLON", &format!("{}", minlon))
                        .replace("MAXLON", &format!("{}", maxlon))
                ));
            } else if line.contains("CENTRALLAT") | line.contains("CENTRALLON") {
                htmlvec.push(format!(
                    "{}",
                    line.replace("CENTRALLAT", &format!("{}", central_lat))
                        .replace("CENTRALLON", &format!("{}", central_lon))
                ));
            } else if line.contains("INSERTOTHERIMAGESHERE") {
                for uri in &graphs {
                    htmlvec.push(format!("{}{}{}", r#"<p><img src=""#, uri, r#""></p>"#));
                }
            } else if line.contains("MAPSAPIKEY") {
                htmlvec.push(format!("{}", line.replace("MAPSAPIKEY", maps_api_key)));
            } else {
                htmlvec.push(format!("{}", line));
            };
        }
    } else {
        for line in GARMIN_TEMPLATE.split("\n") {
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
                    titlecase(&sport),
                    gfile.begin_datetime
                );
                htmlvec.push(format!("{}", line.replace("SPORTTITLEDATE", &newtitle)));
            } else {
                htmlvec.push(format!(
                    "{}",
                    line.replace("<pre>", "<div>").replace("</pre>", "</div>")
                ));
            }
        }
    };

    Ok(htmlvec.join("\n"))
}

fn get_file_html(gfile: &GarminFile) -> String {
    let mut retval = Vec::new();

    let sport = match &gfile.sport {
        Some(s) => s.clone(),
        None => "none".to_string(),
    };

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
                print_h_m_s(gfile.total_duration, true).unwrap(),
                print_h_m_s(min_mile * 60.0, false).unwrap(),
                print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false).unwrap(),
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
                print_h_m_s(gfile.total_duration, true).unwrap(),
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
        print_h_m_s(glap.lap_duration, true).unwrap(),
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
            .unwrap()
        ));
        values.push(format!(
            "{} / km",
            print_h_m_s(glap.lap_duration / (glap.lap_distance / 1000.), false).unwrap()
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
    if gfile.points.len() == 0 {
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

        let split_vector = get_splits(gfile, split_distance_in_meters, label, true)?;

        let values: Vec<_> = split_vector
            .iter()
            .map(|val| {
                let dis = *val.get(0).unwrap() as i32;
                let tim = val.get(1).unwrap();
                let hrt = *val.get(2).unwrap_or(&0.0) as i32;
                vec![
                    format!("{} {}", dis, label),
                    print_h_m_s(*tim, true).unwrap(),
                    print_h_m_s(*tim / (split_distance_in_meters / METERS_PER_MILE), false)
                        .unwrap(),
                    print_h_m_s(*tim / (split_distance_in_meters / 1000.), false).unwrap(),
                    print_h_m_s(
                        *tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                        true,
                    )
                    .unwrap(),
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

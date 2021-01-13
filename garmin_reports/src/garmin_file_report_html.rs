use anyhow::Error;
use chrono::{Date, DateTime, Datelike, Local, Utc};
use itertools::Itertools;
use log::debug;
use maplit::hashmap;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use stack_string::StackString;
use std::collections::HashSet;

use garmin_lib::{
    common::{
        fitbit_activity::FitbitActivity,
        garmin_config::GarminConfig,
        garmin_connect_activity::GarminConnectActivity,
        garmin_file::GarminFile,
        garmin_lap::GarminLap,
        garmin_templates::{get_buttons, get_scripts, get_style, HBR},
        pgpool::PgPool,
        strava_activity::StravaActivity,
    },
    utils::{
        garmin_util::{print_h_m_s, titlecase, MARATHON_DISTANCE_MI, METERS_PER_MILE},
        iso_8601_datetime::convert_datetime_to_str,
        plot_graph::generate_d3_plot,
        plot_opts::PlotOpts,
        sport_types::{get_sport_type_map, SportTypes},
    },
};
use race_result_analysis::race_results::RaceResults;

use crate::garmin_file_report_txt::get_splits;

pub fn generate_history_buttons<T: AsRef<str>>(history_vec: &[T]) -> StackString {
    fn history_button_string(most_recent: &str) -> StackString {
        format!(
            "{}{}{}{}{}",
            r#"<button type="submit" onclick="send_command('filter="#,
            most_recent,
            r#"');"> "#,
            most_recent,
            " </button>"
        )
        .into()
    }

    let local: Date<Local> = Local::today();
    let year = local.year();
    let month = local.month();
    let (prev_year, prev_month) = if month > 1 {
        (year, month - 1)
    } else {
        (year - 1, 12)
    };
    let default_string: StackString = format!(
        "{:04}-{:02},{:04}-{:02},week",
        prev_year, prev_month, year, month
    )
    .into();
    let mut used_buttons: HashSet<StackString> = HashSet::new();
    let mut history_buttons = vec![history_button_string(&default_string)];
    used_buttons.insert(default_string);

    for most_recent in history_vec.iter() {
        let most_recent: &str = most_recent.as_ref();
        if !used_buttons.contains(most_recent) {
            used_buttons.insert(most_recent.into());
            history_buttons.push(history_button_string(most_recent));
        }
    }

    history_buttons.join("\n").into()
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

pub async fn file_report_html<T: AsRef<str>>(
    config: &GarminConfig,
    gfile: &GarminFile,
    history: &[T],
    pool: &PgPool,
    is_demo: bool,
) -> Result<StackString, Error> {
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
        is_demo,
    )
    .await
}

fn extract_report_objects_from_file(gfile: &GarminFile) -> Result<ReportObjects, Error> {
    let speed_values = get_splits(&gfile, 400., "lap", true)?;
    let heart_rate_speed = speed_values
        .iter()
        .map(|v| {
            let t = v.time_value;
            let h = v.avg_heart_rate.unwrap_or(0.0);
            (h, 4.0 * t / 60.)
        })
        .collect();

    let speed_values = speed_values
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            (d / 4., 4. * t / 60.)
        })
        .collect();

    let mile_split_vals = get_splits(&gfile, METERS_PER_MILE, "mi", false)?
        .into_iter()
        .map(|v| {
            let d = v.split_distance;
            let t = v.time_value;
            (d, t / 60.)
        })
        .collect();

    let mut report_objs = ReportObjects {
        heart_rate_speed,
        speed_values,
        mile_split_vals,
        ..ReportObjects::default()
    };

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

fn get_plot_opts(report_objs: &ReportObjects) -> Vec<PlotOpts> {
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
                .with_title(&format!(
                    "Heart Rate {:2.2} avg {:2.2} max",
                    report_objs.avg_hr, report_objs.max_hr
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
        let (_, avg_speed_value) = report_objs.avg_speed_values.last().unwrap_or(&(0.0, 0.0));
        let avg_speed_value_min = *avg_speed_value as i32;
        let avg_speed_value_sec =
            ((*avg_speed_value - f64::from(avg_speed_value_min)) * 60.0) as i32;

        plot_opts.push(
            PlotOpts::new()
                .with_name("avg_speed_minpermi")
                .with_title(&format!(
                    "Avg Speed {}:{:02} min/mi",
                    avg_speed_value_min, avg_speed_value_sec
                ))
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
                .with_title(&format!("Avg Speed {:.2} mph", avg_mph_speed_value))
                .with_data(&report_objs.avg_mph_speed_values)
                .with_scatter()
                .with_labels("mi", "min/mi"),
        );
    };

    plot_opts
}

fn get_graphs(plot_opts: &[PlotOpts]) -> Vec<StackString> {
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

#[allow(clippy::too_many_arguments)]
async fn get_html_string<T, U>(
    config: &GarminConfig,
    gfile: &GarminFile,
    report_objs: &ReportObjects,
    graphs: &[T],
    sport: SportTypes,
    history: &[U],
    pool: &PgPool,
    is_demo: bool,
) -> Result<StackString, Error>
where
    T: AsRef<str>,
    U: AsRef<str>,
{
    let strava_activity = StravaActivity::get_by_begin_datetime(pool, gfile.begin_datetime).await?;
    let fitbit_activity = FitbitActivity::get_by_start_time(pool, gfile.begin_datetime).await?;
    let connect_activity =
        GarminConnectActivity::get_by_begin_datetime(pool, gfile.begin_datetime).await?;
    let race_result = RaceResults::get_race_by_filename(gfile.filename.as_str(), pool).await?;

    let htmlvec = if !report_objs.lat_vals.is_empty()
        & !report_objs.lon_vals.is_empty()
        & (report_objs.lat_vals.len() == report_objs.lon_vals.len())
    {
        get_map_tempate_vec(
            report_objs,
            gfile,
            sport,
            &strava_activity,
            &fitbit_activity,
            &connect_activity,
            &race_result,
            history,
            graphs,
            config,
            is_demo,
        )?
    } else {
        get_garmin_template_vec(
            config,
            gfile,
            sport,
            &strava_activity,
            &fitbit_activity,
            &connect_activity,
            &race_result,
            history,
            is_demo,
        )?
    };

    Ok(htmlvec.join("\n").into())
}

#[allow(clippy::too_many_arguments)]
fn get_garmin_template_vec<T: AsRef<str>>(
    config: &GarminConfig,
    gfile: &GarminFile,
    sport: SportTypes,
    strava_activity: &Option<StravaActivity>,
    fitbit_activity: &Option<FitbitActivity>,
    connect_activity: &Option<GarminConnectActivity>,
    race_result: &Option<RaceResults>,
    history: &[T],
    is_demo: bool,
) -> Result<Vec<StackString>, Error> {
    let insert_text_here = vec![
        format!(
            "{}\n",
            get_file_html(
                &gfile,
                strava_activity,
                fitbit_activity,
                connect_activity,
                race_result
            )
        ),
        format!(
            "<br><br>{}\n",
            get_html_splits(&gfile, METERS_PER_MILE, "mi")?
        ),
        format!("<br><br>{}\n", get_html_splits(&gfile, 5000.0, "km")?),
    ];
    let sport_title_link = format!(
        "Garmin Event {} on {}",
        titlecase(&sport.to_string()),
        gfile.begin_datetime
    );
    let sport_title_link = match strava_activity {
        Some(strava_activity) => format!(
            r#"<a href="https://www.strava.com/activities/{}" target="_blank">{} {}</a>"#,
            strava_activity.id, strava_activity.name, gfile.begin_datetime
        ),
        None => sport_title_link,
    };

    let button_str = if let Some(strava_activity) = strava_activity {
        format!(
            r#"<form>{}</form>"#,
            if is_demo {
                "".to_string()
            } else {
                format!(
                    r#"
                        <input type="text" name="cmd" id="strava_upload"/>
                        <input type="button" name="submitSTRAVA" value="Title"
                         onclick="processStravaUpdate({}, '{}', '{}');"/>
                    "#,
                    strava_activity.id,
                    gfile.sport.to_strava_activity(),
                    convert_datetime_to_str(strava_activity.start_date),
                )
            },
        )
    } else {
        "".to_string()
    };
    let history_buttons = generate_history_buttons(history);
    let insert_text_here = insert_text_here.join("\n");
    let sport_title_date = format!(
        "Garmin Event {} on {}",
        titlecase(&sport.to_string()),
        gfile.begin_datetime
    );
    let filename = config.gps_dir.join(gfile.filename.as_str());
    let filename = filename.to_string_lossy();
    let activity_type = gfile.sport.to_strava_activity();
    let buttons = get_buttons(is_demo).join("\n");
    let style = get_style(false);

    let params = hashmap! {
        "HISTORYBUTTONS" => history_buttons.as_str(),
        "INSERTTEXTHERE" => &insert_text_here,
        "SPORTTITLEDATE" => &sport_title_date,
        "SPORTTITLELINK" => &sport_title_link,
        "DOMAIN" => &config.domain,
        "FILENAME" => filename.as_ref(),
        "ACTIVITYTYPE" => &activity_type,
        "STRAVAUPLOADBUTTON" => &button_str,
        "GARMIN_STYLE" => &style,
        "GARMINBUTTONS" => &buttons,
        "GARMIN_SCRIPTS" => get_scripts(is_demo),
    };
    let body = HBR.render("GARMIN_TEMPLATE", &params)?;
    Ok(body.split('\n').map(Into::into).collect())
}

#[allow(clippy::too_many_arguments)]
fn get_map_tempate_vec<T, U>(
    report_objs: &ReportObjects,
    gfile: &GarminFile,
    sport: SportTypes,
    strava_activity: &Option<StravaActivity>,
    fitbit_activity: &Option<FitbitActivity>,
    connect_activity: &Option<GarminConnectActivity>,
    race_result: &Option<RaceResults>,
    history: &[T],
    graphs: &[U],
    config: &GarminConfig,
    is_demo: bool,
) -> Result<Vec<StackString>, Error>
where
    T: AsRef<str>,
    U: AsRef<str>,
{
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

    let sport_title_date = format!(
        "Garmin Event {} on {}",
        titlecase(&sport.to_string()),
        gfile.begin_datetime
    );
    let sport_title_link = match strava_activity {
        Some(strava_activity) => format!(
            r#"<a href="https://www.strava.com/activities/{}" target="_blank">{} {}</a>"#,
            strava_activity.id, strava_activity.name, gfile.begin_datetime
        ),
        None => format!(
            "Garmin Event {} on {}",
            titlecase(&sport.to_string()),
            gfile.begin_datetime
        ),
    };
    let button_str = if let Some(strava_activity) = strava_activity {
        format!(
            r#"<form>{}</form>"#,
            if is_demo {
                "".to_string()
            } else {
                format!(
                    r#"
                        <input type="text" name="cmd" id="strava_upload"/>
                        <input type="button" name="submitSTRAVA" value="Title"
                         onclick="processStravaUpdate({}, '{}', '{}');"/>
                    "#,
                    strava_activity.id,
                    gfile.sport.to_strava_activity(),
                    convert_datetime_to_str(strava_activity.start_date),
                )
            },
        )
    } else {
        format!(
            r#"<form>{}</form>"#,
            if is_demo {
                "".to_string()
            } else {
                format!(
                    r#"
                        <input type="text" name="cmd" id="strava_upload"/>
                        <input type="button" name="submitSTRAVA" value="Title"
                        onclick="processStravaData('{}', '{}');"/>
                    "#,
                    gfile.filename,
                    gfile.sport.to_strava_activity()
                )
            },
        )
    };
    let mut zoom_value = "".to_string();
    for (zoom, thresh) in &latlon_thresholds {
        if (latlon_min < *thresh) | (*zoom == 10) {
            zoom_value = zoom.to_string();
            break;
        }
    }
    let insert_table_here = format!(
        "{}\n<br><br>{}\n<br><br>{}\n",
        get_file_html(
            &gfile,
            strava_activity,
            fitbit_activity,
            connect_activity,
            race_result
        ),
        get_html_splits(&gfile, METERS_PER_MILE, "mi")?,
        get_html_splits(&gfile, 5000.0, "km")?
    );
    let map_segment = report_objs
        .lat_vals
        .iter()
        .zip(report_objs.lon_vals.iter())
        .map(|(latv, lonv)| format!("new google.maps.LatLng({},{}),", latv, lonv))
        .join("\n");
    let minlat = minlat.to_string();
    let maxlat = maxlat.to_string();
    let minlon = minlon.to_string();
    let maxlon = maxlon.to_string();
    let central_lat = central_lat.to_string();
    let central_lon = central_lon.to_string();
    let insert_other_images_here = graphs.iter().map(AsRef::as_ref).join("\n");
    let history_buttons = generate_history_buttons(history);
    let filename = config.gps_dir.join(gfile.filename.as_str());
    let filename = filename.to_string_lossy();
    let activity_type = gfile.sport.to_strava_activity();
    let buttons = get_buttons(is_demo).join("\n");

    let params = hashmap! {
        "CENTRALLAT" => central_lat.as_str(),
        "CENTRALLON" => &central_lon,
        "ZOOMVALUE" => &zoom_value,
        "INSERTMAPSEGMENTSHERE" => &map_segment,
        "MAPSAPIKEY" => &config.maps_api_key,
    };
    let google_maps_script = HBR.render("GOOGLE_MAP_SCRIPT", &params)?;
    let style = get_style(true);

    let params = hashmap! {
        "SPORTTITLEDATE" => sport_title_date.as_str(),
        "SPORTTITLELINK" => &sport_title_link,
        "STRAVAUPLOADBUTTON" => &button_str,
        "INSERTTABLESHERE" => &insert_table_here,
        "MINLAT" => &minlat,
        "MAXLAT" => &maxlat,
        "MINLON" => &minlon,
        "MAXLON" => &maxlon,
        "INSERTOTHERIMAGESHERE" => &insert_other_images_here,
        "HISTORYBUTTONS" => history_buttons.as_str(),
        "FILENAME" => filename.as_ref(),
        "ACTIVITYTYPE" => activity_type.as_str(),
        "DOMAIN" => &config.domain,
        "GARMIN_STYLE" => &style,
        "GARMINBUTTONS" => &buttons,
        "GARMIN_SCRIPTS" => get_scripts(is_demo),
        "GOOGLE_MAP_SCRIPT" => &google_maps_script,
    };
    let body = HBR.render("GARMIN_TEMPLATE", &params)?;
    Ok(body.split('\n').map(Into::into).collect())
}

fn get_sport_selector(current_sport: SportTypes) -> StackString {
    let current_sport = current_sport.to_string().into();
    let mut sport_types: Vec<_> = get_sport_type_map()
        .keys()
        .filter_map(|s| {
            if s == &current_sport {
                None
            } else {
                Some(s.clone())
            }
        })
        .collect();
    sport_types.sort();
    sport_types.insert(0, current_sport);
    let sport_types = sport_types
        .into_iter()
        .map(|s| format!(r#"<option value="{sport}">{sport}</option>"#, sport = s))
        .join("\n");
    format!(r#"<select id="sport_select">{}</select>"#, sport_types).into()
}

fn get_correction_button(begin_datetime: DateTime<Utc>) -> StackString {
    format!(
        r#"<button type="submit" onclick="addGarminCorrectionSport('{}')">Apply</button>"#,
        begin_datetime
    )
    .into()
}

fn get_file_html(
    gfile: &GarminFile,
    strava_activity: &Option<StravaActivity>,
    fitbit_activity: &Option<FitbitActivity>,
    connect_activity: &Option<GarminConnectActivity>,
    race_result: &Option<RaceResults>,
) -> StackString {
    let mut retval = Vec::new();

    let sport = gfile.sport.to_string();

    retval.push(r#"<table border="1" class="dataframe">"#.to_string());
    retval.push(
        r#"<thead><tr style="text-align: center;"><th>Start Time</th>
                   <th>Sport</th><th></th><th>FitbitID</th><th>Fitbit Steps</th><th>GarminConnectID</th>
                   <th>Garmin Steps</th><th>StravaID</th></tr></thead>"#
            .to_string(),
    );
    retval.push(format!(
        "<tbody><tr style={qt}text-align: \
         center;{qt}><td>{dt}</td><td>{sp}</td><td>{gc}</td><td>{fid}</td><td>{fstep}</td>
         <td>{gid}</td><td>{gstep}</td><td>{sid}</td></tr></tbody>",
        qt = '"',
        dt=gfile.begin_datetime,
        sp=get_sport_selector(gfile.sport),
        gc=get_correction_button(gfile.begin_datetime),
        sid = if let Some(strava_activity) = strava_activity {
            format!(
                r#"<a href="https://www.strava.com/activities/{0}" target="_blank">{0}</a>"#,
                strava_activity.id,
            )
        } else {
            format!(
                r#"<button type="submit" onclick="createStravaActivity('{}');">create</button>"#,
                gfile.filename,
            )
        },
        fid = if let Some(fitbit_activity) = fitbit_activity {
            format!(
                r#"<a href="https://www.fitbit.com/activities/exercise/{0}" target="_blank">{0}</a>"#,
                fitbit_activity.log_id,
            )
        } else {
            "".to_string()
        },
        fstep = fitbit_activity.as_ref().map_or(0, |x| x.steps.unwrap_or(0)),
        gid = if let Some(connect_activity) = connect_activity {
            format!(
                r#"<a href="https://connect.garmin.com/modern/activity/{0}" target="_blank">{0}</a>"#,
                connect_activity.activity_id,
            )
        } else {
            "".to_string()
        },
        gstep = connect_activity.as_ref().map_or(0, |x| x.steps.unwrap_or(0)),
    ));
    retval.push(r#"</table><br>"#.to_string());
    if race_result.is_none() && gfile.sport == SportTypes::Running {
        retval.push(format!(
            r#"<button type="submit" onclick="raceResultImport('{}');">ImportRace</button>"#,
            gfile.filename,
        ));
    }

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
            retval.push(lap_html.into());
        }
        retval.push("</tr>".to_string());
    }
    retval.push(r#"</table><br>"#.to_string());

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
                print_h_m_s(gfile.total_duration, true)
                    .unwrap_or_else(|_| "".into())
                    .into(),
                print_h_m_s(min_mile * 60.0, false)
                    .unwrap_or_else(|_| "".into())
                    .into(),
                print_h_m_s(min_mile * 60.0 / METERS_PER_MILE * 1000., false)
                    .unwrap_or_else(|_| "".into())
                    .into(),
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
                print_h_m_s(gfile.total_duration, true)
                    .unwrap_or_else(|_| "".into())
                    .into(),
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

    retval.join("\n").into()
}

fn get_lap_html(glap: &GarminLap, sport: &str) -> Vec<StackString> {
    let mut values = vec![
        sport.to_string(),
        format!("{}", glap.lap_number),
        format!("{:.2} mi", glap.lap_distance / METERS_PER_MILE),
        print_h_m_s(glap.lap_duration, true)
            .unwrap_or_else(|_| "".into())
            .into(),
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
            .unwrap_or_else(|_| "".into())
        ));
        values.push(format!(
            "{} / km",
            print_h_m_s(glap.lap_duration / (glap.lap_distance / 1000.), false)
                .unwrap_or_else(|_| "".into())
        ));
    }
    if let Some(lap_avg_hr) = glap.lap_avg_hr {
        values.push(format!("{} bpm", lap_avg_hr));
    }
    values
        .iter()
        .map(|v| format!("<td>{}</td>", v).into())
        .collect()
}

fn get_html_splits(
    gfile: &GarminFile,
    split_distance_in_meters: f64,
    label: &str,
) -> Result<StackString, Error> {
    if gfile.points.is_empty() {
        Ok("".into())
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
                    print_h_m_s(tim, true).unwrap_or_else(|_| "".into()).into(),
                    print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false)
                        .unwrap_or_else(|_| "".into())
                        .into(),
                    print_h_m_s(tim / (split_distance_in_meters / 1000.), false)
                        .unwrap_or_else(|_| "".into())
                        .into(),
                    print_h_m_s(
                        tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                        true,
                    )
                    .unwrap_or_else(|_| "".into())
                    .into(),
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
        Ok(retval.join("\n").into())
    }
}

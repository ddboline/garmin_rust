use dioxus::prelude::{component, dioxus_elements, rsx, Element, IntoDynNode, Props, VirtualDom};
use itertools::Itertools;
use serde::Serialize;
use stack_string::{format_sstr, StackString};
use std::{collections::HashMap, fmt::Write};
use time::{macros::format_description, Date, Duration, OffsetDateTime};
use time_tz::OffsetDateTimeExt;

#[cfg(debug_assertions)]
use dioxus::prelude::{GlobalSignal, Readable};

use fitbit_lib::{
    fitbit_heartrate::FitbitHeartRate,
    scale_measurement::{ScaleMeasurement, LBS_PER_KG},
};
use garmin_lib::{
    date_time_wrapper::{iso8601::convert_datetime_to_str, DateTimeWrapper},
    garmin_config::GarminConfig,
};
use garmin_models::{
    garmin_connect_activity::{GarminConnectActivity, GarminConnectSocialProfile},
    garmin_file::GarminFile,
    garmin_summary::GarminSummary,
    strava_activity::StravaActivity,
};
use garmin_reports::{
    garmin_file_report_txt::get_splits,
    garmin_summary_report_txt::{GarminReportQuery, HtmlResult},
};
use garmin_utils::{
    garmin_util::{print_h_m_s, titlecase, MARATHON_DISTANCE_MI, METERS_PER_MILE},
    pgpool::PgPool,
    plot_graph::{generate_plot_data, ScatterPlotData},
    sport_types::{get_sport_type_map, SportTypes},
};
use race_result_analysis::{
    race_result_analysis::{PlotData, RaceResultAnalysis},
    race_results::RaceResults,
    race_type::RaceType,
};
use strava_lib::strava_client::StravaAthlete;

use crate::{
    errors::ServiceError as Error,
    garmin_file_report_html::{extract_report_objects_from_file, get_plot_opts, ReportObjects},
    FitbitStatisticsSummary,
};

#[derive(PartialEq, Clone)]
struct HeartrateOpts {
    heartrate: Vec<FitbitHeartRate>,
    button_date: Option<Date>,
}

pub enum IndexConfig {
    Report {
        reports: GarminReportQuery,
    },
    File {
        gfile: GarminFile,
    },
    Scale {
        measurements: Vec<ScaleMeasurement>,
        offset: usize,
        start_date: Date,
        end_date: Date,
    },
    HearRateSummary {
        stats: Vec<FitbitStatisticsSummary>,
        offset: Option<usize>,
        start_date: Option<Date>,
        end_date: Option<Date>,
    },
    HeartRate {
        heartrate: Vec<FitbitHeartRate>,
        start_date: Date,
        end_date: Date,
        button_date: Option<Date>,
    },
    RaceResult {
        model: RaceResultAnalysis,
    },
}

/// # Errors
/// Return error if deserialization fails
pub async fn index_new_body(
    config: &GarminConfig,
    pool: &PgPool,
    title: StackString,
    is_demo: bool,
    history: Vec<StackString>,
    index_config: IndexConfig,
) -> Result<String, Error> {
    let map_api_key = config.maps_api_key.clone();
    match index_config {
        IndexConfig::Report { reports } => {
            let mut url_strings = reports.get_url_strings();
            url_strings.shrink_to_fit();
            let mut reports = reports.get_text_entries().map_err(Into::<Error>::into)?;
            reports.shrink_to_fit();
            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports,
                    url_strings,
                    plot_reports: None,
                    gfile: None,
                    strava_activity: None,
                    connect_activity: None,
                    race_result: None,
                    is_demo,
                    map_api_key,
                    history,
                    measurements: Vec::new(),
                    offset: None,
                    start_date: None,
                    end_date: None,
                    heartrate_stats: Vec::new(),
                    heartrate_opts: None,
                    model: None,
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
        IndexConfig::File { gfile } => {
            let report_objs = extract_report_objects_from_file(&gfile);

            let summary = GarminSummary::get_by_filename(pool, &gfile.filename).await?;
            let strava_activity = if let Some(s) = &summary {
                StravaActivity::get_from_summary_id(pool, s.id).await?
            } else {
                None
            };
            let connect_activity = if let Some(s) = &summary {
                GarminConnectActivity::get_from_summary_id(pool, s.id).await?
            } else {
                None
            };
            let race_result = if let Some(s) = &summary {
                RaceResults::get_race_by_summary_id(s.id, pool).await?
            } else {
                None
            };

            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports: Vec::new(),
                    url_strings: Vec::new(),
                    plot_reports: Some(report_objs),
                    gfile: Some(gfile),
                    strava_activity,
                    connect_activity,
                    race_result,
                    is_demo,
                    map_api_key,
                    history,
                    measurements: Vec::new(),
                    offset: None,
                    start_date: None,
                    end_date: None,
                    heartrate_stats: Vec::new(),
                    heartrate_opts: None,
                    model: None,
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
        IndexConfig::Scale {
            measurements,
            offset,
            start_date,
            end_date,
        } => {
            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports: Vec::new(),
                    url_strings: Vec::new(),
                    plot_reports: None,
                    gfile: None,
                    strava_activity: None,
                    connect_activity: None,
                    race_result: None,
                    is_demo,
                    map_api_key,
                    history,
                    measurements,
                    offset: Some(offset),
                    start_date: Some(start_date),
                    end_date: Some(end_date),
                    heartrate_stats: Vec::new(),
                    heartrate_opts: None,
                    model: None,
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
        IndexConfig::HearRateSummary {
            stats,
            offset,
            start_date,
            end_date,
        } => {
            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports: Vec::new(),
                    url_strings: Vec::new(),
                    plot_reports: None,
                    gfile: None,
                    strava_activity: None,
                    connect_activity: None,
                    race_result: None,
                    is_demo,
                    map_api_key,
                    history,
                    measurements: Vec::new(),
                    offset,
                    start_date,
                    end_date,
                    heartrate_stats: stats,
                    heartrate_opts: None,
                    model: None,
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
        IndexConfig::HeartRate {
            heartrate,
            start_date,
            end_date,
            button_date,
        } => {
            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports: Vec::new(),
                    url_strings: Vec::new(),
                    plot_reports: None,
                    gfile: None,
                    strava_activity: None,
                    connect_activity: None,
                    race_result: None,
                    is_demo,
                    map_api_key,
                    history,
                    measurements: Vec::new(),
                    offset: None,
                    start_date: Some(start_date),
                    end_date: Some(end_date),
                    heartrate_stats: Vec::new(),
                    heartrate_opts: Some(HeartrateOpts {
                        heartrate,
                        button_date,
                    }),
                    model: None,
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
        IndexConfig::RaceResult { model } => {
            let mut app = VirtualDom::new_with_props(
                IndexElement,
                IndexElementProps {
                    title,
                    reports: Vec::new(),
                    url_strings: Vec::new(),
                    plot_reports: None,
                    gfile: None,
                    strava_activity: None,
                    connect_activity: None,
                    race_result: None,
                    is_demo,
                    map_api_key,
                    history,
                    measurements: Vec::new(),
                    offset: None,
                    start_date: None,
                    end_date: None,
                    heartrate_stats: Vec::new(),
                    heartrate_opts: None,
                    model: Some(model),
                    config: config.clone(),
                },
            );
            app.rebuild_in_place();
            let mut renderer = dioxus_ssr::Renderer::default();
            let mut buffer = String::new();
            renderer
                .render_to(&mut buffer, &app)
                .map_err(Into::<Error>::into)?;
            Ok(buffer)
        }
    }
}

#[component]
fn IndexElement(
    title: StackString,
    reports: Vec<Vec<(StackString, Option<HtmlResult>)>>,
    url_strings: Vec<StackString>,
    plot_reports: Option<ReportObjects>,
    gfile: Option<GarminFile>,
    strava_activity: Option<StravaActivity>,
    connect_activity: Option<GarminConnectActivity>,
    race_result: Option<RaceResults>,
    is_demo: bool,
    map_api_key: StackString,
    history: Vec<StackString>,
    measurements: Vec<ScaleMeasurement>,
    offset: Option<usize>,
    start_date: Option<Date>,
    end_date: Option<Date>,
    heartrate_stats: Vec<FitbitStatisticsSummary>,
    heartrate_opts: Option<HeartrateOpts>,
    model: Option<RaceResultAnalysis>,
    config: GarminConfig,
) -> Element {
    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    struct HeartRatePoint {
        x: StackString,
        y: i32,
    }

    #[derive(Serialize)]
    struct TimeSeriesPoint {
        x: StackString,
        y: f64,
    }

    struct TimeSeriesData {
        data: Vec<TimeSeriesPoint>,
        title: &'static str,
        xaxis: &'static str,
        yaxis: &'static str,
        units: &'static str,
    }

    let offset = offset.unwrap_or(0);
    let history_buttons = generate_history_buttons(&history);
    let buttons = get_buttons(is_demo);
    let mut sport_title: Option<Element> = None;
    let mut button_str: Option<Element> = None;
    let mut script_box: Option<Element> = None;
    let mut text_box: Option<Element> = None;
    let mut table_box: Option<Element> = None;
    let mut image_box: Option<Element> = None;
    let local = DateTimeWrapper::local_tz();

    if let Some(model) = model {
        script_box.replace(create_analysis_plot(&model, is_demo));
    }
    if let Some(HeartrateOpts {
        heartrate,
        button_date,
    }) = heartrate_opts
    {
        let start_date: Date = start_date.map_or_else(
            || {
                (OffsetDateTime::now_utc() - Duration::days(3))
                    .to_timezone(local)
                    .date()
            },
            Into::into,
        );
        let end_date: Date = end_date.map_or_else(
            || OffsetDateTime::now_utc().to_timezone(local).date(),
            Into::into,
        );
        let button_date = button_date.map_or_else(
            || OffsetDateTime::now_utc().to_timezone(local).date(),
            Into::into,
        );
        let mut final_values: Vec<_> = heartrate
            .iter()
            .chunk_by(|hv| hv.datetime.unix_timestamp() / (5 * 60))
            .into_iter()
            .filter_map(|(_, group)| {
                let (begin_datetime, entries, heartrate_sum) = group.fold(
                    (None, 0, 0),
                    |(begin_datetime, entries, heartrate_sum),
                     FitbitHeartRate {
                         datetime,
                         value: heartrate,
                     }| {
                        (
                            if begin_datetime.is_none() || begin_datetime < Some(datetime) {
                                Some(datetime)
                            } else {
                                begin_datetime
                            },
                            entries + 1,
                            heartrate_sum + heartrate,
                        )
                    },
                );
                begin_datetime.map(|begin_datetime| {
                    let average_heartrate = heartrate_sum / entries;
                    let begin_datetime_str = begin_datetime
                        .format(format_description!(
                            "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour \
                             sign:mandatory]:[offset_minute]"
                        ))
                        .unwrap_or_else(|_| String::new())
                        .into();
                    HeartRatePoint {
                        x: begin_datetime_str,
                        y: average_heartrate,
                    }
                })
            })
            .collect();
        final_values.shrink_to_fit();
        final_values.sort();
        let data = serde_json::to_string(&final_values).unwrap_or_else(|_| String::new());
        let mut script_body = String::new();
        script_body.push_str("\n!function(){\n");
        writeln!(&mut script_body, "\tlet data = {data};").unwrap();
        writeln!(
            &mut script_body,
            "\ttime_series(data, 'Heart Rate', 'Date', 'Heart Rate', 'bpm');"
        )
        .unwrap();
        script_body.push_str("}();\n");
        let date_input = {
            rsx! {
                input {
                    "type": "date",
                    name: "start-date",
                    id: "start_date_selector_heart",
                    value: "{start_date}",
                }
                input {
                    "type": "date",
                    name: "end-date",
                    id: "end_date_selector_heart",
                    value: "{end_date}",
                }
                button {
                    "type": "submit",
                    "onclick": "heartrate_plot_button('{start_date}', '{end_date}', '{button_date}')",
                    "Update",
                }
            }
        };
        let date_buttons = (0..5).map(move |i| {
            let date = button_date - Duration::days(i64::from(i));
            let update_button = if is_demo {
                None
            } else {
                Some(rsx! {
                    button {
                        "type": "submit",
                        id: "ID",
                        "onclick": "heartrate_sync('{date}');",
                        "Sync {date}",
                    }
                })
            };
            rsx! {
                div {
                    key: "date-button-key-{i}",
                    button {
                        "type": "submit",
                        id: "ID",
                        "onclick": "heartrate_plot_button_single('{date}', '{button_date}')",
                        "Plot {date}",
                    },
                    {update_button},
                }
            }
        });
        let prev_date = button_date + Duration::days(5);
        let next_date = button_date - Duration::days(5);
        let today = OffsetDateTime::now_utc().to_timezone(local).date();
        let prev_button = if prev_date <= today {
            Some(rsx! {
                button {
                    "type": "submit",
                    "onclick": "heartrate_plot_button('{start_date}', '{end_date}', '{prev_date}');",
                    "Prev",
                }
            })
        } else {
            None
        };
        script_box.replace(rsx! {
            div {
                {date_input},
            }
            div {
                {date_buttons},
            }
            br {
                {prev_button},
                button {
                    "type": "submit",
                    "onclick": "heartrate_plot_button('{start_date}', '{end_date}', '{next_date}');",
                    "Next",
                },
            },
            script {
                dangerous_inner_html: "{script_body}",
            },
        });
    }
    if !heartrate_stats.is_empty() {
        let start_date: Date = start_date.map_or_else(
            || {
                (OffsetDateTime::now_utc() - Duration::days(365))
                    .to_timezone(local)
                    .date()
            },
            Into::into,
        );
        let end_date: Date = end_date.map_or_else(
            || OffsetDateTime::now_utc().to_timezone(local).date(),
            Into::into,
        );
        let mut plots = Vec::new();
        let dformat = format_description!("[year]-[month]-[day]T00:00:00Z");
        let mut min_heartrate: Vec<TimeSeriesPoint> = heartrate_stats
            .iter()
            .map(|stat| {
                let x = stat
                    .date
                    .format(dformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: stat.min_heartrate,
                }
            })
            .collect();
        min_heartrate.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: min_heartrate,
            title: "Minimum Heartrate",
            xaxis: "Date",
            yaxis: "Heatrate [bpm]",
            units: "bpm",
        });
        let mut max_heartrate: Vec<TimeSeriesPoint> = heartrate_stats
            .iter()
            .map(|stat| {
                let x = stat
                    .date
                    .format(dformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: stat.max_heartrate,
                }
            })
            .collect();
        max_heartrate.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: max_heartrate,
            title: "Maximum Heartrate",
            xaxis: "Date",
            yaxis: "Heatrate [bpm]",
            units: "bpm",
        });
        let mut mean_heartrate: Vec<TimeSeriesPoint> = heartrate_stats
            .iter()
            .map(|stat| {
                let x = stat
                    .date
                    .format(dformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: stat.mean_heartrate,
                }
            })
            .collect();
        mean_heartrate.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: mean_heartrate,
            title: "Mean Heartrate",
            xaxis: "Date",
            yaxis: "Heatrate [bpm]",
            units: "bpm",
        });
        let graphs = plots.into_iter().enumerate().map(|(idx, plot)| {
            let data = serde_json::to_string(&plot.data).unwrap_or_else(|_| String::new());
            let title = plot.title;
            let xaxis = plot.xaxis;
            let yaxis = plot.yaxis;
            let units = plot.units;
            let mut script_body = String::new();
            script_body.push_str("\n!function(){\n");
            writeln!(&mut script_body, "\tlet data = {data};").unwrap();
            writeln!(
                &mut script_body,
                "\ttime_series(data, '{title}', '{xaxis}', '{yaxis}', '{units}');"
            )
            .unwrap();
            script_body.push_str("}();\n");
            rsx! {
                script {
                    key: "scale-script-key-{idx}",
                    dangerous_inner_html: "{script_body}",
                }
            }
        });
        let n = heartrate_stats.len();
        let lower = n.saturating_sub(offset + 10);
        let upper = n.saturating_sub(offset);
        let entries = heartrate_stats[lower..upper]
            .iter()
            .enumerate()
            .map(|(idx, stat)| {
                let date = stat.date;
                let min = stat.min_heartrate;
                let max = stat.max_heartrate;
                let mnh = stat.mean_heartrate;
                let mdh = stat.median_heartrate;
                rsx! {
                    tr {
                        key: "heartrate-stat-key-{idx}",
                        td {"{date}"},
                        td {"{min:3.1}"},
                        td {"{max:2.1}"},
                        td {"{mnh:2.1}"},
                        td {"{mdh:2.1}"},
                    }
                }
            });
        let prev_button = if offset >= 10 {
            let o = offset - 10;
            Some(rsx! {
                button {
                    "type": "submit",
                    "onclick": "heartrate_stat_plot({o}, '{start_date}', '{end_date}')",
                    "Previous",
                }
            })
        } else {
            None
        };
        let o = offset + 10;
        let next_button = rsx! {
            button {
                "type": "submit",
                "onclick": "heartrate_stat_plot({o}, '{start_date}', '{end_date}')",
                "Next",
            }
        };
        let date_input = {
            rsx! {
                input {
                    "type": "date",
                    name: "start-date",
                    id: "start_date_selector_stat",
                    value: "{start_date}",
                }
                input {
                    "type": "date",
                    name: "end-date",
                    id: "end_date_selector_stat",
                    value: "{end_date}",
                }
                button {
                    "type": "submit",
                    "onclick": "heartrate_stat_plot({offset}, '{start_date}', '{end_date}')",
                    "Update",
                }
            }
        };
        script_box.replace(rsx! {
            table {
                "border": "1",
                thead {
                    th {"Date"},
                    th {"Min"}
                    th {"Max"},
                    th {"Mean"},
                    th {"Median"},
                },
                tbody {
                    {entries},
                },
            },
            br {
                {prev_button},
                {next_button},
            },
            div {
                {date_input}
            },
            {graphs},
        });
    }
    if !measurements.is_empty() {
        let tformat = format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour \
             sign:mandatory]:[offset_minute]"
        );
        let start_date: Date = start_date.map_or_else(
            || {
                (OffsetDateTime::now_utc() - Duration::days(365))
                    .to_timezone(local)
                    .date()
            },
            Into::into,
        );
        let end_date: Date = end_date.map_or_else(
            || OffsetDateTime::now_utc().to_timezone(local).date(),
            Into::into,
        );

        let mut plots = Vec::new();

        let mut mass: Vec<TimeSeriesPoint> = measurements
            .iter()
            .map(|meas| {
                let x = meas
                    .datetime
                    .format(tformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint { x, y: meas.mass }
            })
            .collect();
        mass.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: mass,
            title: "Weight",
            xaxis: "Date",
            yaxis: "Weight [lbs]",
            units: "lbs",
        });
        let mut fat: Vec<TimeSeriesPoint> = measurements
            .iter()
            .map(|meas| {
                let x = meas
                    .datetime
                    .format(tformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint { x, y: meas.fat_pct }
            })
            .collect();
        fat.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: fat,
            title: "Fat %",
            xaxis: "Date",
            yaxis: "Fat %",
            units: "%",
        });
        let mut water: Vec<TimeSeriesPoint> = measurements
            .iter()
            .map(|meas| {
                let x = meas
                    .datetime
                    .format(tformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: meas.water_pct,
                }
            })
            .collect();
        water.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: water,
            title: "Water %",
            xaxis: "Date",
            yaxis: "Water %",
            units: "%",
        });
        let mut muscle: Vec<TimeSeriesPoint> = measurements
            .iter()
            .map(|meas| {
                let x = meas
                    .datetime
                    .format(tformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: meas.muscle_pct,
                }
            })
            .collect();
        muscle.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: muscle,
            title: "Muscle %",
            xaxis: "Date",
            yaxis: "Muscle %",
            units: "%",
        });
        let mut bone: Vec<TimeSeriesPoint> = measurements
            .iter()
            .map(|meas| {
                let x = meas
                    .datetime
                    .format(tformat)
                    .unwrap_or_else(|_| String::new())
                    .into();
                TimeSeriesPoint {
                    x,
                    y: meas.bone_pct,
                }
            })
            .collect();
        bone.shrink_to_fit();
        plots.push(TimeSeriesData {
            data: bone,
            title: "Bone %",
            xaxis: "Date",
            yaxis: "Bone %",
            units: "%",
        });
        let graphs = plots.into_iter().enumerate().map(|(idx, plot)| {
            let data = serde_json::to_string(&plot.data).unwrap_or_else(|_| String::new());
            let title = plot.title;
            let xaxis = plot.xaxis;
            let yaxis = plot.yaxis;
            let units = plot.units;
            let mut script_body = String::new();
            script_body.push_str("\n!function(){\n");
            writeln!(&mut script_body, "\tlet data = {data};").unwrap();
            writeln!(
                &mut script_body,
                "\ttime_series(data, '{title}', '{xaxis}', '{yaxis}', '{units}');"
            )
            .unwrap();
            script_body.push_str("}();\n");
            rsx! {
                script {
                    key: "scale-script-key-{idx}",
                    dangerous_inner_html: "{script_body}",
                }
            }
        });
        let n = measurements.len();
        let lower = n.saturating_sub(offset + 10);
        let upper = n.saturating_sub(offset);
        let entries = measurements[lower..upper]
            .iter()
            .enumerate()
            .map(|(idx, meas)| {
                let date = meas.datetime.to_timezone(local).date();
                let m = meas.mass;
                let f = meas.fat_pct;
                let w = meas.water_pct;
                let ms = meas.muscle_pct;
                let b = meas.bone_pct;
                let bmi = meas.get_bmi(&config);
                let date_element = if meas.connect_primary_key.is_some() {
                    rsx! {
                        a {
                            href: "https://connect.garmin.com/modern/weight/{date}/3",
                            target: "_blank",
                            "{date}",
                        }
                    }
                } else {
                    rsx! { "{date}" }
                };
                rsx! {
                    tr {
                        key: "measurement-key-{idx}",
                        td {
                            {date_element},
                        },
                        td {"{m:3.1}"},
                        td {"{f:2.1}"},
                        td {"{w:2.1}"},
                        td {"{ms:2.1}"},
                        td {"{b:2.1}"},
                        td {"{bmi:2.1}"},
                    }
                }
            });
        let prev_button = if offset >= 10 {
            let o = offset - 10;
            Some(rsx! {
                button {
                    "type": "submit",
                    "onclick": "scale_measurement_plots({o}, '{start_date}', '{end_date}')",
                    "Previous",
                }
            })
        } else {
            None
        };
        let o = offset + 10;
        let next_button = rsx! {
            button {
                "type": "submit",
                "onclick": "scale_measurement_plots({o}, '{start_date}', '{end_date}')",
                "Next",
            }
        };
        let date_input = {
            rsx! {
                input {
                    "type": "date",
                    name: "start-date",
                    id: "start_date_selector_scale",
                    value: "{start_date}",
                }
                input {
                    "type": "date",
                    name: "end-date",
                    id: "end_date_selector_scale",
                    value: "{end_date}",
                }
                button {
                    "type": "submit",
                    "onclick": "scale_measurement_plots({offset}, '{start_date}', '{end_date}')",
                    "Update",
                }
            }
        };
        script_box.replace(rsx! {
            button {
                "type": "submit",
                "onclick": "manualScaleMeasurement();",
                "Manual Scale Measurement Input",
            }
            div {
                id: "scale_measurement_box",
                table {
                    "border": "1",
                    thead {
                        th {"Date"},
                        th {
                            a {
                                href: "https://connect.garmin.com/modern/weight",
                                target: "_blank",
                                "Weight",
                            }
                        }
                        th {"Fat %"},
                        th {"Water %"},
                        th {"Muscle %"},
                        th {"Bone %"},
                        th {"BMI kg/m^2"},
                    },
                    tbody {
                        {entries},
                    },
                },
                br {
                    {prev_button},
                    {next_button},
                },
                div {
                    {date_input}
                },
                {graphs},
            }
        });
    }
    let report_str =
        reports
            .iter()
            .zip(url_strings.iter())
            .enumerate()
            .map(|(idx, (text_entries, cmd))| {
                let entries = text_entries.iter().enumerate().map(|(jdx, (s, u))| {
                    let entry = u.as_ref().map_or(rsx! {"{s}"}, |u| match u {
                        HtmlResult {
                            text: Some(t),
                            url: Some(u),
                        } => rsx! {
                            a {href: "{u}", target: "_blank", "{t}"},
                        },
                        HtmlResult {
                            text: Some(t),
                            url: None,
                        } => rsx! {
                            div {
                                dangerous_inner_html: "{t}",
                            }
                        },
                        HtmlResult {
                            text: None,
                            url: Some(u),
                        } => rsx! {
                            a {href: "{u}", target: "_blank", "link"},
                        },
                        _ => rsx! {""},
                    });
                    rsx! {
                        td {
                            key: "report-entry-{jdx}",
                            {entry}
                        }
                    }
                });
                rsx! {
                    tr {
                        key: "report-key-{idx}",
                        td {
                            button {
                                "type": "submit",
                                "onclick": "send_command('filter={cmd}')",
                                "{cmd}",
                            }
                        },
                        {entries}
                    },
                }
            });
    if let Some(report_objs) = plot_reports {
        if !report_objs.lat_vals.is_empty()
            & !report_objs.lon_vals.is_empty()
            & (report_objs.lat_vals.len() == report_objs.lon_vals.len())
        {
            if let Some(gfile) = gfile {
                let plot_opts = get_plot_opts(&report_objs);
                let graphs = plot_opts.into_iter().enumerate().filter_map(|(idx, opts)| {
                    let data = opts.data.as_ref()?;
                    if data.is_empty() {
                        return None;
                    }
                    let title = &opts.title;
                    let xlabel = &opts.xlabel;
                    let ylabel = &opts.ylabel;
                    if let Some(ScatterPlotData { data, xstep, ystep }) =
                        generate_plot_data(&opts, data)
                    {
                        let data = serde_json::to_string(&data).unwrap_or_else(|_| String::new());
                        let mut script_body = String::new();
                        script_body.push_str("\n!function(){\n");
                        writeln!(&mut script_body, "\tlet data = {data};").unwrap();
                        writeln!(
                            &mut script_body,
                            "\tscatter_plot(data, '{title}', '{xlabel}', '{ylabel}', {xstep}, \
                             {ystep});"
                        )
                        .unwrap();
                        script_body.push_str("}();\n");
                        Some(rsx! {
                            script {
                                key: "plot-key-{idx}",
                                dangerous_inner_html: "{script_body}",
                            }
                        })
                    } else {
                        let mut script_body = String::new();
                        script_body.push_str("\n!function(){\n");
                        let data = serde_json::to_string(&data).unwrap_or_else(|_| String::new());
                        writeln!(&mut script_body, "\tlet data = {data};").unwrap();
                        writeln!(
                            &mut script_body,
                            "\tline_plot(data, '{title}', '{xlabel}', '{ylabel}');"
                        )
                        .unwrap();
                        script_body.push_str("}();\n");
                        Some(rsx! {
                            script {
                                key: "plot-key-{idx}",
                                dangerous_inner_html: "{script_body}",
                            }
                        })
                    }
                });
                image_box.replace(rsx! {
                    {graphs}
                });

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
                let sport_str = gfile.sport.to_str();
                let sport_title_link = if let Some(strava_activity) = &strava_activity {
                    let id = strava_activity.id;
                    let name = &strava_activity.name;
                    let dt = gfile.begin_datetime;
                    rsx! {
                        a {
                            href: "https://www.strava.com/activities/{id}",
                            target: "_blank",
                            "{name} {dt}",
                        }
                    }
                } else {
                    let s = titlecase(sport_str);
                    let dt = gfile.begin_datetime;
                    rsx! {"Garmin Event {s} on {dt}"}
                };
                sport_title.replace(sport_title_link);
                if !is_demo {
                    if let Some(strava_activity) = &strava_activity {
                        let id = strava_activity.id;
                        let s = gfile.sport.to_strava_activity();
                        let dt = convert_datetime_to_str(strava_activity.start_date.into());
                        button_str.replace(rsx! {
                            form {
                                input {
                                    "type": "text",
                                    name: "cmd",
                                    id: "strava_upload",
                                },
                                input {
                                    "type": "button",
                                    name: "submitSTRAVA",
                                    value: "Title",
                                    "onclick": "processStravaUpdate({id}, '{s}', '{dt}');"
                                }
                            }
                        });
                    } else {
                        let f = &gfile.filename;
                        let s = gfile.sport.to_strava_activity();
                        button_str.replace(rsx! {
                            form {
                                input {
                                    "type": "text",
                                    name: "cmd",
                                    id: "strava_upload",
                                },
                                input {
                                    "type": "button",
                                    name: "submitSTRAVA",
                                    value: "Title",
                                    "onclick": "processStravaData('{f}', '{s}');",
                                }
                            }
                        });
                    }
                }
                let mut zoom_value = StackString::new();
                for (zoom, thresh) in &latlon_thresholds {
                    if (latlon_min < *thresh) | (*zoom == 10) {
                        zoom_value = StackString::from_display(zoom);
                        break;
                    }
                }
                let map_segment = report_objs
                    .lat_vals
                    .iter()
                    .zip(report_objs.lon_vals.iter())
                    .map(|(latv, lonv)| format_sstr!("new google.maps.LatLng({latv}, {lonv})"))
                    .join(",");
                let mut script_body = String::new();
                script_body.push_str("\n!function(){\n");
                writeln!(
                    &mut script_body,
                    "\tlet runningRouteCoordinates = [{map_segment}];"
                )
                .unwrap();
                writeln!(
                    &mut script_body,
                    "\tinitialize({central_lat}, {central_lon}, {zoom_value}, \
                     runningRouteCoordinates);"
                )
                .unwrap();
                script_body.push_str("}();\n");
                script_box.replace(rsx! {
                    script {
                        dangerous_inner_html: "{script_body}",
                    }
                });
                let file_html = Some(get_file_html(
                    &gfile,
                    strava_activity.as_ref(),
                    connect_activity.as_ref(),
                    race_result.as_ref(),
                ));
                let splits_mi = Some(get_html_splits(&gfile, METERS_PER_MILE, "mi"));
                let splits_5k = Some(get_html_splits(&gfile, 5000.0, "km"));
                table_box.replace(rsx! {
                    div {
                        {file_html},
                        {splits_mi},
                        {splits_5k},
                    }
                });
            }
        } else if let Some(gfile) = gfile {
            let file_html = Some(get_file_html(
                &gfile,
                strava_activity.as_ref(),
                connect_activity.as_ref(),
                race_result.as_ref(),
            ));
            let splits_mi = Some(get_html_splits(&gfile, METERS_PER_MILE, "mi"));
            let splits_5k = Some(get_html_splits(&gfile, 5000.0, "km"));
            text_box.replace(rsx! {
                div {
                    {file_html},
                    {splits_mi},
                    {splits_5k},
                }
            });
        }
    } else if !reports.is_empty() {
        text_box.replace(rsx! {
            table {
                "border": "0",
                {report_str},
            }
        });
    }
    let upload_button = if is_demo {
        None
    } else {
        Some(rsx! {
            form {
                action: "/garmin/upload_file",
                method: "post",
                enctype: "multipart/form-data",
                input {
                    "type": "file",
                    name: "filename",
                },
                input {"type": "submit"},
            }
        })
    };

    rsx! {
        head {
            title {"{title}"},
            meta {
                name: "viewport",
                content: "initial-scale=1.0, user-scalable=no",
            },
            meta {
                charset: "utf-8",
            },
            meta {
                "http-equiv": "Cache-Control",
                content: "no-store",
            }
            style {
                dangerous_inner_html: include_str!("../../templates/style.css")
            }
        },
        body {
            h3 {
                {buttons},
            },
            form {
                action: "javascript:processFormData()",
                method: "get",
                input {
                    "type": "text",
                    name: "cmd",
                    id: "garmin_filter",
                },
                input {
                    "type": "button",
                    name: "submit_input",
                    value: "Submit",
                    "onclick": "processFormData()"
                }
            }
            {history_buttons},
            br {
                {upload_button},
                {button_str},
            },
            h1 {
                style: "text-align: center",
                b { {sport_title} },
            },
            script {src: "https://d3js.org/d3.v4.min.js"},
            script {src: "/garmin/scripts/garmin_scripts.js"},
            script {src: "/garmin/scripts/line_plot.js"},
            script {src: "/garmin/scripts/scatter_plot.js"},
            script {src: "/garmin/scripts/time_series.js"},
            script {
                "type": "text/javascript",
                src: "https://maps.googleapis.com/maps/api/js?key={map_api_key}",
            },
            script {src: "/garmin/scripts/initialize_map.js"},
            {script_box},
            div {
                id: "garmin_text_box",
                {text_box},
            },
            div {
                id: "garmin_table_box",
                {table_box},
            },
            div {
                id: "garmin_image_box",
                {image_box},
            },
        }
    }
}

fn get_file_html(
    gfile: &GarminFile,
    strava_activity: Option<&StravaActivity>,
    connect_activity: Option<&GarminConnectActivity>,
    race_result: Option<&RaceResults>,
) -> Element {
    let dt = gfile.begin_datetime;
    let sp = {
        let current_sport = gfile.sport.to_str();
        let mut sport_types: Vec<_> = get_sport_type_map()
            .keys()
            .filter_map(|s| if *s == current_sport { None } else { Some(*s) })
            .collect();
        sport_types.shrink_to_fit();
        sport_types.sort_unstable();
        sport_types.insert(0, current_sport);
        let sport_types = sport_types.into_iter().enumerate().map(|(idx, s)| {
            rsx! {
                option {
                    key: "sport-types-key-{idx}",
                    value: "{s}",
                    "{s}",
                }
            }
        });
        rsx! {
            select {
                id: "sport_select",
                {sport_types},
            }
        }
    };
    let begin_datetime = gfile.begin_datetime;
    let gc = rsx! {
        button {
            "type": "submit",
            "onclick": "addGarminCorrectionSport('{begin_datetime}')",
            "Apply",
        }
    };
    let sid = if let Some(strava_activity) = strava_activity {
        let id = strava_activity.id;
        rsx! {
            a {
                href: "https://www.strava.com/activities/{id}",
                target: "_blank",
                "{id}",
            }
        }
    } else {
        let filename = &gfile.filename;
        rsx! {
            a {
                button {
                    "type": "submit",
                    "onclick": "createStravaActivity('{filename}');",
                    "create",
                }
            }
        }
    };
    let gid = connect_activity.as_ref().map(|connect_activity| {
        let activity_id = connect_activity.activity_id;
        rsx! {
            a {
                href: "https://connect.garmin.com/modern/activity/{activity_id}",
                target: "_blank",
                "{activity_id}",
            }
        }
    });
    let gstep = connect_activity
        .as_ref()
        .map_or(0, |x| x.steps.unwrap_or(0));
    let import_button = if race_result.is_none() && gfile.sport == SportTypes::Running {
        let filename = &gfile.filename;
        Some(rsx! {
            button {
                "type": "submit",
                "onclick": "raceResultImport('{filename}');",
                "ImportRace",
            }
        })
    } else {
        None
    };

    let labels = [
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

    rsx! {
        table {
            "border": "1",
            class: "dataframe",
            thead {
                tr {
                    "style": "text-align: center;",
                    th {"Start Time"},
                    th {"Sport"},
                    th {},
                    th {"GarminConnectID"},
                    th {"Garmin Steps"},
                    th {"StravaID"},
                }
            },
            tbody {
                tr {
                    "style": "text-align: center;",
                    td {"{dt}"},
                    td { {sp} },
                    td { {gc} },
                    td { {gid} },
                    td {"{gstep}"},
                    td { {sid} },
                }
            }
        },
        {import_button},
        br {
            table {
                "border": "1",
                class: "dataframe",
                thead {
                    tr {
                        "type": "text-align: center;",
                        {labels.iter().enumerate().map(|(idx, label)| {
                            rsx! {
                                th {
                                    key: "label-key-{idx}",
                                    "{label}"
                                },
                            }
                        })}
                    }
                },
                tbody {
                    {gfile.laps.iter().enumerate().map(|(idx, lap)| {
                        let mut values = vec![
                            gfile.sport.into(),
                            format_sstr!("{}", lap.lap_number),
                            format_sstr!("{:.2} mi", lap.lap_distance / METERS_PER_MILE),
                            print_h_m_s(lap.lap_duration, true).unwrap_or_else(|_| "".into()),
                            format_sstr!("{}", lap.lap_calories),
                            format_sstr!("{:.2} min", lap.lap_duration / 60.),
                        ];
                        if lap.lap_distance > 0.0 {
                            values.push(format_sstr!(
                                "{} / mi",
                                print_h_m_s(
                                    lap.lap_duration / (lap.lap_distance / METERS_PER_MILE),
                                    false
                                )
                                .unwrap_or_else(|_| "".into())
                            ));
                            values.push(format_sstr!(
                                "{} / km",
                                print_h_m_s(lap.lap_duration / (lap.lap_distance / 1000.), false)
                                    .unwrap_or_else(|_| "".into())
                            ));
                        }
                        if let Some(lap_avg_hr) = lap.lap_avg_hr {
                            values.push(format_sstr!("{lap_avg_hr} bpm"));
                        }
                        rsx! {
                            tr {
                                key: "lap-key-{idx}",
                                "type": "text-align: center;",
                                {values.iter().enumerate().map(|(i, v)| rsx! {
                                    td {
                                        key: "v-key-{i}",
                                        "{v}"
                                    }
                                })},
                            }
                        }
                    })}
                }
            }
        }
    }
}

fn get_html_splits(gfile: &GarminFile, split_distance_in_meters: f64, label: &str) -> Element {
    let labels = [
        "Split",
        "Time",
        "Pace / mi",
        "Pace / km",
        "Marathon Time",
        "Heart Rate",
    ];
    let values = get_splits(gfile, split_distance_in_meters, label, true)
        .into_iter()
        .enumerate()
        .map(move |(idx, val)| {
            let dis = val.split_distance as i32;
            let tim = val.time_value;
            let hrt = val.avg_heart_rate.unwrap_or(0.0) as i32;
            let tim0 = print_h_m_s(tim, true).unwrap_or_else(|_| "".into());
            let tim1 = print_h_m_s(tim / (split_distance_in_meters / METERS_PER_MILE), false)
                .unwrap_or_else(|_| "".into());
            let tim2 = print_h_m_s(tim / (split_distance_in_meters / 1000.), false)
                .unwrap_or_else(|_| "".into());
            let tim3 = print_h_m_s(
                tim / (split_distance_in_meters / METERS_PER_MILE) * MARATHON_DISTANCE_MI,
                true,
            )
            .unwrap_or_else(|_| "".into());
            rsx! {
                tr {
                    key: "split-key-{idx}",
                    td {"{dis} {label}"},
                    td {"{tim0}"},
                    td {"{tim1}"},
                    td {"{tim2}"},
                    td {"{tim3}"},
                    td {"{hrt} bpm"},
                }
            }
        });

    rsx! {
        table {
            "border": "1",
            class: "dataframe",
            thead {
                tr {
                    "style": "text-align: center;",
                    {labels.iter().enumerate().map(|(idx, label)| {
                        rsx! {
                            th {
                                key: "label-key-{idx}",
                                "{label}",
                            }
                        }
                    })},
                }
            },
            tbody {
                {values},
            }
        }
    }
}

fn generate_history_buttons(history_vec: &[StackString]) -> Element {
    let local = DateTimeWrapper::local_tz();
    let local = OffsetDateTime::now_utc().to_timezone(local).date();
    let year = local.year();
    let month: u8 = local.month().into();
    let (prev_year, prev_month) = if month > 1 {
        (year, month - 1)
    } else {
        (year - 1, 12)
    };
    let default_string = format_sstr!("{prev_year:04}-{prev_month:02},{year:04}-{month:02},week");
    let mut history = history_vec.to_vec();
    if !history.contains(&default_string) {
        history.insert(0, default_string);
    }
    rsx! {
        {history.into_iter().enumerate().map(move |(idx, filter)| {
            rsx! {
                button {
                    key: "history-key-{idx}",
                    "type": "submit",
                    "onclick": "send_command('filter={filter}')",
                    "{filter}",
                }
            }
        })}
    }
}

fn get_buttons(demo: bool) -> Element {
    let top_buttons: Option<Element> = if demo {
        None
    } else {
        Some(rsx! {
            button {
                "type": "submit",
                "onclick": "garmin_sync();",
                "Sync with S3",
            },
            button {
                "type": "submit",
                "onclick": "stravaAthlete();",
                "Strava Athlete",
            },
            button {
                "type": "submit",
                "onclick": "garminConnectProfile();",
                "Garmin Connect Profile",
            }
            button {
                "type": "submit",
                "onclick": "heartrateSync();",
                "Scale sync",
            },
        })
    };
    rsx! {
        br {
            {top_buttons},
        }
        button {
            "type": "submit",
            "onclick": "scale_measurement_plots(0);",
            "Scale Plots",
        },
        button {
            "type": "submit",
            "onclick": "heartrate_stat_plot(0);",
            "Heart Rate Stats",
        },
        button {
            "type": "submit",
            "onclick": "heartrate_plot();",
            "Heart Rate Plots",
        },
        button {
            "type": "submit",
            "onclick": "race_result_plot_personal();",
            "Race Result Plot",
        },
        button {
            name: "garminconnectoutput",
            id: "garminconnectoutput",
            dangerous_inner_html: "&nbsp;",
        },
        button {
            "type": "submit",
            "onclick": "send_command('filter=latest');",
            "latest",
        },
        button {
            "type": "submit",
            "onclick": "send_command('filter=sport');",
            "sport",
        }
    }
}

fn create_analysis_plot(model: &RaceResultAnalysis, is_demo: bool) -> Element {
    let PlotData {
        data,
        other_data,
        x_proj,
        y_proj,
        x_vals,
        y_nom,
        y_neg,
        y_pos,
        xticks,
        yticks,
        ymin,
        ymax,
    } = model.get_data();

    let model_data = &model.data;
    let summary_map = &model.summary_map;
    let race_type = model.race_type;

    let xlabels = [
        "100m", "", "", "800m", "1mi", "5k", "10k", "Half", "Mar", "", "50mi", "100mi", "300mi",
    ];
    let mut xmap: HashMap<_, _> = xticks.iter().zip(xlabels.iter()).collect();
    xmap.shrink_to_fit();

    let pace_results = x_proj
        .into_iter()
        .zip(y_proj)
        .enumerate()
        .map(move |(idx, (x, y))| {
            let pace = print_h_m_s(y * 60.0, false).unwrap_or_else(|_| "".into());
            let time = print_h_m_s(x * y * 60.0, true).unwrap_or_else(|_| "".into());
            rsx! {
                tr {
                    key: "pace-table-key-{idx}",
                    td {"{x:02}"},
                    td {"{pace}"},
                    td {"{time}"},
                }
            }
        });

    let race_results = model_data
        .iter()
        .sorted_by(|x, y| x.race_date.cmp(&y.race_date))
        .rev()
        .enumerate()
        .map(move |(idx, result)| {
            let distance = f64::from(result.race_distance) / METERS_PER_MILE;
            let time = print_h_m_s(result.race_time, true).unwrap_or_else(|_| "".into());
            let pace =
                print_h_m_s(result.race_time / distance, false).unwrap_or_else(|_| "".into());
            let date = if let Some(date) = result.race_date {
                if is_demo {
                    None
                } else {
                    let filter = result
                        .race_summary_ids
                        .iter()
                        .filter_map(|id| id.and_then(|i| summary_map.get(&i).map(|s| &s.filename)))
                        .join(",");
                    if filter.is_empty() {
                        Some(rsx! {"{date}"})
                    } else {
                        Some(rsx! {
                            button {
                                "type": "submit",
                                "onclick": "send_command('filter={filter},file');",
                                "{date}",
                            }
                        })
                    }
                }
            } else {
                None
            };
            let name: StackString = result
                .race_name
                .as_ref()
                .map_or("", StackString::as_str)
                .into();
            let flag = result.race_flag;
            let flag = if is_demo {
                rsx! {"{flag}"}
            } else {
                let id = result.id;
                rsx! {
                    button {
                        "type": "button",
                        id: "race_flag_{id}",
                        "onclick": "flipRaceResultFlag({id});",
                        "{flag}"
                    }
                }
            };
            rsx! {
                tr {
                    key: "race-results-key-{idx}",
                    td {
                        "align": "right",
                        "{distance:0.1}",
                    },
                    td {"{time}"},
                    td {
                        "align": "center",
                        "{pace}",
                    },
                    td {
                        "align": "center",
                        {date},
                    },
                    td {"{name}"},
                    td { {flag} },
                }
            }
        });

    let x_vals: Vec<f64> = x_vals.map(|x| x * METERS_PER_MILE).to_vec();
    let mut y_nom: Vec<(f64, f64)> = y_nom
        .iter()
        .zip(x_vals.iter())
        .map(|(y, x)| (*x, *y))
        .collect();
    y_nom.shrink_to_fit();
    let mut y_neg: Vec<(f64, f64)> = y_neg
        .iter()
        .zip(x_vals.iter())
        .map(|(y, x)| (*x, *y))
        .collect();
    y_neg.shrink_to_fit();
    let mut y_pos: Vec<(f64, f64)> = y_pos
        .iter()
        .zip(x_vals.iter())
        .map(|(y, x)| (*x, *y))
        .collect();
    y_pos.shrink_to_fit();

    let data = serde_json::to_string(&data).unwrap_or_else(|_| String::new());
    let other_data = serde_json::to_string(&other_data).unwrap_or_else(|_| String::new());
    let xticks = serde_json::to_string(&xticks).unwrap_or_else(|_| String::new());
    let xmap = serde_json::to_string(&xmap).unwrap_or_else(|_| String::new());
    let yticks = serde_json::to_string(&yticks).unwrap_or_else(|_| String::new());
    let fitdata = serde_json::to_string(&y_nom).unwrap_or_else(|_| String::new());
    let negdata = serde_json::to_string(&y_neg).unwrap_or_else(|_| String::new());
    let posdata = serde_json::to_string(&y_pos).unwrap_or_else(|_| String::new());

    let title = match race_type {
        RaceType::Personal => "Race Results",
        RaceType::WorldRecordMen => "Mens World Record",
        RaceType::WorldRecordWomen => "Womens World Record",
    };

    let mut script_body = String::new();
    script_body.push_str("\n!function(){\n");
    writeln!(&mut script_body, "\tlet data = {data};").unwrap();
    writeln!(&mut script_body, "\tlet other_data = {other_data};").unwrap();
    writeln!(&mut script_body, "\tlet xticks = {xticks};").unwrap();
    writeln!(&mut script_body, "\tlet xmap = {xmap};").unwrap();
    writeln!(&mut script_body, "\tlet yticks = {yticks};").unwrap();
    writeln!(&mut script_body, "\tlet fitdata = {fitdata};").unwrap();
    writeln!(&mut script_body, "\tlet negdata = {negdata};").unwrap();
    writeln!(&mut script_body, "\tlet posdata = {posdata};").unwrap();
    writeln!(&mut script_body, "\tscatter_plot_with_lines(").unwrap();
    writeln!(&mut script_body, "\t\tdata, other_data, fitdata,").unwrap();
    writeln!(
        &mut script_body,
        "\t\tnegdata, posdata, xmap, {ymin}, {ymax},"
    )
    .unwrap();
    writeln!(
        &mut script_body,
        "\t\txticks, yticks, '{title}', 'Distance', 'Pace (min/mi)',"
    )
    .unwrap();
    writeln!(&mut script_body, "\t);").unwrap();
    script_body.push_str("}();\n");

    let buttons = rsx! {
        button {
            "type": "submit",
            "onclick": "race_result_plot_personal();",
            "Personal",
        },
        button {
            "type": "submit",
            "onclick": "race_result_plot_world_record_men();",
            "Mens World Records",
        },
        button {
            "type": "submit",
            "onclick": "race_result_plot_world_record_women();",
            "Womens World Records",
        },
    };

    let tables = rsx! {
        table {
            "border": "1",
            thead {
                th {"Distance (mi)"},
                th {"Pace (min/mi)"},
                th {"Time"},
            },
            tbody {
                {pace_results},
            }
        },
        br {
            table {
                "border": "1",
                thead {
                    th {"Distance (mi)"},
                    th {"Time"},
                    th {"Pace (min/mi)"},
                    th {"Date"},
                    th {"Name"},
                    th {"Flag"},
                },
                tbody {
                    {race_results},
                }
            }
        }
    };

    let scripts = rsx! {
        script {src: "/garmin/scripts/scatter_plot_with_lines.js"},
        script {
            dangerous_inner_html: "{script_body}"
        },
    };

    rsx! {
        br {
            {buttons},
        }
        {scripts},
        {tables},
    }
}

/// # Errors
/// Returns error if formatting fails
pub fn table_body(body: StackString) -> Result<String, Error> {
    let mut app = VirtualDom::new_with_props(TableElement, TableElementProps { body });
    app.rebuild_in_place();
    let mut renderer = dioxus_ssr::Renderer::default();
    let mut buffer = String::new();
    renderer
        .render_to(&mut buffer, &app)
        .map_err(Into::<Error>::into)?;
    Ok(buffer)
}

#[component]
fn TableElement(body: StackString) -> Element {
    rsx! {
        textarea {
            cols: "100",
            rows: "40",
            "{body}"
        }
    }
}

/// # Errors
/// Returns error if formatting fails
pub fn strava_body(athlete: StravaAthlete) -> Result<String, Error> {
    let mut app = VirtualDom::new_with_props(StravaElement, StravaElementProps { athlete });
    app.rebuild_in_place();
    let mut renderer = dioxus_ssr::Renderer::default();
    let mut buffer = String::new();
    renderer
        .render_to(&mut buffer, &app)
        .map_err(Into::<Error>::into)?;
    Ok(buffer)
}

#[component]
fn StravaElement(athlete: StravaAthlete) -> Element {
    let id = athlete.id;
    let username = &athlete.username;
    let firstname = &athlete.firstname;
    let lastname = &athlete.lastname;
    let city = &athlete.city;
    let state = &athlete.state;
    let sex = &athlete.sex;
    let weight = athlete.weight * LBS_PER_KG;
    let created_at = athlete.created_at;
    let updated_at = athlete.updated_at;
    let follower_count = athlete.follower_count.map(|follower_count| {
        rsx! {
            td {"Follower Count"},
            td {"{follower_count}"},
        }
    });
    let friend_count = athlete.friend_count.map(|friend_count| {
        rsx! {
            td {"Friend Count"},
            td {"{friend_count}"},
        }
    });
    let measurement_preference =
        athlete
            .measurement_preference
            .as_ref()
            .map(|measurement_preference| {
                rsx! {
                    td {"Measurement Preference"},
                    td {"{measurement_preference}"},
                }
            });
    let clubs = athlete.clubs.as_ref().map(|clubs| {
        let lines = clubs.iter().enumerate().map(|(idx, c)| {
            let id = c.id;
            let name = &c.name;
            let sport_type = &c.sport_type;
            let city = &c.city;
            let state = &c.state;
            let country = &c.country;
            let private = c.private;
            let member_count = c.member_count;
            let url = &c.url;

            rsx! {
                tr {
                    key: "club-key-{idx}",
                    td {"{id}"},
                    td {"{name}"},
                    td {"{sport_type}"},
                    td {"{city}"},
                    td {"{state}"},
                    td {"{country}"},
                    td {"{private}"},
                    td {"{member_count}"},
                    td {"{url}"},
                }
            }
        });
        rsx! {
            br {"Clubs"},
            table {
                "border": "1",
                thead {
                    th {"ID"},
                    th {"Name"},
                    th {"Sport Type"},
                    th {"City"},
                    th {"State"},
                    th {"Country"},
                    th {"Private"},
                    th {"Member Count"},
                    th {"Url"},
                },
                tbody {
                    {lines},
                }
            }
        }
    });
    let shoes = athlete.shoes.as_ref().map(|shoes| {
        let lines = shoes.iter().enumerate().map(|(idx, s)| {
            let id = &s.id;
            let resource_state = s.resource_state;
            let primary = s.primary;
            let name = &s.name;
            let distance = s.distance / METERS_PER_MILE;
            rsx! {
                tr {
                    key: "shoes-key-{idx}",
                    td {"{id}"},
                    td {"{resource_state}"},
                    td {"{primary}"},
                    td {"{name}"},
                    td {"{distance:0.2}"},
                }
            }
        });
        rsx! {
            br {"Shoes"},
            table {
                "border": "1",
                thead {
                    th {"ID"},
                    th {"Resource State"},
                    th {"Primary"},
                    th {"Name"},
                    th {"Distance (mi)"},
                },
                tbody {
                    {lines},
                }
            }
        }
    });

    rsx! {
        table {
            "border": "1",
            tbody {
                tr {td {"ID"}, td {"{id}"}},
                tr {td {"Username"}, td {"{username}"}},
                tr {td {"First Name"}, td {"{firstname}"}},
                tr {td {"Last Name"}, td {"{lastname}"}},
                tr {td {"City"}, td {"{city}"}},
                tr {td {"State"}, td {"{state}"}},
                tr {td {"Sex"}, td {"{sex}"}},
                tr {td {"Weight"}, td {"{weight} lbs"}},
                tr {td {"Created At"}, td {"{created_at}"}},
                tr {td {"Updated At"}, td {"{updated_at}"}},
                tr { {follower_count} },
                tr { {friend_count} },
                tr { {measurement_preference} },
            },
            {clubs},
            {shoes},
        }
    }
}

/// # Errors
/// Returns error if formatting fails
pub fn scale_measurement_manual_input_body() -> Result<String, Error> {
    let mut app = VirtualDom::new(scale_measurement_manual_input_element);
    app.rebuild_in_place();
    let mut renderer = dioxus_ssr::Renderer::default();
    let mut buffer = String::new();
    renderer
        .render_to(&mut buffer, &app)
        .map_err(Into::<Error>::into)?;
    Ok(buffer)
}

fn scale_measurement_manual_input_element() -> Element {
    rsx! {
        form {
            table {
                tbody {
                    tr {
                        td {"Weight (lbs)"}
                        td {
                            input {
                                "type": "text",
                                name: "weight_in_lbs",
                                id: "weight_in_lbs",
                            }
                        }
                    }
                    tr {
                        td {"Body Fat %"}
                        td {
                            input {
                                "type": "text",
                                name: "body_fat_percent",
                                id: "body_fat_percent",
                            }
                        }
                    }
                    tr {
                        td {"Muscle Mass (lbs)"}
                        td {
                            input {
                                "type": "text",
                                name: "muscle_mass_lbs",
                                id: "muscle_mass_lbs",
                            }
                        }
                    }
                    tr {
                        td {"Body Water %"}
                        td {
                            input {
                                "type": "text",
                                name: "body_water_percent",
                                id: "body_water_percent",
                            }
                        }
                    }
                    tr {
                        td {"Bone Mass (lbs)"}
                        td {
                            input {
                                "type": "text",
                                name: "bone_mass_lbs",
                                id: "bone_mass_lbs",
                            }
                        }
                    }
                    tr {
                        td {
                            input {
                                "type": "button",
                                name: "scale_measurement_manual_input",
                                value: "Submit",
                                "onclick": "scaleMeasurementManualInput();",
                            }
                        }
                    }
                }
            }
        }
    }
}

/// # Errors
/// Returns error if formatting fails
pub fn garmin_connect_profile_body(profile: GarminConnectSocialProfile) -> Result<String, Error> {
    let mut app = VirtualDom::new_with_props(
        GarminConnectProfileElement,
        GarminConnectProfileElementProps { profile },
    );
    app.rebuild_in_place();
    let mut renderer = dioxus_ssr::Renderer::default();
    let mut buffer = String::new();
    renderer
        .render_to(&mut buffer, &app)
        .map_err(Into::<Error>::into)?;
    Ok(buffer)
}

#[component]
fn GarminConnectProfileElement(profile: GarminConnectSocialProfile) -> Element {
    let id = profile.id;
    let display_name = &profile.display_name;
    let profile_id = profile.profile_id;
    let garmin_guid = profile.garmin_guid;
    let full_name = &profile.full_name;
    let username = &profile.username;
    let location = &profile.location;

    rsx! {
        table {
            "border": "1",
            tbody {
                tr { td  {"ID"}, td {"{id}"}},
                tr { td  {"Display Name"}, td {"{display_name}"}},
                tr { td  {"Profile ID"}, td {"{profile_id}"}},
                tr { td  {"Garmin GUID"}, td {"{garmin_guid}"}},
                tr { td  {"Full Name"}, td {"{full_name}"}},
                tr { td  {"Username"}, td {"{username}"}},
                tr { td  {"Location"}, td {"{location}"}},
            }
        }
    }
}

#[macro_use]
extern crate serde_derive;
extern crate actix;
extern crate actix_web;

use actix_web::{http::Method, http::StatusCode, server, App, HttpResponse, Json, Query};

use failure::Error;

use garmin_rust::common::garmin_cli::GarminCli;
use garmin_rust::common::garmin_config::GarminConfig;
use garmin_rust::common::garmin_correction_lap::GarminCorrectionList;
use garmin_rust::common::garmin_file;
use garmin_rust::parsers::garmin_parse;
use garmin_rust::reports::garmin_file_report_txt;
use garmin_rust::utils::garmin_util::{get_list_of_files_from_db, get_pg_conn};

#[derive(Debug, Deserialize)]
struct FilterRequest {
    filter: Option<String>,
    history: Option<String>,
}

fn garmin(request: Query<FilterRequest>) -> Result<HttpResponse, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());
    let history = request.history.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (options, constraints) = GarminCli::process_pattern(&filter_vec);

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(GarminCli::new().with_config().run_html(
            &options,
            &constraints,
            &filter,
            &history,
        )?);
    Ok(resp)
}

#[derive(Serialize)]
struct GpsList {
    gps_list: Vec<String>,
}

#[derive(Serialize)]
struct TimeValue {
    time: String,
    value: f64,
}

#[derive(Serialize)]
struct HrData {
    hr_data: Vec<TimeValue>,
}

#[derive(Serialize)]
struct HrPace {
    hr: f64,
    pace: f64,
}

#[derive(Serialize)]
struct HrPaceList {
    hr_pace: Vec<HrPace>,
}

fn garmin_list_gps_tracks(request: Query<FilterRequest>) -> Result<Json<GpsList>, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (_, constraints) = GarminCli::process_pattern(&filter_vec);

    let config = GarminCli::new().with_config().config;

    let pg_conn = get_pg_conn(&config.pgurl)?;

    Ok(Json(GpsList {
        gps_list: get_list_of_files_from_db(&pg_conn, &constraints)?,
    }))
}

fn garmin_get_hr_data(request: Query<FilterRequest>) -> Result<Json<HrData>, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (_, constraints) = GarminCli::process_pattern(&filter_vec);

    let config = GarminCli::new().with_config().config;

    let pg_conn = get_pg_conn(&config.pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

    match file_list.len() {
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    garmin_parse::GarminParse::new(&gps_file, &corr_map).gfile
                }
            };

            Ok(Json(HrData {
                hr_data: gfile
                    .points
                    .iter()
                    .filter_map(|p| match p.heart_rate {
                        Some(hr) => Some(TimeValue {
                            time: p.time.clone(),
                            value: hr,
                        }),
                        None => None,
                    })
                    .collect(),
            }))
        }
        _ => Ok(Json(HrData {
            hr_data: Vec::new(),
        })),
    }
}

fn garmin_get_hr_pace(request: Query<FilterRequest>) -> Result<Json<HrPaceList>, Error> {
    let request = request.into_inner();

    let filter = request.filter.unwrap_or_else(|| "sport".to_string());

    let filter_vec: Vec<String> = filter.split(',').map(|x| x.to_string()).collect();

    let (_, constraints) = GarminCli::process_pattern(&filter_vec);

    let config = GarminCli::new().with_config().config;

    let pg_conn = get_pg_conn(&config.pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

    match file_list.len() {
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            let avro_file = format!("{}/{}.avro", &config.cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => g,
                Err(_) => {
                    let gps_file = format!("{}/{}", &config.gps_dir, file_name);

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    garmin_parse::GarminParse::new(&gps_file, &corr_map).gfile
                }
            };

            let splits = garmin_file_report_txt::get_splits(&gfile, 400., "mi", true)?;

            Ok(Json(HrPaceList {
                hr_pace: splits
                    .iter()
                    .filter_map(|v| {
                        let s = v[1];
                        let h = v[2];
                        let pace = 4. * s / 60.;
                        if pace >= 5.5 && pace <= 20. {
                            Some(HrPace { hr: h, pace })
                        } else {
                            None
                        }
                    })
                    .collect(),
            }))
        }
        _ => Ok(Json(HrPaceList {
            hr_pace: Vec::new(),
        })),
    }
}

fn main() {
    let sys = actix::System::new("garmin");
    let config = GarminConfig::get_config();

    server::new(|| {
        App::new()
            .resource("/garmin", |r| r.method(Method::GET).with(garmin))
            .resource("/garmin/list_gps_tracks", |r| {
                r.method(Method::GET).with(garmin_list_gps_tracks)
            })
            .resource("/garmin/get_hr_data", |r| {
                r.method(Method::GET).with(garmin_get_hr_data)
            })
            .resource("/garmin/get_hr_pace", |r| {
                r.method(Method::GET).with(garmin_get_hr_pace)
            })
    })
    .bind(&format!("127.0.0.1:{}", config.port))
    .unwrap()
    .start();

    let _ = sys.run();
}

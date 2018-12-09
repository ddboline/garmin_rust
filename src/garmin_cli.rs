extern crate config;
extern crate tempdir;

use clap::{App, Arg};
use failure::Error;
use tempdir::TempDir;

use crate::garmin_correction_lap;
use crate::garmin_file;
use crate::garmin_summary;
use crate::garmin_sync;
use crate::parsers::garmin_parse;
use crate::reports::garmin_file_report_html::file_report_html;
use crate::reports::garmin_file_report_txt::generate_txt_report;
use crate::reports::garmin_report_options::GarminReportOptions;
use crate::reports::garmin_summary_report_html::summary_report_html;
use crate::reports::garmin_summary_report_txt::{create_report_query, get_list_of_files_from_db};
use crate::utils::sport_types::get_sport_type_map;

fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

pub fn cli_garmin_proc() {
    let home_dir = env!("HOME");

    let settings = config::Config::new()
        .merge(config::File::with_name("config.yml"))
        .unwrap()
        .clone();

    let pg_url = settings.get_str("PGURL").unwrap();
    let gps_bucket = settings.get_str("GPS_BUCKET").unwrap();
    let cache_bucket = settings.get_str("CACHE_BUCKET").unwrap();

    let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
    let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);

    let gps_dir = settings.get_str("GPS_DIR").unwrap_or(default_gps_dir);
    let cache_dir = settings.get_str("CACHE_DIR").unwrap_or(default_cache_dir);

    let matches = App::new("Garmin Rust Proc")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Convert GPS files to avro format, dump stuff to postgres")
        .arg(
            Arg::with_name("filename")
                .short("f")
                .long("filename")
                .value_name("FILENAME")
                .multiple(true)
                .help("Convert a single file"),
        )
        .arg(
            Arg::with_name("all")
                .short("a")
                .long("all")
                .value_name("ALL")
                .takes_value(false)
                .help("Convert all files in gps dir"),
        )
        .arg(
            Arg::with_name("sync")
                .short("s")
                .long("sync")
                .value_name("SYNC")
                .help("Sync gps_files and cache with s3")
                .takes_value(false),
        )
        .get_matches();

    let filenames = matches.values_of("filename");

    let do_sync = matches.is_present("sync");
    let do_all = matches.is_present("all");

    match do_sync {
        true => {
            let s3_client = garmin_sync::get_s3_client();
            garmin_sync::sync_dir(
                format!("{}/.garmin_cache/run/gps_tracks", home_dir).as_str(),
                &gps_bucket,
                &s3_client,
            )
            .unwrap();
            garmin_sync::sync_dir(
                format!("{}/.garmin_cache/run/cache", home_dir).as_str(),
                &cache_bucket,
                &s3_client,
            )
            .unwrap();
        }
        false => {
            let corr_list = garmin_correction_lap::read_corrections_from_db(&pg_url).unwrap();

            garmin_correction_lap::dump_corr_list_to_avro(
                &corr_list,
                &format!("{}/garmin_correction.avro", &cache_dir),
            )
            .unwrap();

            let corr_map = garmin_correction_lap::get_corr_list_map(&corr_list);

            let gsum_list = match filenames {
                Some(flist) => flist
                    .map(|f| {
                        println!("{}", &f);
                        garmin_summary::process_single_gps_file(&f, &cache_dir, &corr_map).unwrap()
                    })
                    .collect(),
                None => match do_all {
                    true => garmin_summary::process_all_gps_files(&gps_dir, &cache_dir, &corr_map)
                        .unwrap(),
                    false => Vec::new(),
                },
            };

            if gsum_list.len() > 0 {
                garmin_summary::write_summary_to_postgres(&pg_url, &gsum_list)
            };
        }
    }
}

pub fn cli_garmin_report() {
    let matches = App::new("Garmin Rust Report")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Convert GPS files to avro format, dump stuff to postgres")
        .arg(Arg::with_name("patterns").multiple(true))
        .get_matches();

    let (options, constraints) = match matches.values_of("patterns") {
        Some(patterns) => {
            let strings: Vec<String> = patterns.map(|x| x.to_string()).collect();
            process_pattern(&strings)
        }
        None => {
            let default_patterns = vec!["year".to_string()];
            process_pattern(&default_patterns)
        }
    };

    run_cli(&options, &constraints).unwrap()
}

pub fn process_pattern(patterns: &Vec<String>) -> (GarminReportOptions, Vec<String>) {
    let mut options = GarminReportOptions::new();

    let sport_type_map = get_sport_type_map();

    let mut constraints: Vec<String> = Vec::new();

    for pattern in patterns {
        match pattern.as_str() {
            "year" => options.do_year = true,
            "month" => options.do_month = true,
            "week" => options.do_week = true,
            "day" => options.do_day = true,
            "file" => options.do_file = true,
            pat => match sport_type_map.get(pat) {
                Some(&x) => options.do_sport = Some(x),
                None => {
                    constraints.push(format!("begin_datetime like '%{}%'", pat));
                    constraints.push(format!("filename like '%{}%'", pat));
                }
            },
        };
    }

    (options, constraints)
}

pub fn run_cli(options: &GarminReportOptions, constraints: &Vec<String>) -> Result<(), Error> {
    let home_dir = env!("HOME");

    let settings = config::Config::new()
        .merge(config::File::with_name("config.yml"))
        .unwrap()
        .clone();

    let pg_url = settings.get_str("PGURL").unwrap();

    let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
    let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);

    let gps_dir = settings.get_str("GPS_DIR").unwrap_or(default_gps_dir);
    let cache_dir = settings.get_str("CACHE_DIR").unwrap_or(default_cache_dir);

    let file_list = get_list_of_files_from_db(&pg_url, &constraints).unwrap();

    match file_list.len() {
        0 => (),
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            debug!("{}", &file_name);
            let avro_file = format!("{}/{}.avro", cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => {
                    debug!("Cached avro file read: {}", &avro_file);
                    g
                }
                Err(_) => {
                    let gps_file = format!("{}/{}", gps_dir, file_name);

                    let corr_list =
                        garmin_correction_lap::read_corrections_from_db(&pg_url).unwrap();
                    let corr_map = garmin_correction_lap::get_corr_list_map(&corr_list);

                    debug!("Reading gps_file: {}", &gps_file);
                    garmin_parse::GarminParse::new(&gps_file, &corr_map)
                }
            };
            debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
            println!("{}", generate_txt_report(&gfile).join("\n"));
        }
        _ => {
            debug!("{:?}", options);
            let txt_result = create_report_query(&pg_url, &options, &constraints);

            println!("{}", txt_result.join("\n"));
        }
    };
    Ok(())
}

pub fn run_html(options: &GarminReportOptions, constraints: &Vec<String>) -> Result<String, Error> {
    let home_dir = env!("HOME");

    let settings = config::Config::new()
        .merge(config::File::with_name("config.yml"))
        .unwrap()
        .clone();

    let pg_url = settings.get_str("PGURL").unwrap();
    let maps_api_key = settings.get_str("MAPS_API_KEY").unwrap();

    let default_gps_dir = format!("{}/.garmin_cache/run/gps_tracks", home_dir);
    let default_cache_dir = format!("{}/.garmin_cache/run/cache", home_dir);

    let gps_dir = settings.get_str("GPS_DIR").unwrap_or(default_gps_dir);
    let cache_dir = settings.get_str("CACHE_DIR").unwrap_or(default_cache_dir);

    let http_bucket = settings.get_str("HTTP_BUCKET").unwrap();

    let file_list = get_list_of_files_from_db(&pg_url, &constraints).unwrap();

    match file_list.len() {
        0 => Ok("".to_string()),
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            debug!("{}", &file_name);
            let avro_file = format!("{}/{}.avro", cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => {
                    debug!("Cached avro file read: {}", &avro_file);
                    g
                }
                Err(_) => {
                    let gps_file = format!("{}/{}", gps_dir, file_name);

                    let corr_list =
                        garmin_correction_lap::read_corrections_from_db(&pg_url).unwrap();
                    let corr_map = garmin_correction_lap::get_corr_list_map(&corr_list);

                    debug!("Reading gps_file: {}", &gps_file);
                    garmin_parse::GarminParse::new(&gps_file, &corr_map)
                }
            };
            debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());

            let tempdir = TempDir::new("garmin_html").unwrap();
            let htmlcachedir = tempdir.path().to_str().unwrap();

            file_report_html(&gfile, &maps_api_key, &htmlcachedir, &http_bucket)
        }
        _ => {
            debug!("{:?}", options);
            let txt_result = create_report_query(&pg_url, &options, &constraints);

            let tempdir = TempDir::new("garmin_html").unwrap();
            let htmlcachedir = tempdir.path().to_str().unwrap();

            summary_report_html(&txt_result, &options, &htmlcachedir)
        }
    }
}

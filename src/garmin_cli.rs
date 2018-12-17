extern crate config;
extern crate rayon;
extern crate tempdir;

use clap::{App, Arg};
use failure::Error;
use rayon::prelude::*;
use std::collections::HashSet;
use std::env::var;
use std::path::Path;
use tempdir::TempDir;

use crate::garmin_config::GarminConfig;
use crate::garmin_correction_lap::GarminCorrectionList;
use crate::garmin_file;
use crate::garmin_summary::{GarminSummary, GarminSummaryList};
use crate::garmin_sync::GarminSync;
use crate::parsers::garmin_parse;
use crate::reports::garmin_file_report_html::file_report_html;
use crate::reports::garmin_file_report_txt::generate_txt_report;
use crate::reports::garmin_report_options::GarminReportOptions;
use crate::reports::garmin_summary_report_html::summary_report_html;
use crate::reports::garmin_summary_report_txt::create_report_query;
use crate::utils::garmin_util::{
    get_file_list, get_list_of_files_from_db, get_pg_conn, map_result_vec,
};
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

pub fn get_garmin_config() -> GarminConfig {
    let home_dir = var("HOME").expect("No HOME directory...");

    GarminConfig::new()
        .from_yml(&format!("{}/.config/garmin_rust/config.yml", home_dir))
        .from_yml("config.yml")
        .from_env()
}

pub fn cli_garmin_proc() -> Result<(), Error> {
    let config = get_garmin_config();

    let pgurl = config.pgurl.expect("No Postgres server specified (PGURL)");
    let cache_dir = config.cache_dir;
    let gps_dir = config.gps_dir;
    let gps_bucket = config.gps_bucket.expect("No GPS_BUCKET specified");
    let cache_bucket = config.cache_bucket.expect("No CACHE_BUCKET specified");
    let summary_cache = config.summary_cache.expect("No SUMMARY_CACHE specified");
    let summary_bucket = config.summary_bucket.expect("No SUMMARY_BUCKET specified");

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
        .arg(
            Arg::with_name("bootstrap")
                .short("b")
                .long("bootstrap")
                .value_name("BOOTSTRAP")
                .help("Bootstrap a new node")
                .takes_value(false),
        )
        .get_matches();

    let filenames = matches.values_of("filename");

    let do_sync = matches.is_present("sync");
    let do_all = matches.is_present("all");
    let do_bootstrap = matches.is_present("bootstrap");

    let pg_conn = get_pg_conn(&pgurl)?;

    if do_bootstrap {
        let gsync = GarminSync::new();
        println!("Syncing GPS files");
        gsync.sync_dir(&gps_dir, &gps_bucket)?;
        println!("Syncing CACHE files");
        gsync.sync_dir(&cache_dir, &cache_bucket)?;
        println!("Syncing SUMMARY files");
        gsync.sync_dir(&summary_cache, &summary_bucket)?;

        println!("Read corrections from avro file");
        let corr_list = GarminCorrectionList::read_corr_list_from_avro(&format!(
            "{}/garmin_correction.avro",
            &cache_dir
        ))?;

        println!("Write corrections to postgres");
        corr_list.dump_corrections_to_db(&pg_conn)?;

        println!("Read summaries from avro files");
        let gsum_list = GarminSummaryList::read_summary_from_avro_files(&summary_cache)?;

        println!("Write summaries to postgres");
        gsum_list.write_summary_to_postgres(&pg_conn)?;
    } else if do_sync {
        let gsync = GarminSync::new();

        println!("Syncing GPS files");
        gsync.sync_dir(&gps_dir, &gps_bucket)?;

        println!("Syncing CACHE files");
        gsync.sync_dir(&cache_dir, &cache_bucket)?;

        println!("Syncing SUMMARY file");
        gsync.sync_dir(&summary_cache, &summary_bucket)?;
    } else {
        let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;

        let corr_map = corr_list.get_corr_list_map();

        let gsum_list = match filenames {
            Some(flist) => {
                let proc_list: Vec<Result<_, Error>> = flist
                    .map(|f| {
                        println!("Process {}", &f);
                        Ok(GarminSummary::process_single_gps_file(
                            &f, &cache_dir, &corr_map,
                        )?)
                    })
                    .collect();
                GarminSummaryList::from_vec(map_result_vec(proc_list)?)
            }
            None => {
                if do_all {
                    GarminSummaryList::process_all_gps_files(&gps_dir, &cache_dir, &corr_map)?
                } else {
                    let path = Path::new(&cache_dir);
                    let cacheset: HashSet<String> = get_file_list(&path)
                        .into_par_iter()
                        .filter_map(|f| {
                            if f.contains("garmin_correction.avro") {
                                None
                            } else {
                                Some(f.split('/').last().unwrap().to_string())
                            }
                        })
                        .collect();

                    let dbset: HashSet<String> = get_list_of_files_from_db(&pg_conn, &Vec::new())?
                        .into_iter()
                        .collect();

                    let path = Path::new(&gps_dir);
                    let proc_list: Vec<Result<_, Error>> = get_file_list(&path)
                        .into_par_iter()
                        .map(|f| f.split('/').last().unwrap().to_string())
                        .filter_map(|f| {
                            let cachefile = format!("{}.avro", f);
                            if dbset.contains(&f) && cacheset.contains(&cachefile) {
                                None
                            } else {
                                let gps_path = format!("{}/{}", &gps_dir, &f);
                                println!("Process {}", &gps_path);
                                Some(gps_path)
                            }
                        })
                        .map(|f| {
                            Ok(GarminSummary::process_single_gps_file(
                                &f, &cache_dir, &corr_map,
                            )?)
                        })
                        .collect();
                    GarminSummaryList::from_vec(map_result_vec(proc_list)?)
                }
            }
        };

        if !gsum_list.summary_list.is_empty() {
            gsum_list.write_summary_to_avro_files(&summary_cache)?;
            gsum_list.write_summary_to_postgres(&pg_conn)?;
            corr_list.dump_corr_list_to_avro(&format!("{}/garmin_correction.avro", &cache_dir))?;
        };
    };
    Ok(())
}

pub fn cli_garmin_report() -> Result<(), Error> {
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

    run_cli(&options, &constraints)
}

pub fn process_pattern(patterns: &[String]) -> (GarminReportOptions, Vec<String>) {
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
            "sport" => options.do_all_sports = true,
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

pub fn run_cli(options: &GarminReportOptions, constraints: &[String]) -> Result<(), Error> {
    let config = get_garmin_config();

    let pgurl = config.pgurl.expect("No Postgres server specified (PGURL)");
    let cache_dir = config.cache_dir;
    let gps_dir = config.gps_dir;

    let pg_conn = get_pg_conn(&pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

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

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    debug!("Reading gps_file: {}", &gps_file);
                    garmin_parse::GarminParse::new(&gps_file, &corr_map).gfile
                }
            };
            debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
            println!("{}", generate_txt_report(&gfile)?.join("\n"));
        }
        _ => {
            debug!("{:?}", options);
            let txt_result = create_report_query(&pg_conn, &options, &constraints)?;

            println!("{}", txt_result.join("\n"));
        }
    };
    Ok(())
}

pub fn run_html(
    options: &GarminReportOptions,
    constraints: &[String],
    filter: &str,
    history: &str,
) -> Result<String, Error> {
    let config = get_garmin_config();

    let pgurl = config.pgurl.expect("No Postgres server specified (PGURL)");
    let gps_dir = config.gps_dir;

    let http_bucket = config.http_bucket.expect("No HTTP_BUCKET specified");

    let pg_conn = get_pg_conn(&pgurl)?;

    let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

    match file_list.len() {
        0 => Ok("".to_string()),
        1 => {
            let file_name = file_list.get(0).expect("This shouldn't be happening...");
            debug!("{}", &file_name);
            let avro_file = format!("{}/{}.avro", config.cache_dir, file_name);
            let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                Ok(g) => {
                    debug!("Cached avro file read: {}", &avro_file);
                    g
                }
                Err(_) => {
                    let gps_file = format!("{}/{}", gps_dir, file_name);

                    let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                    let corr_map = corr_list.get_corr_list_map();

                    debug!("Reading gps_file: {}", &gps_file);
                    garmin_parse::GarminParse::new(&gps_file, &corr_map).gfile
                }
            };
            debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());

            let tempdir = TempDir::new("garmin_html")?;
            let htmlcachedir = tempdir
                .path()
                .to_str()
                .expect("Path is invalid unicode somehow");

            file_report_html(
                &gfile,
                &config.maps_api_key.unwrap_or_else(|| "EMPTY".to_string()),
                &htmlcachedir,
                &http_bucket,
                &history,
            )
        }
        _ => {
            debug!("{:?}", options);
            let txt_result = create_report_query(&pg_conn, &options, &constraints)?;

            let tempdir = TempDir::new("garmin_html")?;
            let htmlcachedir = tempdir
                .path()
                .to_str()
                .expect("Path is invalid unicode somehow");

            summary_report_html(&txt_result, &options, &htmlcachedir, &filter, &history)
        }
    }
}

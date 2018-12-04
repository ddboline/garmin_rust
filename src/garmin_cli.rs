use clap::{App, Arg};

use crate::garmin_correction_lap;
use crate::garmin_file;
use crate::garmin_parse;
use crate::garmin_report;
use crate::garmin_summary;
use crate::garmin_sync;
use crate::garmin_util;

pub fn cli_garmin_proc() {
    let matches = App::new("Garmin Rust Proc")
        .version("0.1")
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
            Arg::with_name("gps_dir")
                .short("d")
                .long("gps_dir")
                .value_name("GPS_DIR")
                .help("Convert all files in a directory")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("cache_dir")
                .short("c")
                .long("cache_dir")
                .value_name("CACHE_DIR")
                .help("Specify cache directory")
                .takes_value(true),
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
    let gps_dir = matches
        .value_of("gps_dir")
        .unwrap_or("/home/ddboline/.garmin_cache/run/gps_tracks");
    let cache_dir = matches
        .value_of("cache_dir")
        .unwrap_or("/home/ddboline/.garmin_cache/run/cache");

    let do_sync = matches.is_present("sync");
    let do_all = matches.is_present("all");

    match do_sync {
        true => {
            garmin_sync::sync_file(
                "/home/ddboline/.garmin_cache/run/gps_tracks",
                "garmin_scripts_gps_files_ddboline",
            );
            garmin_sync::sync_file(
                "/home/ddboline/.garmin_cache/run/cache",
                "garmin-scripts-cache-ddboline",
            );
        }
        false => {
            let corr_list = garmin_correction_lap::read_corrections_from_db().unwrap();
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
                garmin_summary::write_summary_to_postgres(&gsum_list)
            };
        }
    }
}

pub fn cli_garmin_report() {
    let matches = App::new("Garmin Rust Report")
        .version("0.1")
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Convert GPS files to avro format, dump stuff to postgres")
        .arg(
            Arg::with_name("gps_dir")
                .short("d")
                .long("gps_dir")
                .value_name("GPS_DIR")
                .help("Convert all files in a directory")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("cache_dir")
                .short("c")
                .long("cache_dir")
                .value_name("CACHE_DIR")
                .help("Specify cache directory")
                .takes_value(true),
        )
        .arg(Arg::with_name("patterns").multiple(true))
        .get_matches();

    let gps_dir = matches
        .value_of("gps_dir")
        .unwrap_or("/home/ddboline/.garmin_cache/run/gps_tracks");
    let cache_dir = matches
        .value_of("cache_dir")
        .unwrap_or("/home/ddboline/.garmin_cache/run/cache");

    let patterns = matches.values_of("patterns");

    let mut options = garmin_report::GarminReportOptions::new();

    let sport_type_map = garmin_util::get_sport_type_map();

    let mut constraints: Vec<String> = Vec::new();

    match patterns {
        Some(p) => {
            for pattern in p {
                match pattern {
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
        }
        None => (),
    };

    let file_list = garmin_report::get_list_of_files_from_db(&constraints).unwrap();

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

                    let corr_list = garmin_correction_lap::read_corrections_from_db().unwrap();
                    let corr_map = garmin_correction_lap::get_corr_list_map(&corr_list);

                    debug!("Reading gps_file: {}", &gps_file);
                    garmin_parse::GarminParse::new(&gps_file, &corr_map)
                }
            };
            debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
            println!("{}", garmin_report::generate_txt_report(&gfile).join("\n"));
            garmin_report::file_report_html(&gfile).expect("Failed to generate html report");
        }
        _ => {
            debug!("{:?}", options);
            println!(
                "{}",
                garmin_report::create_report_query(&options, &constraints)
            );
        }
    };
}

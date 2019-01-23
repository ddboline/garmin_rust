extern crate config;
extern crate rayon;
extern crate tempdir;

use clap::{App, Arg};
use failure::Error;
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use tempdir::TempDir;

use super::garmin_config::GarminConfig;
use super::garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionList};
use super::garmin_file;
use super::garmin_summary::{GarminSummary, GarminSummaryList};
use super::garmin_sync::GarminSync;
use super::garmin_sync::GarminSyncTrait;
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

#[derive(Debug, Default)]
pub struct GarminCli {
    pub config: GarminConfig,
    pub do_sync: bool,
    pub do_all: bool,
    pub do_bootstrap: bool,
    pub filenames: Option<Vec<String>>,
}

impl GarminCli {
    pub fn new() -> GarminCli {
        GarminCli {
            config: GarminConfig::new(),
            do_sync: false,
            do_all: false,
            do_bootstrap: false,
            filenames: None,
        }
    }

    pub fn with_config(mut self) -> GarminCli {
        self.config = GarminConfig::get_config(None);
        self
    }

    pub fn with_cli_proc(mut self) -> GarminCli {
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

        self.filenames = matches
            .values_of("filename")
            .map(|f| f.map(|f| f.to_string()).collect());

        self.do_sync = matches.is_present("sync");
        self.do_all = matches.is_present("all");
        self.do_bootstrap = matches.is_present("bootstrap");
        self
    }

    pub fn garmin_proc(&self) -> Result<(), Error> {
        let pg_conn = get_pg_conn(&self.config.pgurl)?;

        if self.do_bootstrap {
            self.run_bootstrap()?;
        } else if self.do_sync {
            let gsync = GarminSync::new();

            println!("Syncing GPS files");
            gsync.sync_dir(&self.config.gps_dir, &self.config.gps_bucket)?;

            println!("Syncing CACHE files");
            gsync.sync_dir(&self.config.cache_dir, &self.config.cache_bucket)?;

            println!("Syncing SUMMARY file");
            gsync.sync_dir(&self.config.summary_cache, &self.config.summary_bucket)?;
        } else {
            let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
            let corr_map = corr_list.get_corr_list_map();

            let gsum_list = self.get_summary_list(&corr_map)?;

            if !gsum_list.summary_list.is_empty() {
                gsum_list.write_summary_to_avro_files(&self.config.summary_cache)?;
                gsum_list.write_summary_to_postgres(&pg_conn)?;
                corr_list.dump_corr_list_to_avro(&format!(
                    "{}/garmin_correction.avro",
                    &self.config.cache_dir
                ))?;
            };
        };
        Ok(())
    }

    pub fn get_summary_list(
        &self,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let pg_conn = get_pg_conn(&self.config.pgurl)?;

        let gsum_list = match &self.filenames {
            Some(flist) => {
                let proc_list: Vec<Result<_, Error>> = flist
                    .iter()
                    .map(|f| {
                        println!("Process {}", &f);
                        Ok(GarminSummary::process_single_gps_file(
                            &f,
                            &self.config.cache_dir,
                            &corr_map,
                        )?)
                    })
                    .collect();
                GarminSummaryList::from_vec(map_result_vec(proc_list)?)
            }
            None => {
                if self.do_all {
                    GarminSummaryList::process_all_gps_files(
                        &self.config.gps_dir,
                        &self.config.cache_dir,
                        &corr_map,
                    )?
                } else {
                    let path = Path::new(&self.config.cache_dir);
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

                    let path = Path::new(&self.config.gps_dir);
                    let proc_list: Vec<Result<_, Error>> = get_file_list(&path)
                        .into_par_iter()
                        .map(|f| f.split('/').last().unwrap().to_string())
                        .filter_map(|f| {
                            let cachefile = format!("{}.avro", f);
                            if dbset.contains(&f) && cacheset.contains(&cachefile) {
                                None
                            } else {
                                let gps_path = format!("{}/{}", &self.config.gps_dir, &f);
                                println!("Process {}", &gps_path);
                                Some(gps_path)
                            }
                        })
                        .map(|f| {
                            Ok(GarminSummary::process_single_gps_file(
                                &f,
                                &self.config.cache_dir,
                                &corr_map,
                            )?)
                        })
                        .collect();
                    GarminSummaryList::from_vec(map_result_vec(proc_list)?)
                }
            }
        };
        Ok(gsum_list)
    }

    pub fn run_bootstrap(&self) -> Result<(), Error> {
        let pg_conn = get_pg_conn(&self.config.pgurl)?;

        let gsync = GarminSync::new();
        println!("Syncing GPS files");
        gsync.sync_dir(&self.config.gps_dir, &self.config.gps_bucket)?;
        println!("Syncing CACHE files");
        gsync.sync_dir(&self.config.cache_dir, &self.config.cache_bucket)?;
        println!("Syncing SUMMARY files");
        gsync.sync_dir(&self.config.summary_cache, &self.config.summary_bucket)?;

        println!("Read corrections from avro file");
        let corr_list = GarminCorrectionList::read_corr_list_from_avro(&format!(
            "{}/garmin_correction.avro",
            &self.config.cache_dir
        ))?;

        println!("Write corrections to postgres");
        corr_list.dump_corrections_to_db(&pg_conn)?;

        println!("Read summaries from avro files");
        let gsum_list =
            GarminSummaryList::read_summary_from_avro_files(&self.config.summary_cache)?;

        println!("Write summaries to postgres");
        gsum_list.write_summary_to_postgres(&pg_conn)?;
        Ok(())
    }

    pub fn cli_garmin_report(&self) -> Result<(), Error> {
        let matches = App::new("Garmin Rust Report")
            .version(get_version_number().as_str())
            .author("Daniel Boline <ddboline@gmail.com>")
            .about("Convert GPS files to avro format, dump stuff to postgres")
            .arg(Arg::with_name("patterns").multiple(true))
            .get_matches();

        let (options, constraints) = match matches.values_of("patterns") {
            Some(patterns) => {
                let strings: Vec<String> = patterns.map(|x| x.to_string()).collect();
                GarminCli::process_pattern(&strings)
            }
            None => {
                let default_patterns = vec!["year".to_string()];
                GarminCli::process_pattern(&default_patterns)
            }
        };

        self.run_cli(&options, &constraints)
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
                        if pat.contains('w') {
                            let vals: Vec<_> = pat.split('w').collect();
                            if let Ok(year) = vals[0].parse::<i32>() {
                                if let Ok(week) = vals[1].parse::<i32>() {
                                    constraints.push(format!(
                                        "(EXTRACT(isoyear from cast(begin_datetime as timestamp with time zone) at time zone 'EST') = {} AND
                                        EXTRACT(week from cast(begin_datetime as timestamp with time zone) at time zone 'EST') = {})", year, week));
                                }
                            }
                        } else {
                            constraints.push(format!("begin_datetime like '%{}%'", pat));
                        }
                        constraints.push(format!("filename like '%{}%'", pat));
                    }
                },
            };
        }

        (options, constraints)
    }

    pub fn run_cli(
        &self,
        options: &GarminReportOptions,
        constraints: &[String],
    ) -> Result<(), Error> {
        let pg_conn = get_pg_conn(&self.config.pgurl)?;

        let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

        match file_list.len() {
            0 => (),
            1 => {
                let file_name = file_list.get(0).expect("This shouldn't be happening...");
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", &self.config.cache_dir, file_name);
                let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                    Ok(g) => {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    }
                    Err(_) => {
                        let gps_file = format!("{}/{}", &self.config.gps_dir, file_name);

                        let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                        let corr_map = corr_list.get_corr_list_map();

                        debug!("Reading gps_file: {}", &gps_file);
                        garmin_parse::GarminParse::new()
                            .with_file(&gps_file, &corr_map)
                            .gfile
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
        &self,
        options: &GarminReportOptions,
        constraints: &[String],
        filter: &str,
        history: &str,
    ) -> Result<String, Error> {
        let pg_conn = get_pg_conn(&self.config.pgurl)?;

        let file_list = get_list_of_files_from_db(&pg_conn, &constraints)?;

        match file_list.len() {
            0 => Ok("".to_string()),
            1 => {
                let file_name = file_list.get(0).expect("This shouldn't be happening...");
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", self.config.cache_dir, file_name);
                let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                    Ok(g) => {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    }
                    Err(_) => {
                        let gps_file = format!("{}/{}", &self.config.gps_dir, file_name);

                        let corr_list = GarminCorrectionList::read_corrections_from_db(&pg_conn)?;
                        let corr_map = corr_list.get_corr_list_map();

                        debug!("Reading gps_file: {}", &gps_file);
                        garmin_parse::GarminParse::new()
                            .with_file(&gps_file, &corr_map)
                            .gfile
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
                    &self.config.maps_api_key,
                    &htmlcachedir,
                    &history,
                    &self.config.gps_dir,
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
}

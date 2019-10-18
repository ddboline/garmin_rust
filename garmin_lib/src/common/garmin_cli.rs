use chrono::{DateTime, Utc};
use clap::{App, Arg};
use failure::{err_msg, Error};
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{copy, rename};
use std::io::{stdout, Write};
use std::path::Path;
use tempdir::TempDir;

use crate::common::garmin_summary::get_maximum_begin_datetime;
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::reports::garmin_file_report_html::file_report_html;
use crate::reports::garmin_file_report_txt::generate_txt_report;
use crate::reports::garmin_report_options::{GarminReportAgg, GarminReportOptions};
use crate::reports::garmin_summary_report_html::summary_report_html;
use crate::reports::garmin_summary_report_txt::create_report_query;
use crate::utils::garmin_util::{extract_zip_from_garmin_connect, get_file_list};
use crate::utils::sport_types::get_sport_type_map;

use super::garmin_config::GarminConfig;
use super::garmin_connect_client::GarminConnectClient;
use super::garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionList};
use super::garmin_file;
use super::garmin_summary::{get_list_of_files_from_db, GarminSummary, GarminSummaryList};
use super::garmin_sync::GarminSync;
use super::pgpool::PgPool;

fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

#[derive(Debug, PartialEq)]
pub enum GarminCliOptions {
    Sync(bool),
    All,
    Bootstrap,
    FileNames(Vec<String>),
    ImportFileNames(Vec<String>),
    Connect,
}

#[derive(Debug, Default)]
pub struct GarminCli {
    pub config: GarminConfig,
    pub opts: Option<GarminCliOptions>,
    pub pool: Option<PgPool>,
    pub corr: GarminCorrectionList,
    pub parser: GarminParse,
}

impl GarminCli {
    /// ```
    /// # use garmin_lib::common::garmin_cli::GarminCli;
    /// # use garmin_lib::parsers::garmin_parse::GarminParse;
    /// # use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
    /// let gcli = GarminCli::new();
    /// assert_eq!(gcli.opts, None);
    /// ```
    pub fn new() -> GarminCli {
        GarminCli {
            config: GarminConfig::new(),
            ..Default::default()
        }
    }

    pub fn with_config() -> Result<GarminCli, Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let corr = GarminCorrectionList::from_pool(&pool);
        let obj = GarminCli {
            config,
            pool: Some(pool),
            corr,
            ..Default::default()
        };
        Ok(obj)
    }

    pub fn from_pool(pool: &PgPool) -> Result<GarminCli, Error> {
        let config = GarminConfig::get_config(None)?;
        let corr = GarminCorrectionList::from_pool(&pool);
        let obj = GarminCli {
            config,
            pool: Some(pool.clone()),
            corr,
            ..Default::default()
        };
        Ok(obj)
    }

    pub fn with_cli_proc() -> Result<GarminCli, Error> {
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
                    .help("Convert (a) file(s)"),
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
                Arg::with_name("check_md5sum")
                    .short("m")
                    .long("check_md5sum")
                    .value_name("CHECK_MD5SUM")
                    .help("Check file md5sums")
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
            .arg(
                Arg::with_name("import")
                    .short("i")
                    .long("import")
                    .value_name("IMPORT")
                    .multiple(true)
                    .help("Import fit file(s) rename and copy to cache"),
            )
            .arg(
                Arg::with_name("connect")
                    .short("c")
                    .long("connect")
                    .value_name("CONNECT")
                    .help("Download new files from Garmin Connect")
                    .takes_value(false),
            )
            .get_matches();

        let obj = GarminCli {
            opts: if matches.is_present("sync") {
                let check_md5sum = matches.is_present("check_md5sum");
                Some(GarminCliOptions::Sync(check_md5sum))
            } else if matches.is_present("all") {
                Some(GarminCliOptions::All)
            } else if matches.is_present("bootstrap") {
                Some(GarminCliOptions::Bootstrap)
            } else if matches.is_present("filename") {
                match matches
                    .values_of("filename")
                    .map(|f| f.map(|f| f.to_string()).collect())
                {
                    Some(v) => Some(GarminCliOptions::FileNames(v)),
                    None => None,
                }
            } else if matches.is_present("connect") {
                Some(GarminCliOptions::Connect)
            } else {
                match matches
                    .values_of("import")
                    .map(|f| f.map(|f| f.to_string()).collect())
                {
                    Some(v) => Some(GarminCliOptions::ImportFileNames(v)),
                    None => None,
                }
            },
            ..GarminCli::with_config()?
        };
        Ok(obj)
    }

    pub fn get_pool(&self) -> Result<PgPool, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| err_msg("No Database Connection"))
            .map(|x| x.clone())
    }

    pub fn get_config(&self) -> &GarminConfig {
        &self.config
    }

    pub fn get_opts(&self) -> &Option<GarminCliOptions> {
        &self.opts
    }

    pub fn get_corr(&self) -> &GarminCorrectionList {
        &self.corr
    }

    pub fn get_parser(&self) -> &GarminParse {
        &self.parser
    }

    pub fn garmin_proc(&self) -> Result<Vec<String>, Error> {
        if let Some(GarminCliOptions::Connect) = self.get_opts() {
            self.sync_with_garmin_connect()?;
        }

        if let Some(GarminCliOptions::ImportFileNames(filenames)) = self.get_opts() {
            self.extract_zip_files(filenames)?;
        }

        match self.get_opts() {
            Some(GarminCliOptions::Bootstrap) => self.run_bootstrap(),
            Some(GarminCliOptions::Sync(check_md5)) => self.sync_everything(*check_md5),
            _ => self.proc_everything(),
        }
    }

    pub fn proc_everything(&self) -> Result<Vec<String>, Error> {
        let corr_list = self.get_corr().read_corrections_from_db()?;
        let corr_map = corr_list.get_corr_list_map();

        let gsum_list = self.get_summary_list(&corr_map)?;

        if !gsum_list.summary_list.is_empty() {
            gsum_list.write_summary_to_avro_files(&self.get_config().summary_cache)?;
            gsum_list.write_summary_to_postgres().map(|_| Vec::new())
        } else {
            Ok(Vec::new())
        }
    }

    pub fn get_summary_list(
        &self,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let pg_conn = self.get_pool()?;

        let gsum_list = match self.get_opts() {
            Some(GarminCliOptions::FileNames(flist)) => {
                let proc_list: Result<Vec<_>, Error> = flist
                    .par_iter()
                    .map(|f| {
                        writeln!(stdout().lock(), "Process {}", &f)?;
                        Ok(GarminSummary::process_single_gps_file(
                            &f,
                            &self.get_config().cache_dir,
                            &corr_map,
                        )?)
                    })
                    .collect();
                GarminSummaryList::from_vec(proc_list?)
            }
            Some(GarminCliOptions::All) => GarminSummaryList::process_all_gps_files(
                &self.get_config().gps_dir,
                &self.get_config().cache_dir,
                &corr_map,
            )?,
            _ => {
                let path = Path::new(&self.get_config().cache_dir);
                let cacheset: HashSet<String> = get_file_list(&path)
                    .into_par_iter()
                    .filter_map(|f| {
                        if f.contains("garmin_correction.avro") {
                            None
                        } else {
                            f.split('/').last().map(|x| x.to_string())
                        }
                    })
                    .collect();

                let dbset: HashSet<String> = get_list_of_files_from_db(&[], &pg_conn)?
                    .into_iter()
                    .collect();

                let path = Path::new(&self.get_config().gps_dir);
                let proc_list: Result<Vec<_>, Error> = get_file_list(&path)
                    .into_par_iter()
                    .filter_map(|f| f.split('/').last().map(|x| x.to_string()))
                    .filter_map(|f| {
                        let cachefile = format!("{}.avro", f);
                        if dbset.contains(&f) && cacheset.contains(&cachefile) {
                            None
                        } else {
                            let gps_path = format!("{}/{}", &self.get_config().gps_dir, &f);
                            debug!("Process {}", &gps_path);
                            Some(gps_path)
                        }
                    })
                    .map(|f| {
                        GarminSummary::process_single_gps_file(
                            &f,
                            &self.get_config().cache_dir,
                            &corr_map,
                        )
                    })
                    .collect();
                GarminSummaryList::from_vec(proc_list?)
            }
        }
        .with_pool(&pg_conn);
        Ok(gsum_list)
    }

    pub fn run_bootstrap(&self) -> Result<Vec<String>, Error> {
        self.sync_everything(true)
    }

    pub fn sync_everything(&self, check_md5: bool) -> Result<Vec<String>, Error> {
        let gsync = GarminSync::new();

        let options = vec![
            (
                "Syncing GPS files",
                &self.get_config().gps_dir,
                &self.get_config().gps_bucket,
                check_md5,
            ),
            (
                "Syncing CACHE files",
                &self.get_config().cache_dir,
                &self.get_config().cache_bucket,
                check_md5,
            ),
            (
                "Syncing SUMMARY file",
                &self.get_config().summary_cache,
                &self.get_config().summary_bucket,
                check_md5,
            ),
        ];

        options
            .into_par_iter()
            .map(|(title, local_dir, s3_bucket, check_md5)| {
                writeln!(stdout().lock(), "{}", title)?;
                let mut output = vec![title.to_string()];
                output.extend_from_slice(&gsync.sync_dir(local_dir, s3_bucket, check_md5)?);
                Ok(output.join("\n"))
            })
            .collect()
    }

    pub fn cli_garmin_report(&self) -> Result<(), Error> {
        let matches = App::new("Garmin Rust Report")
            .version(get_version_number().as_str())
            .author("Daniel Boline <ddboline@gmail.com>")
            .about("Convert GPS files to avro format, dump stuff to postgres")
            .arg(Arg::with_name("patterns").multiple(true))
            .get_matches();

        let req = match matches.values_of("patterns") {
            Some(patterns) => {
                let strings: Vec<String> = patterns.map(|x| x.to_string()).collect();
                Self::process_pattern(&strings)
            }
            None => Self::process_pattern(&["year".to_string()]),
        };

        self.run_cli(&req.options, &req.constraints)
    }

    pub fn process_pattern(patterns: &[String]) -> GarminRequest {
        let mut options = GarminReportOptions::new();

        let sport_type_map = get_sport_type_map();

        let mut constraints: Vec<String> = Vec::new();

        for pattern in patterns {
            match pattern.as_str() {
                "year" => options.agg = Some(GarminReportAgg::Year),
                "month" => options.agg = Some(GarminReportAgg::Month),
                "week" => options.agg = Some(GarminReportAgg::Week),
                "day" => options.agg = Some(GarminReportAgg::Day),
                "file" => options.agg = Some(GarminReportAgg::File),
                "sport" => options.do_sport = None,
                "latest" => constraints.push(
                    "begin_datetime=(select max(begin_datetime) from garmin_summary)".to_string(),
                ),
                pat => match sport_type_map.get(pat) {
                    Some(&x) => options.do_sport = Some(x),
                    None => {
                        if pat.contains('w') {
                            let vals: Vec<_> = pat.split('w').collect();
                            if vals.len() >= 2 {
                                if let Ok(year) = vals[0].parse::<i32>() {
                                    if let Ok(week) = vals[1].parse::<i32>() {
                                        constraints.push(format!(
                                            "(EXTRACT(isoyear from begin_datetime at time zone 'localtime') = {} AND
                                            EXTRACT(week from begin_datetime at time zone 'localtime') = {})", year, week));
                                    }
                                }
                            }
                        } else {
                            constraints.push(
                                    format!(
                                        "replace({}, '%', 'T') like '%{}%'",
                                        "to_char(begin_datetime at time zone 'utc', 'YYYY-MM-DD%HH24:MI:SSZ')",
                                        pat
                                    )
                                );
                        }
                        constraints.push(format!("filename like '%{}%'", pat));
                    }
                },
            };
        }

        GarminRequest {
            options,
            constraints,
            ..Default::default()
        }
    }

    pub fn run_cli(
        &self,
        options: &GarminReportOptions,
        constraints: &[String],
    ) -> Result<(), Error> {
        let pg_conn = self.get_pool()?;

        let file_list = get_list_of_files_from_db(constraints, &pg_conn)?;

        match file_list.len() {
            0 => (),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", &self.get_config().cache_dir, file_name);
                let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                    Ok(g) => {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    }
                    Err(_) => {
                        let gps_file = format!("{}/{}", &self.get_config().gps_dir, file_name);

                        let corr_list = self.get_corr().read_corrections_from_db()?;
                        let corr_map = corr_list.get_corr_list_map();

                        debug!("Reading gps_file: {}", &gps_file);
                        GarminParse::new().with_file(&gps_file, &corr_map)?
                    }
                };
                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
                writeln!(
                    stdout().lock(),
                    "{}",
                    generate_txt_report(&gfile)?.join("\n")
                )?;
            }
            _ => {
                debug!("{:?}", options);
                let txt_result: Vec<_> = create_report_query(&pg_conn, &options, &constraints)?
                    .iter()
                    .map(|x| x.join(" "))
                    .collect();

                writeln!(stdout().lock(), "{}", txt_result.join("\n"))?;
            }
        };
        Ok(())
    }

    pub fn run_html(&self, req: &GarminRequest) -> Result<String, Error> {
        let pg_conn = self.get_pool()?;

        let file_list = get_list_of_files_from_db(&req.constraints, &pg_conn)?;

        match file_list.len() {
            0 => Ok("".to_string()),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| err_msg("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", self.get_config().cache_dir, file_name);
                let gfile = match garmin_file::GarminFile::read_avro(&avro_file) {
                    Ok(g) => {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    }
                    Err(_) => {
                        let gps_file = format!("{}/{}", &self.get_config().gps_dir, file_name);

                        let corr_list = self.get_corr().read_corrections_from_db()?;
                        let corr_map = corr_list.get_corr_list_map();

                        debug!("Reading gps_file: {}", &gps_file);
                        GarminParse::new().with_file(&gps_file, &corr_map)?
                    }
                };
                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());

                file_report_html(self.get_config(), &gfile, &req.history, Some(&pg_conn))
            }
            _ => {
                debug!("{:?}", req.options);
                let txt_result: Vec<_> =
                    create_report_query(&pg_conn, &req.options, &req.constraints)?
                        .iter()
                        .map(|x| x.join("</td><td>"))
                        .collect();

                summary_report_html(
                    &self.get_config().domain,
                    &txt_result,
                    &req.options,
                    &req.filter,
                    &req.history,
                )
            }
        }
    }

    pub fn extract_zip_files(&self, filenames: &[String]) -> Result<(), Error> {
        let tempdir = TempDir::new("garmin_zip")?;
        let ziptmpdir = tempdir.path().to_string_lossy().to_string();

        filenames
            .par_iter()
            .filter(|f| (f.ends_with(".zip") || f.ends_with(".fit")) && Path::new(f).exists())
            .map(|filename| {
                let filename = if filename.ends_with(".zip") {
                    extract_zip_from_garmin_connect(&filename, &ziptmpdir)?
                } else {
                    filename.into()
                };
                assert!(Path::new(&filename).exists(), "No such file");
                assert!(filename.ends_with(".fit"), "Only fit files are supported");
                let gfile = GarminParse::new().with_file(&filename, &HashMap::new())?;

                let outfile = format!(
                    "{}/{}",
                    &self.get_config().gps_dir,
                    gfile.get_standardized_name()?
                );

                writeln!(stdout().lock(), "{} {}", filename, outfile)?;

                rename(&filename, &outfile)
                    .or_else(|_| copy(&filename, &outfile).map(|_| ()))
                    .map_err(err_msg)
            })
            .collect()
    }

    pub fn sync_with_garmin_connect(&self) -> Result<Vec<String>, Error> {
        if let Some(pool) = self.pool.as_ref() {
            if let Some(max_datetime) = get_maximum_begin_datetime(&pool)? {
                let session = GarminConnectClient::get_session(self.config.clone())?;
                let filenames = session.get_activities(max_datetime)?;
                self.extract_zip_files(&filenames)?;
                return Ok(filenames);
            }
        }
        Ok(Vec::new())
    }
}

#[derive(Debug, Default)]
pub struct GarminRequest {
    pub filter: String,
    pub history: String,
    pub options: GarminReportOptions,
    pub constraints: Vec<String>,
}

use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{copy, rename};
use std::io::{stdout, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use tempdir::TempDir;
use tokio::task::spawn_blocking;

use crate::common::garmin_summary::get_maximum_begin_datetime;
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::parsers::garmin_parse_gmn::GarminParseGmn;
use crate::parsers::garmin_parse_tcx::GarminParseTcx;
use crate::parsers::garmin_parse_txt::GarminParseTxt;
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

lazy_static! {
    static ref YMD_REG: Regex =
        Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})").expect("Bad regex");
    static ref YM_REG: Regex = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})").expect("Bad regex");
    static ref Y_REG: Regex = Regex::new(r"(?P<year>\d{4})").expect("Bad regex");
}

#[derive(Debug, PartialEq, Clone)]
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
    pub pool: PgPool,
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
    pub fn new() -> Self {
        Self {
            config: GarminConfig::new(),
            ..Self::default()
        }
    }

    pub fn with_config() -> Result<Self, Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let corr = GarminCorrectionList::new(&pool);
        let obj = Self {
            config,
            pool,
            corr,
            ..Self::default()
        };
        Ok(obj)
    }

    pub fn from_pool(pool: &PgPool) -> Result<Self, Error> {
        let config = GarminConfig::get_config(None)?;
        let corr = GarminCorrectionList::new(&pool);
        let obj = Self {
            config,
            pool: pool.clone(),
            corr,
            ..Self::default()
        };
        Ok(obj)
    }

    pub fn get_pool(&self) -> PgPool {
        self.pool.clone()
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

    pub async fn garmin_proc(self) -> Result<Vec<String>, Error> {
        let cli = Arc::new(self);
        if let Some(GarminCliOptions::Connect) = cli.get_opts() {
            cli.sync_with_garmin_connect().await?;
        }

        if let Some(GarminCliOptions::ImportFileNames(filenames)) = cli.get_opts() {
            let filenames = filenames.clone();
            let cli = Arc::clone(&cli);
            cli.process_filenames(&filenames).await?;
        }

        match cli.get_opts() {
            Some(GarminCliOptions::Bootstrap) => cli.run_bootstrap().await,
            Some(GarminCliOptions::Sync(check_md5)) => cli.sync_everything(*check_md5).await,
            _ => cli.proc_everything().await,
        }
    }

    pub async fn proc_everything(&self) -> Result<Vec<String>, Error> {
        let corr_list = self.get_corr().read_corrections_from_db().await?;
        let corr_map = corr_list.get_corr_list_map();

        let gsum_list = Arc::new(self.get_summary_list(&corr_map).await?);

        if gsum_list.summary_list.is_empty() {
            Ok(Vec::new())
        } else {
            let gsum_list_ = Arc::clone(&gsum_list);
            let summary_cache = self.get_config().summary_cache.clone();
            spawn_blocking(move || gsum_list_.write_summary_to_avro_files(&summary_cache))
                .await??;
            gsum_list
                .write_summary_to_postgres()
                .await
                .map(|_| Vec::new())
        }
    }

    pub async fn get_summary_list(
        &self,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let pg_conn = self.get_pool();

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
                GarminSummaryList::from_vec(&pg_conn, proc_list?)
            }
            Some(GarminCliOptions::All) => GarminSummaryList::process_all_gps_files(
                &pg_conn,
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
                            f.split('/').last().map(ToString::to_string)
                        }
                    })
                    .collect();

                let dbset: HashSet<String> = get_list_of_files_from_db(&[], &pg_conn)
                    .await?
                    .into_iter()
                    .collect();

                let path = Path::new(&self.get_config().gps_dir);
                let proc_list: Result<Vec<_>, Error> = get_file_list(&path)
                    .into_par_iter()
                    .filter_map(|f| f.split('/').last().map(ToString::to_string))
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
                GarminSummaryList::from_vec(&pg_conn, proc_list?)
            }
        };
        Ok(gsum_list)
    }

    pub async fn run_bootstrap(&self) -> Result<Vec<String>, Error> {
        self.sync_everything(true).await
    }

    pub async fn sync_everything(&self, check_md5: bool) -> Result<Vec<String>, Error> {
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
            (
                "Syncing Fitbit Cache",
                &self.get_config().fitbit_cachedir,
                &self.get_config().fitbit_bucket,
                check_md5,
            ),
        ];

        let futures = options
            .into_iter()
            .map(|(title, local_dir, s3_bucket, check_md5)| {
                debug!("{}", title);
                gsync.sync_dir(title, local_dir, s3_bucket, check_md5)
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results.map(|results| {
            results
                .into_iter()
                .map(|output| output.join("\n"))
                .collect()
        })
    }

    fn match_patterns(config: &GarminConfig, pat: &str) -> Vec<String> {
        let mut constraints = Vec::new();
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
            let gps_file = format!("{}/{}", &config.gps_dir, pat);
            if Path::new(&gps_file).exists() {
                constraints.push(format!("filename = '{}'", pat));
            } else if DateTime::parse_from_rfc3339(&pat.replace("Z", "+00:00")).is_ok() {
                constraints.push(format!(
                    "replace({}, '%', 'T') = '{}'",
                    "to_char(begin_datetime at time zone 'utc', 'YYYY-MM-DD%HH24:MI:SSZ')", pat
                ));
            } else {
                let mut datelike_str = Vec::new();
                if YMD_REG.is_match(pat) {
                    for cap in YMD_REG.captures_iter(pat) {
                        let year = cap.name("year").map_or_else(|| "", |s| s.as_str());
                        let month = cap.name("month").map_or_else(|| "", |s| s.as_str());
                        let day = cap.name("day").map_or_else(|| "", |s| s.as_str());
                        datelike_str.push(format!("{}-{}-{}", year, month, day));
                    }
                } else if YM_REG.is_match(pat) {
                    for cap in YM_REG.captures_iter(pat) {
                        let year = cap.name("year").map_or_else(|| "", |s| s.as_str());
                        let month = cap.name("month").map_or_else(|| "", |s| s.as_str());
                        datelike_str.push(format!("{}-{}", year, month));
                    }
                } else if Y_REG.is_match(pat) {
                    for cap in Y_REG.captures_iter(pat) {
                        let year = cap.name("year").map_or_else(|| "", |s| s.as_str());
                        datelike_str.push(year.to_string());
                    }
                }
                for dstr in datelike_str {
                    constraints.push(format!(
                        "replace({}, '%', 'T') like '{}%'",
                        "to_char(begin_datetime at time zone 'localtime', 'YYYY-MM-DD%HH24:MI:SS')",
                        dstr
                    ));
                }
            }
        }
        constraints
    }

    pub fn process_pattern(config: &GarminConfig, patterns: &[String]) -> GarminRequest {
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
                pat => {
                    if let Some(x) = sport_type_map.get(pat) {
                        options.do_sport = Some(*x)
                    } else {
                        constraints.extend_from_slice(&Self::match_patterns(config, pat));
                    }
                }
            };
        }

        GarminRequest {
            options,
            constraints,
            ..GarminRequest::default()
        }
    }

    pub async fn run_cli(
        &self,
        options: &GarminReportOptions,
        constraints: &[String],
    ) -> Result<(), Error> {
        let pg_conn = self.get_pool();

        let file_list = get_list_of_files_from_db(constraints, &pg_conn).await?;

        let mut stdout = BufWriter::new(stdout());

        match file_list.len() {
            0 => (),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", &self.get_config().cache_dir, file_name);

                let gfile =
                    if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    } else {
                        let gps_file = format!("{}/{}", &self.get_config().gps_dir, file_name);
                        let corr_list = self.get_corr().read_corrections_from_db().await?;
                        debug!("Reading gps_file: {}", &gps_file);
                        spawn_blocking(move || {
                            let corr_map = corr_list.get_corr_list_map();
                            GarminParse::new().with_file(&gps_file, &corr_map)
                        })
                        .await??
                    };

                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
                writeln!(stdout, "{}", generate_txt_report(&gfile)?.join("\n"))?;
            }
            _ => {
                debug!("{:?}", options);
                let txt_result: Vec<_> = create_report_query(&pg_conn, &options, &constraints)
                    .await?
                    .iter()
                    .map(|x| x.join(" "))
                    .collect();

                writeln!(stdout, "{}", txt_result.join("\n"))?;
            }
        };
        Ok(())
    }

    pub async fn run_html(&self, req: &GarminRequest) -> Result<String, Error> {
        let pg_conn = self.get_pool();

        let file_list = get_list_of_files_from_db(&req.constraints, &pg_conn).await?;

        match file_list.len() {
            0 => Ok("".to_string()),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = format!("{}/{}.avro", self.get_config().cache_dir, file_name);

                let gfile =
                    if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                        debug!("Cached avro file read: {}", &avro_file);
                        g
                    } else {
                        let gps_file = format!("{}/{}", &self.get_config().gps_dir, file_name);

                        let corr_list = self.get_corr().read_corrections_from_db().await?;

                        debug!("Reading gps_file: {}", &gps_file);
                        spawn_blocking(move || {
                            let corr_map = corr_list.get_corr_list_map();
                            GarminParse::new().with_file(&gps_file, &corr_map)
                        })
                        .await??
                    };

                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());

                file_report_html(self.get_config(), &gfile, &req.history, &pg_conn).await
            }
            _ => {
                debug!("{:?}", req.options);
                let txt_result: Vec<_> =
                    create_report_query(&pg_conn, &req.options, &req.constraints)
                        .await?
                        .iter()
                        .map(|x| x.join("</td><td>"))
                        .collect();

                summary_report_html(
                    &self.get_config().domain,
                    &txt_result,
                    &req.options,
                    &req.history,
                )
            }
        }
    }

    fn transform_file_name(filename: &str) -> Result<String, Error> {
        macro_rules! check_filename {
            ($suffix:expr, $T:expr) => {
                let fname = format!("{}.{}", filename, $suffix);
                rename(&filename, &fname)?;
                if $T.with_file(&fname, &HashMap::new()).is_ok() {
                    return Ok(fname);
                }
                rename(&fname, &filename)?;
            };
        }
        check_filename!("fit", GarminParseTcx::new(true));
        check_filename!("tcx", GarminParseTcx::new(false));
        check_filename!("txt", GarminParseTxt::new());
        check_filename!("gmn", GarminParseGmn::new());

        Err(format_err!("Bad filename {}", filename))
    }

    pub async fn process_filenames(&self, filenames: &[String]) -> Result<(), Error> {
        let config = self.get_config().clone();

        let filenames = filenames.to_vec();
        spawn_blocking(move || {
            let tempdir = TempDir::new("garmin_zip")?;
            let ziptmpdir = tempdir.path().to_string_lossy().to_string();

            let filenames: Result<Vec<_>, Error> = filenames
                .iter()
                .map(|filename| match filename.to_lowercase().split('.').last() {
                    Some("zip") => extract_zip_from_garmin_connect(filename, &ziptmpdir),
                    Some("fit") | Some("tcx") | Some("txt") => Ok(filename.to_string()),
                    _ => Self::transform_file_name(&filename),
                })
                .collect();

            filenames?
                .into_par_iter()
                .map(|filename| {
                    assert!(Path::new(&filename).exists(), "No such file");
                    let suffix = match filename.to_lowercase().split('.').last() {
                        Some("fit") => "fit",
                        Some("tcx") => "tcx",
                        Some("txt") => "txt",
                        Some("gmn") => "gmn",
                        _ => return Err(format_err!("Bad filename {}", filename)),
                    };
                    let gfile = GarminParse::new().with_file(&filename, &HashMap::new())?;

                    let outfile = format!(
                        "{}/{}",
                        &config.gps_dir,
                        gfile.get_standardized_name(suffix)?
                    );

                    writeln!(stdout().lock(), "{} {}", filename, outfile)?;

                    if Path::new(&outfile).exists() {
                        return Ok(());
                    }

                    rename(&filename, &outfile)
                        .or_else(|_| copy(&filename, &outfile).map(|_| ()))
                        .map_err(Into::into)
                })
                .collect()
        })
        .await?
    }

    pub async fn sync_with_garmin_connect(&self) -> Result<Vec<String>, Error> {
        if let Some(max_datetime) = get_maximum_begin_datetime(&self.pool).await? {
            let session = GarminConnectClient::get_session(self.config.clone()).await?;
            let filenames = session.get_activities(max_datetime).await?;
            self.process_filenames(&filenames).await?;
            return Ok(filenames);
        }
        Ok(Vec::new())
    }
}

#[derive(Debug, Default)]
pub struct GarminRequest {
    pub filter: String,
    pub history: Vec<String>,
    pub options: GarminReportOptions,
    pub constraints: Vec<String>,
}

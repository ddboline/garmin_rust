use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use lazy_static::lazy_static;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use stack_string::StackString;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{copy, rename},
    path::{Path, PathBuf},
    sync::Arc,
};
use tempdir::TempDir;
use tokio::task::spawn_blocking;

use garmin_lib::{
    common::{
        garmin_config::GarminConfig,
        garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionMap},
        garmin_file,
        garmin_summary::{get_list_of_files_from_db, GarminSummary},
        garmin_sync::GarminSync,
        pgpool::PgPool,
    },
    parsers::{
        garmin_parse::{GarminParse, GarminParseTrait},
        garmin_parse_fit::GarminParseFit,
        garmin_parse_gmn::GarminParseGmn,
        garmin_parse_tcx::GarminParseTcx,
        garmin_parse_txt::GarminParseTxt,
    },
    utils::{
        garmin_util::{extract_zip_from_garmin_connect, get_file_list},
        sport_types::get_sport_type_map,
        stdout_channel::StdoutChannel,
    },
};
use garmin_reports::{
    garmin_file_report_html::file_report_html,
    garmin_file_report_txt::generate_txt_report,
    garmin_report_options::{GarminReportAgg, GarminReportOptions},
    garmin_summary_report_html::summary_report_html,
    garmin_summary_report_txt::create_report_query,
};

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
    FileNames(Vec<PathBuf>),
    ImportFileNames(Vec<PathBuf>),
    Connect,
}

#[derive(Debug, Default)]
pub struct GarminCli {
    pub config: GarminConfig,
    pub opts: Option<GarminCliOptions>,
    pub pool: PgPool,
    pub corr: GarminCorrectionMap,
    pub parser: GarminParse,
    pub stdout: StdoutChannel,
}

impl GarminCli {
    /// ```
    /// # use garmin_cli::garmin_cli::GarminCli;
    /// # use garmin_lib::parsers::garmin_parse::GarminParse;
    /// # use garmin_lib::common::garmin_correction_lap::GarminCorrectionMap;
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
        let corr = GarminCorrectionMap::new();
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
        let corr = GarminCorrectionMap::new();
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

    pub fn get_corr(&self) -> &GarminCorrectionMap {
        &self.corr
    }

    pub fn get_parser(&self) -> &GarminParse {
        &self.parser
    }

    pub async fn proc_everything(&self) -> Result<Vec<StackString>, Error> {
        let pool = self.get_pool();
        let corr_map = GarminCorrectionLap::read_corrections_from_db(&pool).await?;
        let summary_list = Arc::new(self.get_summary_list(&corr_map).await?);

        if summary_list.is_empty() {
            Ok(Vec::new())
        } else {
            spawn_blocking({
                let config = self.get_config().clone();
                let summary_list = summary_list.clone();
                move || {
                    GarminSummary::write_summary_to_avro_files(&summary_list, &config.summary_cache)
                }
            })
            .await??;
            let pool = self.get_pool();
            GarminSummary::write_summary_to_postgres(&summary_list, &pool)
                .await
                .map(|_| Vec::new())
        }
    }

    pub async fn get_summary_list(
        &self,
        corr_map: &HashMap<(DateTime<Utc>, i32), GarminCorrectionLap>,
    ) -> Result<Vec<GarminSummary>, Error> {
        let pg_conn = self.get_pool();

        let gsum_list = match self.get_opts() {
            Some(GarminCliOptions::FileNames(flist)) => {
                let proc_list: Result<Vec<_>, Error> = flist
                    .par_iter()
                    .map(|f| {
                        self.stdout.send(format!("Process {:?}", &f).into())?;
                        Ok(GarminSummary::process_single_gps_file(
                            &f,
                            &self.get_config().cache_dir,
                            &corr_map,
                        )?)
                    })
                    .collect();
                proc_list?
            }
            Some(GarminCliOptions::All) => GarminSummary::process_all_gps_files(
                &self.get_config().gps_dir,
                &self.get_config().cache_dir,
                &corr_map,
            )?,
            _ => {
                let cacheset: HashSet<StackString> = get_file_list(&self.get_config().cache_dir)
                    .into_par_iter()
                    .filter_map(|f| {
                        if f.to_string_lossy().contains("garmin_correction.avro") {
                            None
                        } else {
                            f.file_name()
                                .map(|f| f.to_string_lossy().to_string().into())
                        }
                    })
                    .collect();

                let dbset: HashSet<StackString> = get_list_of_files_from_db("", &pg_conn)
                    .await?
                    .into_iter()
                    .collect();

                let proc_list: Result<Vec<_>, Error> = get_file_list(&self.get_config().gps_dir)
                    .into_par_iter()
                    .filter_map(|f| f.file_name().map(|x| x.to_string_lossy().to_string()))
                    .filter_map(|f| {
                        let cachefile = format!("{}.avro", f);
                        if dbset.contains(f.as_str()) && cacheset.contains(cachefile.as_str()) {
                            None
                        } else {
                            let gps_path = self.get_config().gps_dir.join(&f);
                            debug!("Process {:?}", &gps_path);
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
                proc_list?
            }
        };
        Ok(gsum_list)
    }

    pub async fn run_bootstrap(&self) -> Result<Vec<StackString>, Error> {
        self.sync_everything(true).await
    }

    pub async fn sync_everything(&self, check_md5: bool) -> Result<Vec<StackString>, Error> {
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
                gsync.sync_dir(title, &local_dir, &s3_bucket, check_md5)
            });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results.map(|results| {
            results
                .into_iter()
                .map(|output| output.join("\n").into())
                .collect()
        })
    }

    fn match_patterns(config: &GarminConfig, pat: &str) -> Vec<StackString> {
        let mut constraints: Vec<StackString> = Vec::new();
        if pat.contains('w') {
            let vals: Vec<_> = pat.split('w').collect();
            if vals.len() >= 2 {
                if let Ok(year) = vals[0].parse::<i32>() {
                    if let Ok(week) = vals[1].parse::<i32>() {
                        constraints.push(
                            format!(
                                "(EXTRACT(isoyear from begin_datetime at time zone 'localtime') = \
                                 {} AND
                            EXTRACT(week from begin_datetime at time zone 'localtime') = {})",
                                year, week
                            )
                            .into(),
                        );
                    }
                }
            }
        } else {
            let gps_file = config.gps_dir.join(pat);
            if gps_file.exists() {
                constraints.push(format!("filename = '{}'", pat).into());
            } else if DateTime::parse_from_rfc3339(&pat.replace("Z", "+00:00")).is_ok() {
                constraints.push(
                    format!(
                        "replace({}, '%', 'T') = '{}'",
                        "to_char(begin_datetime at time zone 'utc', 'YYYY-MM-DD%HH24:MI:SSZ')", pat
                    )
                    .into(),
                );
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
                    constraints.push(
                        format!(
                            "replace({}, '%', 'T') like '{}%'",
                            "to_char(begin_datetime at time zone 'localtime', \
                             'YYYY-MM-DD%HH24:MI:SS')",
                            dstr
                        )
                        .into(),
                    );
                }
            }
        }
        constraints
    }

    pub fn process_pattern<T: AsRef<str>>(config: &GarminConfig, patterns: &[T]) -> GarminRequest {
        let mut options = GarminReportOptions::new();

        let sport_type_map = get_sport_type_map();

        let mut constraints: Vec<StackString> = Vec::new();

        for pattern in patterns {
            match pattern.as_ref() {
                "year" => options.agg = Some(GarminReportAgg::Year),
                "month" => options.agg = Some(GarminReportAgg::Month),
                "week" => options.agg = Some(GarminReportAgg::Week),
                "day" => options.agg = Some(GarminReportAgg::Day),
                "file" => options.agg = Some(GarminReportAgg::File),
                "sport" => options.do_sport = None,
                "latest" => constraints
                    .push("begin_datetime=(select max(begin_datetime) from garmin_summary)".into()),
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

    pub async fn run_cli<T: AsRef<str>>(
        &self,
        options: &GarminReportOptions,
        constraints: &[T],
    ) -> Result<(), Error> {
        let pg_conn = self.get_pool();
        let constraints: Vec<_> = constraints.iter().map(AsRef::as_ref).collect();

        let file_list = get_list_of_files_from_db(&constraints.join(" OR "), &pg_conn).await?;

        match file_list.len() {
            0 => (),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = self
                    .get_config()
                    .cache_dir
                    .join(file_name.as_str())
                    .with_extension("avro");

                let gfile =
                    if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                        debug!("Cached avro file read: {:?}", &avro_file);
                        g
                    } else {
                        let gps_file = self.get_config().gps_dir.join(file_name.as_str());
                        let pool = self.get_pool();
                        let corr_map = GarminCorrectionLap::read_corrections_from_db(&pool).await?;
                        debug!("Reading gps_file: {:?}", &gps_file);
                        spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map))
                            .await??
                    };

                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
                self.stdout
                    .send(generate_txt_report(&gfile)?.join("\n").into())?;
            }
            _ => {
                debug!("{:?}", options);
                let txt_result: Vec<_> = create_report_query(&pg_conn, &options, &constraints)
                    .await?
                    .get_text_entries()?
                    .into_iter()
                    .map(|x| x.join(" "))
                    .collect();

                self.stdout.send(txt_result.join("\n").into())?;
            }
        };
        Ok(())
    }

    pub async fn run_html(&self, req: &GarminRequest, is_demo: bool) -> Result<StackString, Error> {
        let pg_conn = self.get_pool();

        let file_list = get_list_of_files_from_db(&req.constraints.join(" OR "), &pg_conn).await?;

        match file_list.len() {
            0 => Ok("".into()),
            1 => {
                let file_name = file_list
                    .get(0)
                    .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = self.get_config().cache_dir.join(file_name.as_str());

                let gfile =
                    if let Ok(g) = garmin_file::GarminFile::read_avro_async(&avro_file).await {
                        debug!("Cached avro file read: {:?}", &avro_file);
                        g
                    } else {
                        let gps_file = self.get_config().gps_dir.join(file_name.as_str());
                        let pool = self.get_pool();
                        let corr_map = GarminCorrectionLap::read_corrections_from_db(&pool).await?;

                        debug!("Reading gps_file: {:?}", &gps_file);
                        spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map))
                            .await??
                    };

                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());

                file_report_html(self.get_config(), &gfile, &req.history, &pg_conn, is_demo).await
            }
            _ => {
                debug!("{:?}", req.options);
                let txt_result =
                    create_report_query(&pg_conn, &req.options, &req.constraints).await?;

                summary_report_html(
                    &self.get_config().domain,
                    &txt_result,
                    &req.history,
                    is_demo,
                )
            }
        }
    }

    fn transform_file_name(filename: &Path) -> Result<PathBuf, Error> {
        macro_rules! check_filename {
            ($suffix:expr, $T:expr) => {
                let fname = filename.with_extension($suffix);
                rename(&filename, &fname)?;
                if $T.with_file(&fname, &HashMap::new()).is_ok() {
                    return Ok(fname.to_path_buf());
                }
                rename(&fname, &filename)?;
            };
        }
        check_filename!("fit", GarminParseFit::new());
        check_filename!("tcx", GarminParseTcx::new());
        check_filename!("txt", GarminParseTxt::new());
        check_filename!("gmn", GarminParseGmn::new());

        Err(format_err!("Bad filename {:?}", filename))
    }

    pub async fn process_filenames<T: AsRef<Path>>(&self, filenames: &[T]) -> Result<(), Error> {
        let config = self.get_config().clone();
        let stdout = self.stdout.clone();
        let filenames: Vec<_> = filenames.iter().map(|s| s.as_ref().to_path_buf()).collect();
        spawn_blocking(move || {
            let tempdir = TempDir::new("garmin_zip")?;
            let ziptmpdir = tempdir.path();

            let filenames: Result<Vec<_>, Error> = filenames
                .iter()
                .map(|filename| match filename.extension().map(OsStr::to_str) {
                    Some(Some("zip")) => extract_zip_from_garmin_connect(filename, ziptmpdir),
                    Some(Some("fit")) | Some(Some("tcx")) | Some(Some("txt")) => {
                        Ok(filename.to_path_buf())
                    }
                    _ => Self::transform_file_name(filename.as_ref()),
                })
                .collect();

            filenames?
                .into_par_iter()
                .map(|filename| {
                    assert!(filename.exists(), "No such file");
                    let suffix = match filename.extension().and_then(OsStr::to_str) {
                        Some("fit") => "fit",
                        Some("tcx") => "tcx",
                        Some("txt") => "txt",
                        Some("gmn") => "gmn",
                        _ => return Err(format_err!("Bad filename {:?}", filename)),
                    };
                    let gfile = GarminParse::new().with_file(&filename, &HashMap::new())?;

                    let outfile = config
                        .gps_dir
                        .join(gfile.get_standardized_name(suffix).as_str());

                    stdout.send(format!("{:?} {:?}", filename, outfile).into())?;

                    if outfile.exists() {
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
}

#[derive(Debug, Default)]
pub struct GarminRequest {
    pub filter: StackString,
    pub history: Vec<StackString>,
    pub options: GarminReportOptions,
    pub constraints: Vec<StackString>,
}

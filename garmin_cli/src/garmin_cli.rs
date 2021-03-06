use anyhow::{format_err, Error};
use chrono::{DateTime, NaiveDate, Utc};
use futures::future::try_join_all;
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use stack_string::StackString;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{copy, rename},
    path::{Path, PathBuf},
    sync::Arc,
};
use stdout_channel::StdoutChannel;
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
    utils::garmin_util::{extract_zip_from_garmin_connect, get_file_list},
};
use garmin_reports::{
    garmin_constraints::GarminConstraints, garmin_file_report_html::file_report_html,
    garmin_file_report_txt::generate_txt_report, garmin_report_options::GarminReportOptions,
    garmin_summary_report_html::summary_report_html,
    garmin_summary_report_txt::create_report_query,
};

#[derive(Debug, PartialEq, Clone)]
pub enum GarminCliOptions {
    Sync(bool),
    All,
    Bootstrap,
    FileNames(Vec<PathBuf>),
    ImportFileNames(Vec<PathBuf>),
    Connect {
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    },
}

#[derive(Debug, Default)]
pub struct GarminCli {
    pub config: GarminConfig,
    pub opts: Option<GarminCliOptions>,
    pub pool: PgPool,
    pub corr: GarminCorrectionMap,
    pub parser: GarminParse,
    pub stdout: StdoutChannel<StackString>,
}

impl GarminCli {
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
            Some(GarminCliOptions::FileNames(flist)) => flist
                .par_iter()
                .map(|f| {
                    self.stdout.send(format!("Process {:?}", &f));
                    GarminSummary::process_single_gps_file(
                        &f,
                        &self.get_config().cache_dir,
                        &corr_map,
                    )
                })
                .collect::<Result<Vec<_>, Error>>()?,
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

                get_file_list(&self.get_config().gps_dir)
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
                    .collect::<Result<Vec<_>, Error>>()?
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
        try_join_all(futures).await
    }

    pub fn process_pattern<T, U>(config: &GarminConfig, patterns: T) -> GarminRequest
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let mut constraints = GarminConstraints::default();

        let options = constraints.process_pattern(config, patterns);

        GarminRequest {
            options,
            constraints,
            ..GarminRequest::default()
        }
    }

    pub async fn run_cli(
        &self,
        options: &GarminReportOptions,
        constraints: &GarminConstraints,
    ) -> Result<(), Error> {
        let pg_conn = self.get_pool();
        let file_list = get_list_of_files_from_db(&constraints.to_query_string(), &pg_conn).await?;

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
                self.stdout.send(generate_txt_report(&gfile)?.join("\n"));
            }
            _ => {
                debug!("{:?}", options);
                let txt_result = create_report_query(&pg_conn, &options, &constraints)
                    .await?
                    .get_text_entries()?
                    .into_iter()
                    .map(|x| x.into_iter().map(|(s, _)| s).join(" "))
                    .join("\n");
                self.stdout.send(txt_result);
            }
        };
        Ok(())
    }

    pub async fn run_html(&self, req: &GarminRequest, is_demo: bool) -> Result<StackString, Error> {
        let pg_conn = self.get_pool();

        let file_list =
            get_list_of_files_from_db(&req.constraints.to_query_string(), &pg_conn).await?;

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

    fn process_filenames_sync(
        filenames: Vec<PathBuf>,
        stdout: &StdoutChannel<StackString>,
        config: &GarminConfig,
    ) -> Result<Vec<DateTime<Utc>>, Error> {
        let tempdir = TempDir::new("garmin_zip")?;
        let ziptmpdir = tempdir.path();

        let filenames: Result<Vec<_>, Error> = filenames
            .into_par_iter()
            .map(|filename| match filename.extension().map(OsStr::to_str) {
                Some(Some("zip")) => extract_zip_from_garmin_connect(&filename, ziptmpdir),
                Some(Some("fit" | "tcx" | "txt")) => Ok(filename),
                _ => Self::transform_file_name(&filename),
            })
            .collect();

        filenames?
            .into_par_iter()
            .map(|filename| {
                assert!(
                    filename.exists(),
                    "No such file {}",
                    filename.to_string_lossy()
                );
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

                stdout.send(format!("{:?} {:?}", filename, outfile));

                if outfile.exists() {
                    return Ok(None);
                }

                rename(&filename, &outfile).or_else(|_| copy(&filename, &outfile).map(|_| ()))?;
                Ok(Some(gfile.begin_datetime))
            })
            .filter_map(Result::transpose)
            .collect()
    }

    pub async fn process_filenames<T, U>(&self, filenames: T) -> Result<Vec<DateTime<Utc>>, Error>
    where
        T: IntoIterator<Item = U>,
        U: AsRef<Path>,
    {
        let config = self.get_config().clone();
        let stdout = self.stdout.clone();
        let filenames: Vec<_> = filenames
            .into_iter()
            .map(|s| s.as_ref().to_path_buf())
            .collect();
        spawn_blocking(move || Self::process_filenames_sync(filenames, &stdout, &config)).await?
    }
}

#[derive(Debug, Default)]
pub struct GarminRequest {
    pub filter: StackString,
    pub history: Vec<StackString>,
    pub options: GarminReportOptions,
    pub constraints: GarminConstraints,
}

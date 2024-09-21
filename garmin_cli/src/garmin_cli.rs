use anyhow::{format_err, Error};
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{copy, rename},
    path::{Path, PathBuf},
    sync::Arc,
};
use stdout_channel::StdoutChannel;
use tempfile::TempDir;
use time::Date;
use tokio::task::spawn_blocking;

use fitbit_lib::fitbit_archive::archive_fitbit_heartrates;
use garmin_lib::{date_time_wrapper::DateTimeWrapper, garmin_config::GarminConfig};
use garmin_models::{
    garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionMap},
    garmin_file,
    garmin_summary::{get_list_of_files_from_db, GarminSummary},
    garmin_sync::GarminSync,
};
use garmin_parser::{
    garmin_parse::{GarminParse, GarminParseTrait},
    garmin_parse_fit::GarminParseFit,
    garmin_parse_gmn::GarminParseGmn,
    garmin_parse_tcx::GarminParseTcx,
    garmin_parse_txt::GarminParseTxt,
};
use garmin_reports::{
    garmin_constraints::GarminConstraints, garmin_file_report_txt::generate_txt_report,
    garmin_report_options::GarminReportOptions, garmin_summary_report_txt::create_report_query,
};
use garmin_utils::{
    garmin_util::{extract_zip_from_garmin_connect, get_file_list},
    pgpool::PgPool,
};

#[derive(Debug, PartialEq, Clone, Eq)]
pub enum GarminCliOptions {
    Sync,
    All,
    Bootstrap,
    FileNames(Vec<PathBuf>),
    ImportFileNames(Vec<PathBuf>),
    Connect {
        data_directory: Option<PathBuf>,
        start_date: Option<Date>,
        end_date: Option<Date>,
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: GarminConfig::default(),
            ..Self::default()
        }
    }

    /// # Errors
    /// Return error if config init fails
    pub fn with_config() -> Result<Self, Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl)?;
        let corr = GarminCorrectionMap::new();
        let obj = Self {
            config,
            pool,
            corr,
            ..Self::default()
        };
        Ok(obj)
    }

    /// # Errors
    /// Return error if config init fails
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

    #[must_use]
    pub fn get_pool(&self) -> PgPool {
        self.pool.clone()
    }

    #[must_use]
    pub fn get_config(&self) -> &GarminConfig {
        &self.config
    }

    #[must_use]
    pub fn get_opts(&self) -> &Option<GarminCliOptions> {
        &self.opts
    }

    #[must_use]
    pub fn get_corr(&self) -> &GarminCorrectionMap {
        &self.corr
    }

    #[must_use]
    pub fn get_parser(&self) -> &GarminParse {
        &self.parser
    }

    /// # Errors
    /// Return error if `read_corrections_from_db` fails or `get_summary_list`
    /// fails
    pub async fn proc_everything(&self) -> Result<Vec<StackString>, Error> {
        let pool = self.get_pool();
        let mut corr_map = GarminCorrectionLap::read_corrections_from_db(&pool).await?;
        corr_map.shrink_to_fit();
        let summary_list = Arc::new(self.get_summary_list(&corr_map).await?);

        if summary_list.is_empty() {
            Ok(Vec::new())
        } else {
            let pool = self.get_pool();
            GarminSummary::write_summary_to_postgres(&summary_list, &pool)
                .await
                .map(|()| Vec::new())
        }
    }

    /// # Errors
    /// Return error if reading summary list fails
    pub async fn get_summary_list(
        &self,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<Vec<GarminSummary>, Error> {
        let config = self.get_config();
        let pg_conn = self.get_pool();

        let mut gsum_list: Vec<_> = match self.get_opts() {
            Some(GarminCliOptions::FileNames(flist)) => flist
                .par_iter()
                .map(|f| {
                    self.stdout.send(format_sstr!("Process {f:?}"));
                    GarminParse::process_single_gps_file(f, &config.cache_dir, corr_map)
                })
                .collect::<Result<Vec<_>, Error>>()?,
            Some(GarminCliOptions::All) => {
                GarminParse::process_all_gps_files(&config.gps_dir, &config.cache_dir, corr_map)?
            }
            _ => {
                let cacheset: HashSet<StackString> = get_file_list(&config.cache_dir)
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
                    .try_collect()
                    .await?;

                get_file_list(&config.gps_dir)
                    .into_par_iter()
                    .filter_map(|f| f.file_name().map(|x| x.to_string_lossy().to_string()))
                    .filter_map(|f| {
                        let cachefile = format_sstr!("{f}.avro");
                        if dbset.contains(f.as_str()) && cacheset.contains(cachefile.as_str()) {
                            None
                        } else {
                            let gps_path = config.gps_dir.join(&f);
                            debug!("Process {:?}", &gps_path);
                            Some(gps_path)
                        }
                    })
                    .map(|f| GarminParse::process_single_gps_file(&f, &config.cache_dir, corr_map))
                    .collect::<Result<Vec<_>, Error>>()?
            }
        };
        gsum_list.shrink_to_fit();
        Ok(gsum_list)
    }

    /// # Errors
    /// Return error if `sync_everything` fails
    pub async fn run_bootstrap(&self) -> Result<Vec<StackString>, Error> {
        self.sync_everything().await
    }

    /// # Errors
    /// Return error if `sync_dir` fails
    pub async fn sync_everything(&self) -> Result<Vec<StackString>, Error> {
        let config = self.get_config();
        let sdk_config = aws_config::load_from_env().await;
        let gsync = GarminSync::new(&sdk_config);

        let options = vec![
            ("Syncing GPS files", &config.gps_dir, &config.gps_bucket),
            (
                "Syncing CACHE files",
                &config.cache_dir,
                &config.cache_bucket,
            ),
            (
                "Syncing Fitbit Cache",
                &config.fitbit_cachedir,
                &config.fitbit_bucket,
            ),
            (
                "Syncing Fitbit Archive",
                &config.fitbit_archivedir,
                &config.fitbit_archive_bucket,
            ),
        ];

        let futures = options.into_iter().map(|(title, local_dir, s3_bucket)| {
            debug!("{}", title);
            let pool = self.pool.clone();
            let gsync = gsync.clone();
            async move { gsync.sync_dir(title, local_dir, s3_bucket, &pool).await }
        });
        let mut results = try_join_all(futures).await?;
        results
            .extend_from_slice(&archive_fitbit_heartrates(&self.config, &self.pool, false).await?);
        Ok(results)
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

    /// # Errors
    /// Return error if various function fail
    pub async fn run_cli(
        &self,
        options: &GarminReportOptions,
        constraints: &GarminConstraints,
    ) -> Result<(), Error> {
        let config = self.get_config();
        let pg_conn = self.get_pool();
        let mut file_list: Vec<_> =
            get_list_of_files_from_db(&constraints.to_query_string(), &pg_conn)
                .await?
                .try_collect()
                .await?;
        file_list.shrink_to_fit();

        match file_list.len() {
            0 => (),
            1 => {
                let file_name = file_list
                    .first()
                    .ok_or_else(|| format_err!("This shouldn't be happening..."))?;
                debug!("{}", &file_name);
                let avro_file = self
                    .get_config()
                    .cache_dir
                    .join(file_name.as_str())
                    .with_extension("avro");

                let gfile = if let Ok(g) =
                    garmin_file::GarminFile::read_avro_async(&avro_file).await
                {
                    debug!("Cached avro file read: {:?}", &avro_file);
                    g
                } else {
                    let gps_file = config.gps_dir.join(file_name.as_str());
                    let pool = self.get_pool();
                    let mut corr_map = GarminCorrectionLap::read_corrections_from_db(&pool).await?;
                    corr_map.shrink_to_fit();
                    debug!("Reading gps_file: {:?}", &gps_file);
                    spawn_blocking(move || GarminParse::new().with_file(&gps_file, &corr_map))
                        .await??
                };

                debug!("gfile {} {}", gfile.laps.len(), gfile.points.len());
                self.stdout.send(generate_txt_report(&gfile)?.join("\n"));
            }
            _ => {
                debug!("{:?}", options);
                let txt_result = create_report_query(&pg_conn, options, constraints)
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
    ) -> Result<Vec<DateTimeWrapper>, Error> {
        let tempdir = TempDir::new()?;
        let ziptmpdir = tempdir.path();

        let mut filenames = filenames
            .into_par_iter()
            .map(|filename| match filename.extension().map(OsStr::to_str) {
                Some(Some("zip")) => extract_zip_from_garmin_connect(&filename, ziptmpdir),
                Some(Some("fit" | "tcx" | "txt")) => Ok(filename),
                _ => Self::transform_file_name(&filename),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        filenames.shrink_to_fit();

        let mut result = filenames
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

                stdout.send(format_sstr!("{filename:?} {outfile:?}"));

                if outfile.exists() {
                    return Ok(None);
                }

                rename(&filename, &outfile).or_else(|_| copy(&filename, &outfile).map(|_| ()))?;
                Ok(Some(gfile.begin_datetime))
            })
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, Error>>()?;
        result.shrink_to_fit();
        Ok(result)
    }

    /// # Errors
    /// Return error if `process_filenames_sync` fails
    pub async fn process_filenames(
        &self,
        filenames: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<Vec<DateTimeWrapper>, Error> {
        let config = self.get_config().clone();
        let stdout = self.stdout.clone();

        #[allow(clippy::needless_collect)]
        let mut filenames: Vec<_> = filenames
            .into_iter()
            .map(|s| s.as_ref().to_path_buf())
            .collect();
        filenames.shrink_to_fit();

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

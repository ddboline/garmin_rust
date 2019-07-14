use clap::{App, Arg};
use failure::{err_msg, Error};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{stdout, Write};
use std::path::Path;
use tempdir::TempDir;

use super::garmin_config::GarminConfig;
use super::garmin_correction_lap::{GarminCorrectionLap, GarminCorrectionList};
use super::garmin_file;
use super::garmin_summary::{get_list_of_files_from_db, GarminSummary, GarminSummaryList};
use super::garmin_sync::GarminSync;
use super::pgpool::PgPool;
use crate::parsers::garmin_parse::{GarminParse, GarminParseTrait};
use crate::reports::garmin_file_report_html::file_report_html;
use crate::reports::garmin_file_report_txt::generate_txt_report;
use crate::reports::garmin_report_options::{GarminReportAgg, GarminReportOptions};
use crate::reports::garmin_summary_report_html::summary_report_html;
use crate::reports::garmin_summary_report_txt::create_report_query;
use crate::utils::garmin_util::{
    extract_zip_files, get_file_list, map_result, sync_with_garmin_connect,
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

#[derive(Debug, PartialEq)]
pub enum GarminCliOptions {
    Sync,
    All,
    Bootstrap,
    FileNames(Vec<String>),
    ImportFileNames(Vec<String>),
    Connect,
}

#[derive(Debug, Default)]
pub struct GarminCliObj<T = GarminParse>
where
    T: GarminParseTrait,
{
    pub config: GarminConfig,
    pub opts: Option<GarminCliOptions>,
    pub pool: Option<PgPool>,
    pub corr: GarminCorrectionList,
    pub parser: T,
}

impl<T> GarminCliObj<T>
where
    T: GarminParseTrait + Default,
{
    /// ```
    /// # use garmin_lib::common::garmin_cli::GarminCliObj;
    /// # use garmin_lib::parsers::garmin_parse::GarminParse;
    /// # use garmin_lib::common::garmin_correction_lap::GarminCorrectionList;
    /// let gcli = GarminCliObj::<GarminParse>::new();
    /// assert_eq!(gcli.opts, None);
    /// ```
    pub fn new() -> GarminCliObj<T> {
        GarminCliObj {
            config: GarminConfig::new(),
            ..Default::default()
        }
    }

    pub fn with_config() -> Result<GarminCliObj<T>, Error> {
        let config = GarminConfig::get_config(None)?;
        let pool = PgPool::new(&config.pgurl);
        let corr = GarminCorrectionList::from_pool(&pool);
        let obj = GarminCliObj {
            config,
            pool: Some(pool),
            corr,
            ..Default::default()
        };
        Ok(obj)
    }

    pub fn from_pool(pool: &PgPool) -> Result<GarminCliObj<T>, Error> {
        let config = GarminConfig::get_config(None)?;
        let obj = GarminCliObj {
            config,
            pool: Some(pool.clone()),
            ..Default::default()
        };
        Ok(obj)
    }

    pub fn with_cli_proc() -> Result<GarminCliObj<T>, Error> {
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
                    .help("Download new files from Garmin Connect"),
            )
            .get_matches();

        let obj = GarminCliObj {
            opts: if matches.is_present("sync") {
                Some(GarminCliOptions::Sync)
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
            ..GarminCliObj::with_config()?
        };
        Ok(obj)
    }
}

impl GarminCli for GarminCliObj<GarminParse> {
    fn get_pool(&self) -> Result<PgPool, Error> {
        self.pool
            .as_ref()
            .ok_or_else(|| err_msg("No Database Connection"))
            .map(|x| x.clone())
    }

    fn get_config(&self) -> &GarminConfig {
        &self.config
    }

    fn get_opts(&self) -> &Option<GarminCliOptions> {
        &self.opts
    }

    fn get_corr(&self) -> &GarminCorrectionList {
        &self.corr
    }

    fn get_parser(&self) -> &GarminParse {
        &self.parser
    }
}

pub trait GarminCli<T = GarminParse>
where
    Self: Send + Sync,
    T: GarminParseTrait + Send + Sync,
{
    fn get_pool(&self) -> Result<PgPool, Error>;

    fn get_config(&self) -> &GarminConfig;

    fn get_opts(&self) -> &Option<GarminCliOptions>;

    fn get_corr(&self) -> &GarminCorrectionList;

    fn get_parser(&self) -> &T;

    fn garmin_proc(&self) -> Result<(), Error> {
        if let Some(GarminCliOptions::Connect) = self.get_opts() {
            let pool = self.get_pool()?;
            sync_with_garmin_connect(&pool, &self.get_config().gps_dir)?;
        }

        if let Some(GarminCliOptions::ImportFileNames(filenames)) = self.get_opts() {
            extract_zip_files(filenames, &self.get_config().gps_dir)?;
        }

        match self.get_opts() {
            Some(GarminCliOptions::Bootstrap) => self.run_bootstrap(),
            Some(GarminCliOptions::Sync) => self.sync_everything(),
            _ => {
                let corr_list = self.get_corr().read_corrections_from_db()?;
                let corr_map = corr_list.get_corr_list_map();

                let gsum_list = self.get_summary_list(&corr_map)?;

                if !gsum_list.summary_list.is_empty() {
                    gsum_list.write_summary_to_avro_files(&self.get_config().summary_cache)?;
                    gsum_list.write_summary_to_postgres()?;
                    corr_list.dump_corr_list_to_avro(&format!(
                        "{}/garmin_correction.avro",
                        &self.get_config().cache_dir
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }

    fn get_summary_list(
        &self,
        corr_map: &HashMap<(String, i32), GarminCorrectionLap>,
    ) -> Result<GarminSummaryList, Error> {
        let pg_conn = self.get_pool()?;

        let gsum_list = match self.get_opts() {
            Some(GarminCliOptions::FileNames(flist)) => {
                let proc_list: Vec<Result<_, Error>> = flist
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
                GarminSummaryList::from_vec(map_result(proc_list)?)
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
                let proc_list: Vec<Result<_, Error>> = get_file_list(&path)
                    .into_par_iter()
                    .filter_map(|f| f.split('/').last().map(|x| x.to_string()))
                    .filter_map(|f| {
                        let cachefile = format!("{}.avro", f);
                        if dbset.contains(&f) && cacheset.contains(&cachefile) {
                            None
                        } else {
                            let gps_path = format!("{}/{}", &self.get_config().gps_dir, &f);
                            println!("Process {}", &gps_path);
                            Some(gps_path)
                        }
                    })
                    .map(|f| {
                        Ok(GarminSummary::process_single_gps_file(
                            &f,
                            &self.get_config().cache_dir,
                            &corr_map,
                        )?)
                    })
                    .collect();
                GarminSummaryList::from_vec(map_result(proc_list)?)
            }
        }
        .with_pool(&pg_conn);
        Ok(gsum_list)
    }

    fn run_bootstrap(&self) -> Result<(), Error> {
        let pg_conn = self.get_pool()?;

        self.sync_everything()?;

        writeln!(stdout().lock(), "Read corrections from avro file")?;
        let corr_list = GarminCorrectionList::read_corr_list_from_avro(&format!(
            "{}/garmin_correction.avro",
            &self.get_config().cache_dir
        ))?
        .with_pool(&pg_conn);

        writeln!(stdout().lock(), "Write corrections to postgres")?;
        corr_list.dump_corrections_to_db()?;

        writeln!(stdout().lock(), "Read summaries from avro files")?;
        let cache = &self.get_config().summary_cache;
        let gsum_list = GarminSummaryList::from_avro_files(cache)?.with_pool(&pg_conn);

        writeln!(stdout().lock(), "Write summaries to postgres")?;
        gsum_list.write_summary_to_postgres()
    }

    fn sync_everything(&self) -> Result<(), Error> {
        let gsync = GarminSync::new();

        let options = vec![
            (
                "Syncing GPS files",
                &self.get_config().gps_dir,
                &self.get_config().gps_bucket,
                true,
            ),
            (
                "Syncing CACHE files",
                &self.get_config().cache_dir,
                &self.get_config().cache_bucket,
                false,
            ),
            (
                "Syncing SUMMARY file",
                &self.get_config().summary_cache,
                &self.get_config().summary_bucket,
                false,
            ),
        ];

        let results: Vec<_> = options
            .into_par_iter()
            .map(|(title, local_dir, s3_bucket, check_md5)| {
                writeln!(stdout().lock(), "{}", title)?;
                gsync.sync_dir(local_dir, s3_bucket, check_md5)
            })
            .collect();

        map_result(results)
    }

    fn cli_garmin_report(&self) -> Result<(), Error> {
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
            None => {
                let default_patterns = vec!["year".to_string()];
                Self::process_pattern(&default_patterns)
            }
        };

        self.run_cli(&req.options, &req.constraints)
    }

    fn process_pattern(patterns: &[String]) -> GarminRequest {
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

        GarminRequest {
            options,
            constraints,
            ..Default::default()
        }
    }

    fn run_cli(&self, options: &GarminReportOptions, constraints: &[String]) -> Result<(), Error> {
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

    fn run_html(&self, req: &GarminRequest) -> Result<String, Error> {
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

                let tempdir = TempDir::new("garmin_html")?;
                let htmlcachedir = tempdir
                    .path()
                    .to_str()
                    .ok_or_else(|| err_msg("Path is invalid unicode somehow"))?;

                file_report_html(
                    &gfile,
                    &self.get_config().maps_api_key,
                    &htmlcachedir,
                    &req.history,
                    &self.get_config().gps_dir,
                )
            }
            _ => {
                debug!("{:?}", req.options);
                let txt_result: Vec<_> =
                    create_report_query(&pg_conn, &req.options, &req.constraints)?
                        .iter()
                        .map(|x| x.join("</td><td>"))
                        .collect();

                let tempdir = TempDir::new("garmin_html")?;
                let htmlcachedir = tempdir
                    .path()
                    .to_str()
                    .ok_or_else(|| err_msg("Path is invalid unicode somehow"))?;

                summary_report_html(
                    &txt_result,
                    &req.options,
                    &htmlcachedir,
                    &req.filter,
                    &req.history,
                )
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct GarminRequest {
    pub filter: String,
    pub history: String,
    pub options: GarminReportOptions,
    pub constraints: Vec<String>,
}

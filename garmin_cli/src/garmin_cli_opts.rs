use clap::Parser;
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use log::info;
use refinery::embed_migrations;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{BTreeSet, HashMap},
    ffi::OsStr,
    fmt,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use time::{macros::format_description, Date, Duration, OffsetDateTime};
use time_tz::OffsetDateTimeExt;
use tokio::{
    fs::{metadata, read_to_string, remove_file, write, File},
    io::{stdin, stdout, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::spawn_blocking,
};

use derive_more::{From, Into};
use fitbit_lib::{
    fitbit_archive::{
        archive_fitbit_heartrates, get_heartrate_values, get_number_of_heartrate_values,
    },
    fitbit_heartrate::{import_garmin_heartrate_file, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
    GarminConnectHrData,
};
use garmin_connect_lib::garmin_connect_client::GarminConnectClient;
use garmin_lib::{
    date_time_wrapper::DateTimeWrapper, errors::GarminError as Error, garmin_config::GarminConfig,
};
use garmin_models::{
    fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
    garmin_connect_har_file::GarminConnectHarFile,
    strava_activities_har_file::StravaActivityHarFile, strava_activity::StravaActivity,
};
use garmin_utils::{garmin_util::extract_zip_from_garmin_connect_multiple, pgpool::PgPool};
use race_result_analysis::{race_results::RaceResults, race_type::RaceType};
use std::str::FromStr;
use strava_lib::strava_client::StravaClient;

use crate::garmin_cli::{GarminCli, GarminCliOptions};

embed_migrations!("../migrations");

#[derive(Into, From, Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct DateType(Date);

impl FromStr for DateType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Date::parse(s, format_description!("[year]-[month]-[day]"))
            .map(Self)
            .map_err(Into::into)
    }
}

#[derive(Default)]
pub struct GarminConnectSyncOutput {
    pub filenames: Vec<PathBuf>,
    pub input_files: Vec<PathBuf>,
    pub dates: Vec<Date>,
}

#[derive(Parser, PartialEq, Eq)]
pub enum GarminCliOpts {
    #[clap(alias = "boot")]
    Bootstrap,
    Proc {
        #[clap(short, long, use_value_delimiter = true, value_delimiter = ',')]
        filename: Vec<PathBuf>,
    },
    #[clap(alias = "rpt")]
    Report {
        #[clap(short, long, use_value_delimiter = true, value_delimiter = ',')]
        patterns: Vec<StackString>,
    },
    #[clap(alias = "cnt")]
    /// Go to the network tab in chrome developer tools,
    /// Navigate to `<https://connect.garmin.com/modern/activities>`,
    /// find the entry for `<https://connect.garmin.com/activitylist-service/activities/search/activities>`,
    /// go to the response subtab and copy the output to
    /// `~/Downloads/garmin_connect/activities.json`, Next navigate to `<https://connect.garmin.com/modern/daily-summary/{date}>` where date is a date e.g. 2022-12-20,
    /// find the entry `<https://connect.garmin.com/wellness-service/wellness/dailyHeartRate/ddboline?date=2022-12-18>`,
    /// go to the response subtab and copy the output to
    /// `~/Downloads/garmin_connect/heartrates.json`
    Connect {
        #[clap(short, long)]
        data_directory: Option<PathBuf>,
        #[clap(short, long)]
        start_date: Option<DateType>,
        #[clap(short, long)]
        end_date: Option<DateType>,
    },
    Sync,
    Strava,
    Import {
        #[clap(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: GarminTables,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[clap(short, long)]
        /// table: allowed values: ['scale_measurements', 'strava_activities',
        /// 'fitbit_activities', 'garmin_connect_activities',
        /// 'race_results', 'heartrate_statistics_summary']
        table: GarminTables,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
    },
    SyncAll,
    /// Run refinery migrations
    RunMigrations,
    #[clap(alias = "archive")]
    FitbitArchive {
        #[clap(short, long)]
        all: bool,
    },
    #[clap(alias = "fit-archive-read")]
    FitbitArchiveRead {
        #[clap(short, long)]
        start_date: Option<DateType>,
        #[clap(short, long)]
        end_date: Option<DateType>,
        #[clap(short = 't', long)]
        step_size: Option<usize>,
    },
}

impl GarminCliOpts {
    /// # Errors
    /// Return error if config fails, or `process_opts` fails
    pub async fn process_args() -> Result<(), Error> {
        let config = GarminConfig::get_config(None)?;
        let opts = Self::parse();

        if opts == Self::SyncAll {
            Self::Connect {
                data_directory: None,
                start_date: None,
                end_date: None,
            }
            .process_opts(&config)
            .await?;
            Self::Strava.process_opts(&config).await?;
            Self::Sync.process_opts(&config).await
        } else {
            opts.process_opts(&config).await
        }
    }

    async fn get_options(
        self,
        config: &GarminConfig,
        pool: &PgPool,
    ) -> Result<Option<GarminCliOptions>, Error> {
        match self {
            Self::Bootstrap => Ok(Some(GarminCliOptions::Bootstrap)),
            Self::Proc { filename } => Ok(Some(GarminCliOptions::ImportFileNames(filename))),
            Self::Report { patterns } => {
                let req = if patterns.is_empty() {
                    GarminCli::process_pattern(config, ["year"])
                } else {
                    GarminCli::process_pattern(config, &patterns)
                };
                let cli = GarminCli::with_config()?;
                cli.run_cli(&req.options, &req.constraints).await?;
                return cli.stdout.close().await.map_err(Into::into).map(|()| None);
            }
            Self::Connect {
                data_directory,
                start_date,
                end_date,
            } => Ok(Some(GarminCliOptions::Connect {
                data_directory,
                start_date: start_date.map(Into::into),
                end_date: end_date.map(Into::into),
            })),
            Self::Sync => Ok(Some(GarminCliOptions::Sync)),
            Self::SyncAll => Ok(None),
            Self::Strava => {
                let cli = GarminCli::with_config()?;
                let activity_names = Self::sync_with_strava(&cli)
                    .await?
                    .into_iter()
                    .map(|a| a.name)
                    .join("\n");
                cli.stdout.send(activity_names);
                return cli.stdout.close().await.map_err(Into::into).map(|()| None);
            }
            Self::Import { table, filepath } => {
                let data = if let Some(filepath) = filepath {
                    read_to_string(&filepath).await?
                } else {
                    let mut stdin = stdin();
                    let mut buf = String::new();
                    stdin.read_to_string(&mut buf).await?;
                    buf
                };
                return table.import_table(pool, &data).await.map(|()| None);
            }
            Self::Export { table, filepath } => {
                let mut file: Box<dyn AsyncWrite + Unpin> = if let Some(filepath) = filepath {
                    Box::new(File::create(&filepath).await?)
                } else {
                    Box::new(stdout())
                };
                return table.export_table(pool, &mut file).await.map(|()| None);
            }
            Self::RunMigrations => {
                let mut client = pool.get().await?;
                migrations::runner().run_async(&mut **client).await?;
                Ok(None)
            }
            Self::FitbitArchive { all } => {
                let result = archive_fitbit_heartrates(config, pool, all).await?;
                stdout().write_all(result.join("\n").as_bytes()).await?;
                stdout().write_all(b"\n").await?;
                Ok(None)
            }
            Self::FitbitArchiveRead {
                start_date,
                end_date,
                step_size,
            } => {
                let start_date = start_date.map_or_else(
                    || (OffsetDateTime::now_utc() - Duration::days(1)).date(),
                    Into::into,
                );
                let end_date = end_date.map_or_else(
                    || (OffsetDateTime::now_utc() + Duration::days(1)).date(),
                    Into::into,
                );
                let count = {
                    let config = config.clone();
                    spawn_blocking(move || {
                        get_number_of_heartrate_values(&config, start_date, end_date)
                    })
                    .await??
                };
                let config = config.clone();
                let values = spawn_blocking(move || {
                    get_heartrate_values(&config, start_date, end_date, step_size)
                })
                .await??;
                let mut values: Vec<_> = values
                    .into_iter()
                    .map(|hr| format_sstr!("{d} {v}", d = hr.datetime, v = hr.value))
                    .collect();
                values.shrink_to_fit();
                let s = format_sstr!("count {count} {}", values.len());
                stdout().write_all(s.as_bytes()).await?;
                stdout().write_all(b"\n").await?;
                Ok(None)
            }
        }
    }

    async fn process_opts(self, config: &GarminConfig) -> Result<(), Error> {
        let pool = PgPool::new(&config.pgurl)?;

        let Some(opts) = self.get_options(config, &pool).await? else {
            return Ok(());
        };

        let cli = GarminCli {
            opts: Some(opts),
            pool,
            config: config.clone(),
            ..GarminCli::with_config()?
        };

        Self::garmin_proc(&cli).await?;
        cli.stdout.close().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn garmin_proc(cli: &GarminCli) -> Result<(), Error> {
        let results = match cli.get_opts() {
            Some(GarminCliOptions::ImportFileNames(filenames)) => {
                let filenames = filenames.clone();
                cli.process_filenames(&filenames).await?;
                cli.proc_everything().await
            }
            Some(GarminCliOptions::Bootstrap) => cli.run_bootstrap().await,
            Some(GarminCliOptions::Sync) => cli.sync_everything().await,
            Some(GarminCliOptions::Connect {
                data_directory,
                start_date,
                end_date,
            }) => {
                let mut buf = cli.proc_everything().await?;
                let output = if data_directory.is_some() {
                    Self::sync_with_garmin_connect(
                        cli,
                        data_directory,
                        *start_date,
                        *end_date,
                        true,
                    )
                    .await?
                } else {
                    Self::sync_with_garmin_connect_api(cli, *start_date, *end_date).await?
                };
                let GarminConnectSyncOutput {
                    filenames,
                    input_files,
                    dates,
                } = output;
                if !filenames.is_empty() || !input_files.is_empty() || !dates.is_empty() {
                    buf.extend_from_slice(&cli.sync_everything().await?);
                }
                Ok(buf)
            }
            _ => cli.proc_everything().await,
        }?;
        cli.stdout.send(results.join("\n"));
        Ok(())
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn sync_with_garmin_connect(
        cli: &GarminCli,
        data_directory: &Option<PathBuf>,
        start_date: Option<Date>,
        end_date: Option<Date>,
        check_for_date_json: bool,
    ) -> Result<GarminConnectSyncOutput, Error> {
        async fn exists_and_is_not_empty(path: &Path) -> bool {
            metadata(path)
                .await
                .map_or_else(|_| false, |m| m.size() > 0)
        }

        let har_file = cli.config.download_directory.join("connect.garmin.com.har");
        let data_directory = data_directory
            .as_ref()
            .unwrap_or(&cli.config.garmin_connect_import_directory);
        let activites_json = data_directory.join("activities.json");
        let heartrate_json = data_directory.join("heartrates.json");

        let mut input_files = Vec::new();
        let mut filenames = Vec::new();
        let mut activities = Vec::new();
        let mut dates = BTreeSet::new();
        if exists_and_is_not_empty(&har_file).await {
            let buf = read_to_string(&har_file).await?;
            if !buf.is_empty() {
                let har: GarminConnectHarFile = serde_json::from_str(buf.trim())?;
                activities = har.get_activities()?;
                for buf in har.get_heartrates() {
                    let hr_values: GarminConnectHrData = serde_json::from_str(buf)?;
                    let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                    let config = cli.config.clone();
                    dates.extend(
                        spawn_blocking(move || {
                            FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                        })
                        .await??,
                    );
                }
                input_files.push(har_file);
            }
        }
        if activities.is_empty() && exists_and_is_not_empty(&activites_json).await {
            let buf = read_to_string(&activites_json).await?;
            if !buf.is_empty() {
                activities = serde_json::from_str(buf.trim())?;
                input_files.push(activites_json);
            }
        }
        if !activities.is_empty() {
            for activity in
                GarminConnectActivity::merge_new_activities(activities, &cli.pool).await?
            {
                let filename = cli
                    .config
                    .download_directory
                    .join(format_sstr!("{}.zip", activity.activity_id));
                if exists_and_is_not_empty(&filename).await {
                    filenames.push(filename);
                }
            }
        }
        if exists_and_is_not_empty(&heartrate_json).await {
            let buf = read_to_string(&heartrate_json).await?;
            for line in buf.split('\n') {
                if line.is_empty() {
                    continue;
                }
                if let Ok(hr_values) = serde_json::from_str::<GarminConnectHrData>(line) {
                    let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                    let config = cli.config.clone();
                    dates.extend(
                        spawn_blocking(move || {
                            FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                        })
                        .await??,
                    );
                }
            }
            if !buf.is_empty() {
                input_files.push(heartrate_json);
            }
        }
        let start_date =
            start_date.unwrap_or_else(|| (OffsetDateTime::now_utc() - Duration::days(3)).date());
        let end_date = end_date.unwrap_or_else(|| OffsetDateTime::now_utc().date());
        if check_for_date_json {
            let mut date = start_date;
            while date <= end_date {
                info!("get heartrate {date}");
                let heartrate_file = data_directory.join(format_sstr!("{date}.json"));
                if heartrate_file.exists() {
                    let hr_values: GarminConnectHrData = serde_json::from_reader(
                        File::open(&heartrate_file).await?.into_std().await,
                    )?;
                    info!("got heartrate {date}");
                    let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
                    let config = cli.config.clone();
                    let mut new_dates = spawn_blocking(move || {
                        FitbitHeartRate::merge_slice_to_avro(&config, &hr_values)
                    })
                    .await??;
                    dates.append(&mut new_dates);
                    remove_file(&heartrate_file).await?;
                }
                let connect_wellness_file = data_directory.join(format_sstr!("{date}"));
                let connect_wellness_file = if connect_wellness_file.exists() {
                    connect_wellness_file
                } else {
                    cli.config.download_directory.join(format_sstr!("{date}"))
                };
                if connect_wellness_file.exists() {
                    let tempdir = TempDir::with_prefix("garmin_cli_opts")?;
                    let ziptmpdir = tempdir.path().to_path_buf();
                    let wellness_files = spawn_blocking(move || {
                        extract_zip_from_garmin_connect_multiple(&connect_wellness_file, &ziptmpdir)
                    })
                    .await??;
                    for wellness_file in wellness_files {
                        let config = cli.config.clone();
                        if let Ok(mut new_dates) = spawn_blocking(move || {
                            import_garmin_heartrate_file(&config, &wellness_file)
                        })
                        .await?
                        {
                            dates.append(&mut new_dates);
                        }
                    }
                }
                date += Duration::days(1);
            }
        }
        for date in &dates {
            if let Some(stat) =
                FitbitHeartRate::calculate_summary_statistics(&cli.config, &cli.pool, *date).await?
            {
                info!("update stats {}", stat.date);
            }
        }
        if !filenames.is_empty() {
            let datetimes = cli.process_filenames(&filenames).await?;
            info!("number of files {}", datetimes.len());
        }
        if !filenames.is_empty() || !input_files.is_empty() {
            for line in cli.proc_everything().await? {
                info!("{line}");
            }
            GarminConnectActivity::fix_summary_id_in_db(&cli.pool).await?;
            for line in archive_fitbit_heartrates(&cli.config, &cli.pool, false).await? {
                info!("{line}");
            }
        }

        let har_file = cli.config.download_directory.join("www.strava.com.har");
        if exists_and_is_not_empty(&har_file).await {
            let buf = read_to_string(&har_file).await?;
            if !buf.is_empty() {
                let har: StravaActivityHarFile = serde_json::from_str(&buf)?;
                let activities: Vec<StravaActivity> =
                    har.get_activities()?.map_or(Vec::new(), |a| {
                        a.models.into_iter().map(Into::into).collect()
                    });
                if !activities.is_empty() {
                    StravaActivity::upsert_activities(&activities, &cli.pool).await?;
                    StravaActivity::fix_summary_id_in_db(&cli.pool).await?;
                }
                input_files.push(har_file);
            }
        }

        for f in &input_files {
            if f.extension() == Some(OsStr::new("har")) {
                remove_file(f).await?;
            } else {
                write(f, &[]).await?;
            }
        }
        let mut dates: Vec<_> = dates.into_iter().collect();

        filenames.sort();
        input_files.sort();
        dates.sort();

        filenames.shrink_to_fit();
        input_files.shrink_to_fit();
        dates.shrink_to_fit();

        Ok(GarminConnectSyncOutput {
            filenames,
            input_files,
            dates,
        })
    }

    /// # Errors
    /// Return error if various function fail
    pub async fn sync_with_strava(cli: &GarminCli) -> Result<Vec<StravaActivity>, Error> {
        let config = cli.config.clone();
        let start_datetime = Some(OffsetDateTime::now_utc() - Duration::days(30));
        let end_datetime = Some(OffsetDateTime::now_utc());

        let client = StravaClient::with_auth(config).await?;
        let activities = client
            .sync_with_client(start_datetime, end_datetime, &cli.pool)
            .await?;

        if !activities.is_empty() {
            cli.proc_everything().await?;
        }

        Ok(activities)
    }

    /// # Errors
    /// Returns error on various
    pub async fn sync_with_garmin_connect_api(
        cli: &GarminCli,
        start_date: Option<Date>,
        end_date: Option<Date>,
    ) -> Result<GarminConnectSyncOutput, Error> {
        let mut dates = BTreeSet::new();
        let mut filenames = Vec::new();
        let mut activities = Vec::new();

        let mut client = GarminConnectClient::new(cli.config.clone())?;
        client.init().await?;

        let new_activities = client.get_activities(Some(0), Some(10)).await?;
        let new_activities =
            GarminConnectActivity::merge_new_activities(new_activities, &cli.pool).await?;
        activities.extend(&new_activities);
        let missing_activities: HashMap<_, _> =
            GarminConnectActivity::activities_to_download(&cli.pool)
                .await?
                .map_ok(|activity| (activity.activity_id, activity))
                .try_collect()
                .await?;

        for activity in new_activities {
            if missing_activities.contains_key(&activity.activity_id) {
                filenames.push(client.download_activity(activity.activity_id).await?);
            }
        }

        let now = OffsetDateTime::now_utc();
        let start_date = start_date.unwrap_or_else(|| (now - Duration::days(3)).date());
        let end_date = end_date.unwrap_or_else(|| now.date());

        let local_tz = DateTimeWrapper::local_tz();

        let mut measurement_map: HashMap<_, _> =
            ScaleMeasurement::read_from_db(&cli.pool, Some(start_date), Some(end_date), None, None)
                .await?
                .into_iter()
                .map(|m| (m.datetime.to_timezone(local_tz).date(), m))
                .collect();

        let mut date = start_date;

        while date <= end_date {
            let hr_values = client.get_heartrate(date).await?;
            let hr_values = FitbitHeartRate::from_garmin_connect_hr(&hr_values);
            let config = cli.config.clone();
            dates.extend(
                spawn_blocking(move || FitbitHeartRate::merge_slice_to_avro(&config, &hr_values))
                    .await??,
            );
            let weights = client.get_weight(date).await?;
            if !weights.date_weight_list.is_empty() {
                let weight = &weights.date_weight_list[0];
                if let Some(measurement) = measurement_map.get_mut(&date) {
                    if measurement.connect_primary_key.is_none()
                        && (weight.weight - measurement.mass_in_grams()) < 1.0
                    {
                        println!("set weight {weight:?}");
                        measurement
                            .set_connect_primary_key(weight.sample_primary_key, &cli.pool)
                            .await?;
                    }
                }
            } else if let Some(measurement) = measurement_map.get_mut(&date) {
                client.upload_weight(measurement).await?;
                let weight = client.get_weight(date).await?;
                if !weight.date_weight_list.is_empty()
                    && (weight.date_weight_list[0].weight - measurement.mass_in_grams()) < 1.0
                {
                    let primary_key = weight.date_weight_list[0].sample_primary_key;
                    println!("set weight {weight:?}");
                    measurement
                        .set_connect_primary_key(primary_key, &cli.pool)
                        .await?;
                }
            }
            date += Duration::days(1);
        }

        if !filenames.is_empty() {
            let datetimes = cli.process_filenames(&filenames).await?;
            info!("number of files {}", datetimes.len());
        }

        if !filenames.is_empty() {
            for line in cli.proc_everything().await? {
                info!("{line}");
            }
            GarminConnectActivity::fix_summary_id_in_db(&cli.pool).await?;
            for line in archive_fitbit_heartrates(&cli.config, &cli.pool, false).await? {
                info!("{line}");
            }
        }

        let mut dates: Vec<_> = dates.into_iter().collect();

        filenames.sort();
        dates.sort();

        filenames.shrink_to_fit();
        dates.shrink_to_fit();

        Ok(GarminConnectSyncOutput {
            filenames,
            input_files: Vec::new(),
            dates,
        })
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum GarminTables {
    ScaleMeasurement,
    StravaActivity,
    FitbitActivity,
    HeartrateStatisticsSummary,
    GarminConnectActivity,
    RaceResults,
}

impl FromStr for GarminTables {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "scale_measurements" => Ok(Self::ScaleMeasurement),
            "strava_activities" => Ok(Self::StravaActivity),
            "fitbit_activities" => Ok(Self::FitbitActivity),
            "heartrate_statistics_summary" => Ok(Self::HeartrateStatisticsSummary),
            "garmin_connect_activities" => Ok(Self::GarminConnectActivity),
            "race_results" => Ok(Self::RaceResults),
            _ => Err(Error::StaticCustomError("Invalid Table")),
        }
    }
}

impl fmt::Display for GarminTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

impl GarminTables {
    #[must_use]
    fn to_str(self) -> &'static str {
        match self {
            Self::ScaleMeasurement => "scale_measurements",
            Self::StravaActivity => "strava_activities",
            Self::FitbitActivity => "fitbit_activities",
            Self::HeartrateStatisticsSummary => "heartrate_statistics_summary",
            Self::GarminConnectActivity => "garmin_connect_activities",
            Self::RaceResults => "race_results",
        }
    }

    async fn import_table(&self, pool: &PgPool, data: &str) -> Result<(), Error> {
        match self {
            Self::ScaleMeasurement => {
                let mut measurements: Vec<ScaleMeasurement> = serde_json::from_str(data)?;
                ScaleMeasurement::merge_updates(&mut measurements, pool).await?;
                let s = format_sstr!("scale_measurements {}\n", measurements.len());
                stdout().write_all(s.as_bytes()).await?;
            }
            Self::StravaActivity => {
                let activities: Vec<StravaActivity> = serde_json::from_str(data)?;
                StravaActivity::upsert_activities(&activities, pool).await?;
                StravaActivity::fix_summary_id_in_db(pool).await?;
                let s = format_sstr!("strava_activities {}\n", activities.len());
                stdout().write_all(s.as_bytes()).await?;
            }
            Self::FitbitActivity => {
                let activities: Vec<FitbitActivity> = serde_json::from_str(data)?;
                FitbitActivity::upsert_activities(&activities, pool).await?;
                FitbitActivity::fix_summary_id_in_db(pool).await?;
                let s = format_sstr!("fitbit_activities {}\n", activities.len());
                stdout().write_all(s.as_bytes()).await?;
            }
            Self::HeartrateStatisticsSummary => {
                let entries: Vec<FitbitStatisticsSummary> = serde_json::from_str(data)?;
                let futures = entries.into_iter().map(|entry| {
                    let pool = pool.clone();
                    async move {
                        FitbitStatisticsSummary::upsert_entry(&entry, &pool).await?;
                        Ok(())
                    }
                });
                let results: Result<Vec<()>, Error> = try_join_all(futures).await;
                let s = format_sstr!("heartrate_statistics_summary {}\n", results?.len());
                stdout().write_all(s.as_bytes()).await?;
            }
            Self::GarminConnectActivity => {
                let activities: Vec<GarminConnectActivity> = serde_json::from_str(data)?;
                GarminConnectActivity::upsert_activities(&activities, pool).await?;
                GarminConnectActivity::fix_summary_id_in_db(pool).await?;
                let s = format_sstr!("garmin_connect_activities {}\n", activities.len());
                stdout().write_all(s.as_bytes()).await?;
            }
            Self::RaceResults => {
                let results: Vec<RaceResults> = serde_json::from_str(data)?;
                let futures = results.into_iter().map(|result| {
                    let pool = pool.clone();
                    async move {
                        result.update_db(&pool).await?;
                        Ok(())
                    }
                });
                let results: Result<Vec<()>, Error> = try_join_all(futures).await;
                let s = format_sstr!("race_results {}\n", results?.len());
                stdout().write_all(s.as_bytes()).await?;
            }
        }
        Ok(())
    }

    async fn export_table(
        &self,
        pool: &PgPool,
        file: &mut Box<dyn AsyncWrite + Unpin>,
    ) -> Result<(), Error> {
        let local = DateTimeWrapper::local_tz();
        match self {
            Self::ScaleMeasurement => {
                let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                    .to_timezone(local)
                    .date();
                let measurements =
                    ScaleMeasurement::read_from_db(pool, Some(start_date), None, None, None)
                        .await?;
                let v = serde_json::to_vec(&measurements)?;
                file.write_all(&v).await?;
            }
            Self::StravaActivity => {
                let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                    .to_timezone(local)
                    .date();
                let mut activities: Vec<_> =
                    StravaActivity::read_from_db(pool, Some(start_date), None, None, None)
                        .await?
                        .try_collect()
                        .await?;
                activities.shrink_to_fit();
                let v = serde_json::to_vec(&activities)?;
                file.write_all(&v).await?;
            }
            Self::FitbitActivity => {
                let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                    .to_timezone(local)
                    .date();
                let activities =
                    FitbitActivity::read_from_db(pool, Some(start_date), None, None, None).await?;
                let v = serde_json::to_vec(&activities)?;
                file.write_all(&v).await?;
            }
            Self::HeartrateStatisticsSummary => {
                let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                    .to_timezone(local)
                    .date();
                let mut entries: Vec<_> =
                    FitbitStatisticsSummary::read_from_db(pool, Some(start_date), None, None, None)
                        .await?
                        .try_collect()
                        .await?;
                entries.shrink_to_fit();
                let v = serde_json::to_vec(&entries)?;
                file.write_all(&v).await?;
            }
            Self::GarminConnectActivity => {
                let start_date = (OffsetDateTime::now_utc() - Duration::days(7))
                    .to_timezone(local)
                    .date();
                let mut activities: Vec<_> =
                    GarminConnectActivity::read_from_db(pool, Some(start_date), None, None, None)
                        .await?
                        .try_collect()
                        .await?;
                activities.shrink_to_fit();
                let v = serde_json::to_vec(&activities)?;
                file.write_all(&v).await?;
            }
            Self::RaceResults => {
                let mut results: Vec<_> =
                    RaceResults::get_results_by_type(RaceType::Personal, pool)
                        .await?
                        .try_collect()
                        .await?;
                results.shrink_to_fit();
                let v = serde_json::to_vec(&results)?;
                file.write_all(&v).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};
    use stdout_channel::StdoutChannel;

    use crate::garmin_cli::{GarminCli, GarminCliOptions};
    use fitbit_lib::GarminConnectHrData;
    use garmin_lib::{errors::GarminError as Error, garmin_config::GarminConfig};
    use garmin_models::{
        garmin_connect_har_file::GarminConnectHarFile, garmin_correction_lap::GarminCorrectionMap,
        strava_activities_har_file::StravaActivityHarFile, strava_activity::StravaActivity,
    };
    use garmin_parser::garmin_parse::GarminParse;
    use garmin_utils::pgpool::PgPool;

    #[tokio::test]
    #[ignore]
    async fn test_garmin_file_test_filenames() -> Result<(), Error> {
        let test_config = "tests/data/test.env";
        let config = GarminConfig::get_config(Some(test_config))?;
        let pool = PgPool::new(&config.pgurl)?;
        let corr = GarminCorrectionMap::new();

        let gcli = GarminCli {
            config,
            opts: Some(GarminCliOptions::FileNames(vec![
                "../tests/data/test.fit".into(),
                "../tests/data/test.gmn".into(),
                "../tests/data/test.tcx".into(),
                "../tests/data/test.txt".into(),
                "../tests/data/test.tcx.gz".into(),
            ])),
            pool,
            corr,
            parser: GarminParse::new(),
            stdout: StdoutChannel::new(),
        };

        assert!(gcli.opts.is_some());
        Ok(())
    }

    #[test]
    fn test_garmin_connect_har_file() -> Result<(), Error> {
        let buf = include_str!("../../tests/data/connect.garmin.com.har");
        let har: GarminConnectHarFile = serde_json::from_str(buf)?;
        let activities = har.get_activities()?;
        assert_eq!(activities.len(), 20);
        let mut total = 0;
        for buf in har.get_heartrates() {
            let hr_values: GarminConnectHrData = serde_json::from_str(buf)?;
            assert!(hr_values.heartrate_values.is_some());
            let hr_values = hr_values.heartrate_values.unwrap();
            assert!(hr_values.len() > 0);
            total += hr_values.len();
        }
        assert_eq!(total, 1187);

        let p = Path::new("../../tests/data/connect.garmin.com.har");
        assert_eq!(p.extension(), Some(OsStr::new("har")));
        Ok(())
    }

    #[test]
    fn test_strava_activites_har_file() -> Result<(), Error> {
        let buf = include_str!("../../tests/data/www.strava.com.har");
        let har: StravaActivityHarFile = serde_json::from_str(buf)?;
        let js = har.get_activities()?;
        assert!(js.is_some());
        let js = js.unwrap();
        let activities: Vec<StravaActivity> = js.models.into_iter().map(Into::into).collect();
        let first_activity = activities.first().unwrap();
        assert_eq!(first_activity.name, "Hunter's Point");
        assert_eq!(first_activity.id, 13162225358);
        assert_eq!(first_activity.moving_time, Some(5185));
        Ok(())
    }
}

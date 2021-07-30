#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::similar_names)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::default_trait_access)]

pub mod datetime_wrapper;
pub mod errors;
pub mod garmin_requests;
pub mod garmin_rust_app;
pub mod garmin_rust_routes;
pub mod iso_8601_datetime_wrapper;
pub mod logged_user;
pub mod naivedate_wrapper;
pub mod sport_types_wrapper;
pub mod uuid_wrapper;

use derive_more::{From, Into};
use naivedate_wrapper::NaiveDateWrapper;
use rweb::openapi::{self, Entity, Schema};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use fitbit_lib::{
    fitbit_client::{FitbitBodyWeightFatUpdateOutput, FitbitUserProfile},
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
};
use garmin_connect_lib::garmin_connect_client::GarminConnectUserDailySummary;
use garmin_lib::common::{
    fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
    strava_activity::StravaActivity, strava_timezone::StravaTimeZone,
};
use race_result_analysis::{race_results::RaceResults, race_type::RaceType};
use strava_lib::strava_client::StravaAthlete;

use crate::{datetime_wrapper::DateTimeWrapper, sport_types_wrapper::SportTypesWrapper};

#[derive(Into, From, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
pub struct StravaTimeZoneWrapper(StravaTimeZone);

impl Entity for StravaTimeZoneWrapper {
    #[inline]
    fn describe() -> Schema {
        Schema {
            schema_type: Some(openapi::Type::String),
            format: "timezone".into(),
            ..Schema::default()
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, rweb::Schema)]
pub struct FitbitHeartRateWrapper {
    #[serde(with = "iso_8601_datetime_wrapper")]
    pub datetime: DateTimeWrapper,
    pub value: i32,
}

impl From<FitbitHeartRate> for FitbitHeartRateWrapper {
    fn from(item: FitbitHeartRate) -> Self {
        Self {
            datetime: item.datetime.into(),
            value: item.value,
        }
    }
}

impl From<FitbitHeartRateWrapper> for FitbitHeartRate {
    fn from(item: FitbitHeartRateWrapper) -> Self {
        Self {
            datetime: item.datetime.into(),
            value: item.value,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, rweb::Schema)]
pub struct StravaActivityWrapper {
    pub name: StackString,
    #[serde(with = "iso_8601_datetime_wrapper")]
    pub start_date: DateTimeWrapper,
    pub id: i64,
    pub distance: Option<f64>,
    pub moving_time: Option<i64>,
    pub elapsed_time: i64,
    pub total_elevation_gain: Option<f64>,
    pub elev_high: Option<f64>,
    pub elev_low: Option<f64>,
    #[serde(rename = "type", with = "sport_types_wrapper")]
    pub activity_type: SportTypesWrapper,
    pub timezone: StravaTimeZoneWrapper,
}

impl From<StravaActivity> for StravaActivityWrapper {
    fn from(item: StravaActivity) -> Self {
        Self {
            name: item.name,
            start_date: item.start_date.into(),
            id: item.id,
            distance: item.distance,
            moving_time: item.moving_time,
            elapsed_time: item.elapsed_time,
            total_elevation_gain: item.total_elevation_gain,
            elev_high: item.elev_high,
            elev_low: item.elev_low,
            activity_type: item.activity_type.into(),
            timezone: item.timezone.into(),
        }
    }
}

impl From<StravaActivityWrapper> for StravaActivity {
    fn from(item: StravaActivityWrapper) -> Self {
        Self {
            name: item.name,
            start_date: item.start_date.into(),
            id: item.id,
            distance: item.distance,
            moving_time: item.moving_time,
            elapsed_time: item.elapsed_time,
            total_elevation_gain: item.total_elevation_gain,
            elev_high: item.elev_high,
            elev_low: item.elev_low,
            activity_type: item.activity_type.into(),
            timezone: item.timezone.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, rweb::Schema)]
pub struct FitbitBodyWeightFatWrapper {
    pub datetime: DateTimeWrapper,
    pub weight: f64,
    pub fat: f64,
}

impl From<FitbitBodyWeightFat> for FitbitBodyWeightFatWrapper {
    fn from(item: FitbitBodyWeightFat) -> Self {
        Self {
            datetime: item.datetime.into(),
            weight: item.weight,
            fat: item.fat,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq, rweb::Schema)]
pub struct ScaleMeasurementWrapper {
    pub id: i32,
    pub datetime: DateTimeWrapper,
    pub mass: f64,
    pub fat_pct: f64,
    pub water_pct: f64,
    pub muscle_pct: f64,
    pub bone_pct: f64,
}

impl From<ScaleMeasurement> for ScaleMeasurementWrapper {
    fn from(item: ScaleMeasurement) -> Self {
        Self {
            id: item.id,
            datetime: item.datetime.into(),
            mass: item.mass,
            fat_pct: item.fat_pct,
            water_pct: item.water_pct,
            muscle_pct: item.muscle_pct,
            bone_pct: item.bone_pct,
        }
    }
}

impl From<ScaleMeasurementWrapper> for ScaleMeasurement {
    fn from(item: ScaleMeasurementWrapper) -> Self {
        Self {
            id: item.id,
            datetime: item.datetime.into(),
            mass: item.mass,
            fat_pct: item.fat_pct,
            water_pct: item.water_pct,
            muscle_pct: item.muscle_pct,
            bone_pct: item.bone_pct,
        }
    }
}

#[derive(Debug, Serialize, rweb::Schema)]
pub struct FitbitBodyWeightFatUpdateOutputWrapper {
    pub measurements: Vec<ScaleMeasurementWrapper>,
    pub activities: Vec<DateTimeWrapper>,
    pub duplicates: Vec<StackString>,
}

impl From<FitbitBodyWeightFatUpdateOutput> for FitbitBodyWeightFatUpdateOutputWrapper {
    fn from(item: FitbitBodyWeightFatUpdateOutput) -> Self {
        Self {
            measurements: item.measurements.into_iter().map(Into::into).collect(),
            activities: item.activities.into_iter().map(Into::into).collect(),
            duplicates: item.duplicates,
        }
    }
}

impl From<FitbitBodyWeightFatUpdateOutputWrapper> for FitbitBodyWeightFatUpdateOutput {
    fn from(item: FitbitBodyWeightFatUpdateOutputWrapper) -> Self {
        Self {
            measurements: item.measurements.into_iter().map(Into::into).collect(),
            activities: item.activities.into_iter().map(Into::into).collect(),
            duplicates: item.duplicates,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, rweb::Schema)]
pub struct FitbitActivityWrapper {
    pub log_type: StackString,
    pub start_time: DateTimeWrapper,
    pub tcx_link: Option<StackString>,
    pub activity_type_id: Option<i64>,
    pub activity_name: Option<StackString>,
    pub duration: i64,
    pub distance: Option<f64>,
    pub distance_unit: Option<StackString>,
    pub steps: Option<i64>,
    pub log_id: i64,
}

impl From<FitbitActivity> for FitbitActivityWrapper {
    fn from(item: FitbitActivity) -> Self {
        Self {
            log_type: item.log_type,
            start_time: item.start_time.into(),
            tcx_link: item.tcx_link,
            activity_type_id: item.activity_type_id,
            activity_name: item.activity_name,
            duration: item.duration,
            distance: item.distance,
            distance_unit: item.distance_unit,
            steps: item.steps,
            log_id: item.log_id,
        }
    }
}

impl From<FitbitActivityWrapper> for FitbitActivity {
    fn from(item: FitbitActivityWrapper) -> Self {
        Self {
            log_type: item.log_type,
            start_time: item.start_time.into(),
            tcx_link: item.tcx_link,
            activity_type_id: item.activity_type_id,
            activity_name: item.activity_name,
            duration: item.duration,
            distance: item.distance,
            distance_unit: item.distance_unit,
            steps: item.steps,
            log_id: item.log_id,
        }
    }
}

#[derive(Serialize, Deserialize, rweb::Schema)]
pub struct StravaAthleteWrapper {
    pub id: u64,
    pub username: StackString,
    pub firstname: StackString,
    pub lastname: StackString,
    pub city: StackString,
    pub state: StackString,
    pub sex: StackString,
}

impl From<StravaAthlete> for StravaAthleteWrapper {
    fn from(item: StravaAthlete) -> Self {
        Self {
            id: item.id,
            username: item.username,
            firstname: item.firstname,
            lastname: item.lastname,
            city: item.city,
            state: item.state,
            sex: item.sex,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, rweb::Schema)]
pub struct FitbitUserProfileWrapper {
    pub average_daily_steps: u64,
    pub country: StackString,
    pub date_of_birth: StackString,
    pub display_name: StackString,
    pub distance_unit: StackString,
    pub encoded_id: StackString,
    pub first_name: StackString,
    pub last_name: StackString,
    pub full_name: StackString,
    pub gender: StackString,
    pub height: f64,
    pub height_unit: StackString,
    pub timezone: StackString,
    pub offset_from_utc_millis: i64,
    pub stride_length_running: f64,
    pub stride_length_walking: f64,
    pub weight: f64,
    pub weight_unit: StackString,
}

impl From<FitbitUserProfile> for FitbitUserProfileWrapper {
    fn from(item: FitbitUserProfile) -> Self {
        Self {
            average_daily_steps: item.average_daily_steps,
            country: item.country,
            date_of_birth: item.date_of_birth,
            display_name: item.display_name,
            distance_unit: item.distance_unit,
            encoded_id: item.encoded_id,
            first_name: item.first_name,
            last_name: item.last_name,
            full_name: item.full_name,
            gender: item.gender,
            height: item.height,
            height_unit: item.height_unit,
            timezone: item.timezone,
            offset_from_utc_millis: item.offset_from_utc_millis,
            stride_length_running: item.stride_length_running,
            stride_length_walking: item.stride_length_walking,
            weight: item.weight,
            weight_unit: item.weight_unit,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, rweb::Schema)]
pub struct GarminConnectActivityWrapper {
    pub activity_id: i64,
    pub activity_name: Option<StackString>,
    pub description: Option<StackString>,
    pub start_time_gmt: DateTimeWrapper,
    pub distance: Option<f64>,
    pub duration: f64,
    pub elapsed_duration: Option<f64>,
    pub moving_duration: Option<f64>,
    pub steps: Option<i64>,
    pub calories: Option<f64>,
    pub average_hr: Option<f64>,
    pub max_hr: Option<f64>,
}

impl From<GarminConnectActivity> for GarminConnectActivityWrapper {
    fn from(item: GarminConnectActivity) -> Self {
        Self {
            activity_id: item.activity_id,
            activity_name: item.activity_name,
            description: item.description,
            start_time_gmt: item.start_time_gmt.into(),
            distance: item.distance,
            duration: item.duration,
            elapsed_duration: item.elapsed_duration,
            moving_duration: item.moving_duration,
            steps: item.steps,
            calories: item.calories,
            average_hr: item.average_hr,
            max_hr: item.max_hr,
        }
    }
}

impl From<GarminConnectActivityWrapper> for GarminConnectActivity {
    fn from(item: GarminConnectActivityWrapper) -> Self {
        Self {
            activity_id: item.activity_id,
            activity_name: item.activity_name,
            description: item.description,
            start_time_gmt: item.start_time_gmt.into(),
            distance: item.distance,
            duration: item.duration,
            elapsed_duration: item.elapsed_duration,
            moving_duration: item.moving_duration,
            steps: item.steps,
            calories: item.calories,
            average_hr: item.average_hr,
            max_hr: item.max_hr,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, rweb::Schema)]
pub struct GarminConnectUserDailySummaryWrapper {
    pub user_profile_id: u64,
    pub total_kilocalories: Option<f64>,
    pub active_kilocalories: Option<f64>,
    pub bmr_kilocalories: Option<f64>,
    pub total_steps: Option<u64>,
    pub total_distance_meters: Option<u64>,
    pub user_daily_summary_id: Option<u64>,
    pub calendar_date: NaiveDateWrapper,
}

impl From<GarminConnectUserDailySummary> for GarminConnectUserDailySummaryWrapper {
    fn from(item: GarminConnectUserDailySummary) -> Self {
        Self {
            user_profile_id: item.user_profile_id,
            total_kilocalories: item.total_kilocalories,
            active_kilocalories: item.active_kilocalories,
            bmr_kilocalories: item.bmr_kilocalories,
            total_steps: item.total_steps,
            total_distance_meters: item.total_distance_meters,
            user_daily_summary_id: item.user_daily_summary_id,
            calendar_date: item.calendar_date.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, rweb::Schema)]
pub struct FitbitStatisticsSummaryWrapper {
    pub date: NaiveDateWrapper,
    pub min_heartrate: f64,
    pub max_heartrate: f64,
    pub mean_heartrate: f64,
    pub median_heartrate: f64,
    pub stdev_heartrate: f64,
    pub number_of_entries: i32,
}

impl From<FitbitStatisticsSummary> for FitbitStatisticsSummaryWrapper {
    fn from(item: FitbitStatisticsSummary) -> Self {
        Self {
            date: item.date.into(),
            min_heartrate: item.min_heartrate,
            max_heartrate: item.max_heartrate,
            mean_heartrate: item.mean_heartrate,
            median_heartrate: item.median_heartrate,
            stdev_heartrate: item.stdev_heartrate,
            number_of_entries: item.number_of_entries,
        }
    }
}

impl From<FitbitStatisticsSummaryWrapper> for FitbitStatisticsSummary {
    fn from(item: FitbitStatisticsSummaryWrapper) -> Self {
        Self {
            date: item.date.into(),
            min_heartrate: item.min_heartrate,
            max_heartrate: item.max_heartrate,
            mean_heartrate: item.mean_heartrate,
            median_heartrate: item.median_heartrate,
            stdev_heartrate: item.stdev_heartrate,
            number_of_entries: item.number_of_entries,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize, rweb::Schema)]
pub enum RaceTypeWrapper {
    #[serde(rename = "personal")]
    Personal,
    #[serde(rename = "world_record_men")]
    WorldRecordMen,
    #[serde(rename = "world_record_women")]
    WorldRecordWomen,
}

impl From<RaceType> for RaceTypeWrapper {
    fn from(item: RaceType) -> Self {
        match item {
            RaceType::Personal => Self::Personal,
            RaceType::WorldRecordMen => Self::WorldRecordMen,
            RaceType::WorldRecordWomen => Self::WorldRecordWomen,
        }
    }
}

impl From<RaceTypeWrapper> for RaceType {
    fn from(item: RaceTypeWrapper) -> Self {
        match item {
            RaceTypeWrapper::Personal => Self::Personal,
            RaceTypeWrapper::WorldRecordMen => Self::WorldRecordMen,
            RaceTypeWrapper::WorldRecordWomen => Self::WorldRecordWomen,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, rweb::Schema)]
pub struct RaceResultsWrapper {
    pub id: i32,
    pub race_type: RaceTypeWrapper,
    pub race_date: Option<NaiveDateWrapper>,
    pub race_name: Option<StackString>,
    pub race_distance: i32, // distance in meters
    pub race_time: f64,
    pub race_flag: bool,
    pub race_summary_ids: Vec<Option<i32>>,
}

impl From<RaceResults> for RaceResultsWrapper {
    fn from(item: RaceResults) -> Self {
        Self {
            id: item.id,
            race_type: item.race_type.into(),
            race_date: item.race_date.map(Into::into),
            race_name: item.race_name,
            race_distance: item.race_distance,
            race_time: item.race_time,
            race_flag: item.race_flag,
            race_summary_ids: item.race_summary_ids,
        }
    }
}

impl From<RaceResultsWrapper> for RaceResults {
    fn from(item: RaceResultsWrapper) -> Self {
        Self {
            id: item.id,
            race_type: item.race_type.into(),
            race_date: item.race_date.map(Into::into),
            race_name: item.race_name,
            race_distance: item.race_distance,
            race_time: item.race_time,
            race_flag: item.race_flag,
            race_summary_ids: item.race_summary_ids,
        }
    }
}

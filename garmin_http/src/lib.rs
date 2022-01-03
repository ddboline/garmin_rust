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
#![allow(clippy::unused_async)]

pub mod errors;
pub mod garmin_requests;
pub mod garmin_rust_app;
pub mod garmin_rust_routes;
pub mod logged_user;
pub mod sport_types_wrapper;

use chrono::{DateTime, NaiveDate, Utc};
use derive_more::{From, Into};
use rweb::{
    openapi::{self, ComponentDescriptor, ComponentOrInlineSchema, Entity},
    Schema,
};
use rweb_helper::derive_rweb_schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{borrow::Cow, collections::HashMap};

use fitbit_lib::{
    fitbit_client::FitbitBodyWeightFatUpdateOutput,
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

use crate::sport_types_wrapper::SportTypesWrapper;

#[derive(Into, From, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
pub struct StravaTimeZoneWrapper(StravaTimeZone);

impl Entity for StravaTimeZoneWrapper {
    fn type_name() -> Cow<'static, str> {
        "timezone".into()
    }
    fn describe(_: &mut ComponentDescriptor) -> ComponentOrInlineSchema {
        use rweb::openapi::Schema;
        ComponentOrInlineSchema::Inline(Schema {
            schema_type: Some(openapi::Type::String),
            format: "timezone".into(),
            ..Schema::default()
        })
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Into, From)]
pub struct FitbitHeartRateWrapper(FitbitHeartRate);

derive_rweb_schema!(FitbitHeartRateWrapper, _FitbitHeartRateWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _FitbitHeartRateWrapper {
    #[schema(description = "DateTime")]
    datetime: DateTime<Utc>,
    #[schema(description = "Heartrate Value (bpm)")]
    value: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Into, From)]
pub struct StravaActivityWrapper(StravaActivity);

derive_rweb_schema!(StravaActivityWrapper, _StravaActivityWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _StravaActivityWrapper {
    #[schema(description = "Activity Name")]
    name: StackString,
    #[schema(description = "Start Date")]
    start_date: DateTime<Utc>,
    #[schema(description = "Activity ID")]
    id: i64,
    #[schema(description = "Distance (m)")]
    distance: Option<f64>,
    #[schema(description = "Moving Time (s)")]
    moving_time: Option<i64>,
    #[schema(description = "Elapsed Time (s)")]
    elapsed_time: i64,
    #[schema(description = "Total Elevation Gain (m)")]
    total_elevation_gain: Option<f64>,
    #[schema(description = "Maximum Elevation")]
    elev_high: Option<f64>,
    #[schema(description = "Minimum Elevation")]
    elev_low: Option<f64>,
    #[schema(description = "Activity Type")]
    activity_type: SportTypesWrapper,
    #[schema(description = "Time Zone")]
    timezone: StravaTimeZoneWrapper,
}

#[derive(Serialize, Deserialize, Debug, Into, From)]
pub struct FitbitBodyWeightFatWrapper(FitbitBodyWeightFat);

derive_rweb_schema!(FitbitBodyWeightFatWrapper, _FitbitBodyWeightFatWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _FitbitBodyWeightFatWrapper {
    #[schema(description = "DateTime")]
    datetime: DateTime<Utc>,
    #[schema(description = "Weight (lbs)")]
    weight: f64,
    #[schema(description = "Fat %")]
    fat: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq, Into, From)]
pub struct ScaleMeasurementWrapper(ScaleMeasurement);

derive_rweb_schema!(ScaleMeasurementWrapper, _ScaleMeasurementWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _ScaleMeasurementWrapper {
    #[schema(description = "Scale Measurement ID")]
    id: i32,
    #[schema(description = "DateTime")]
    datetime: DateTime<Utc>,
    #[schema(description = "Mass (lbs)")]
    mass: f64,
    #[schema(description = "Fat %")]
    fat_pct: f64,
    #[schema(description = "Water %")]
    water_pct: f64,
    #[schema(description = "Muscle %")]
    muscle_pct: f64,
    #[schema(description = "Bone %")]
    bone_pct: f64,
}

#[derive(Debug, Serialize, Into, From)]
pub struct FitbitBodyWeightFatUpdateOutputWrapper(FitbitBodyWeightFatUpdateOutput);

derive_rweb_schema!(
    FitbitBodyWeightFatUpdateOutputWrapper,
    _FitbitBodyWeightFatUpdateOutputWrapper
);

#[derive(Debug, Serialize, Schema)]
struct _FitbitBodyWeightFatUpdateOutputWrapper {
    #[schema(description = "Measurements")]
    measurements: Vec<ScaleMeasurementWrapper>,
    #[schema(description = "Activity DateTimes")]
    activities: Vec<DateTime<Utc>>,
    #[schema(description = "Duplicate Messages")]
    duplicates: Vec<StackString>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Into, From)]
pub struct FitbitActivityWrapper(FitbitActivity);

derive_rweb_schema!(FitbitActivityWrapper, _FitbitActivityWrapper);

#[derive(Serialize, Deserialize, Clone, Debug, Schema)]
struct _FitbitActivityWrapper {
    #[schema(description = "Log Type")]
    log_type: StackString,
    #[schema(description = "Start Datetime")]
    start_time: DateTime<Utc>,
    #[schema(description = "TCX Link")]
    tcx_link: Option<StackString>,
    #[schema(description = "Activity Type ID")]
    activity_type_id: Option<i64>,
    #[schema(description = "Activity Name")]
    activity_name: Option<StackString>,
    #[schema(description = "Duration (ms)")]
    duration: i64,
    #[schema(description = "Distance (mi)")]
    distance: Option<f64>,
    #[schema(description = "Distance Unit")]
    distance_unit: Option<StackString>,
    #[schema(description = "Number of Steps")]
    steps: Option<i64>,
    #[schema(description = "Log ID")]
    log_id: i64,
}

#[derive(Serialize, Deserialize, Debug, Into, From)]
pub struct GarminConnectActivityWrapper(GarminConnectActivity);

derive_rweb_schema!(GarminConnectActivityWrapper, _GarminConnectActivityWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _GarminConnectActivityWrapper {
    #[schema(description = "Activity ID")]
    activity_id: i64,
    #[schema(description = "Activity Name")]
    activity_name: Option<StackString>,
    #[schema(description = "Description")]
    description: Option<StackString>,
    #[schema(description = "Start Time UTC")]
    start_time_gmt: DateTime<Utc>,
    #[schema(description = "Distance (m)")]
    distance: Option<f64>,
    #[schema(description = "Duration (s)")]
    duration: f64,
    #[schema(description = "Elapsed Duration (s)")]
    elapsed_duration: Option<f64>,
    #[schema(description = "Moving Duration (s)")]
    moving_duration: Option<f64>,
    #[schema(description = "Number of Steps")]
    steps: Option<i64>,
    #[schema(description = "Calories (kCal)")]
    calories: Option<f64>,
    #[schema(description = "Average Heartrate")]
    average_hr: Option<f64>,
    #[schema(description = "Max Heartrate")]
    max_hr: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Into, From)]
pub struct GarminConnectUserDailySummaryWrapper(GarminConnectUserDailySummary);

derive_rweb_schema!(
    GarminConnectUserDailySummaryWrapper,
    _GarminConnectUserDailySummaryWrapper
);

#[allow(dead_code)]
#[derive(Schema)]
struct _GarminConnectUserDailySummaryWrapper {
    #[schema(description = "User Profile ID")]
    user_profile_id: u64,
    #[schema(description = "Total Calories (kCal)")]
    total_kilocalories: Option<f64>,
    #[schema(description = "Active Calories (kCal)")]
    active_kilocalories: Option<f64>,
    #[schema(description = "BMR Calories (kCal)")]
    bmr_kilocalories: Option<f64>,
    #[schema(description = "Total Number of Steps")]
    total_steps: Option<u64>,
    #[schema(description = "Total Distance (m)")]
    total_distance_meters: Option<u64>,
    #[schema(description = "User Daily Summary ID")]
    user_daily_summary_id: Option<u64>,
    #[schema(description = "Calendar Date")]
    calendar_date: NaiveDate,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Into, From)]
pub struct FitbitStatisticsSummaryWrapper(FitbitStatisticsSummary);

derive_rweb_schema!(
    FitbitStatisticsSummaryWrapper,
    _FitbitStatisticsSummaryWrapper
);

#[allow(dead_code)]
#[derive(Schema)]
struct _FitbitStatisticsSummaryWrapper {
    #[schema(description = "Date")]
    date: NaiveDate,
    #[schema(description = "Minimum Heartrate")]
    min_heartrate: f64,
    #[schema(description = "Maximum Heartrate")]
    max_heartrate: f64,
    #[schema(description = "Mean Heartrate")]
    mean_heartrate: f64,
    #[schema(description = "Median Heartrate")]
    median_heartrate: f64,
    #[schema(description = "Heartrate Standard Deviation")]
    stdev_heartrate: f64,
    #[schema(description = "Number of Entries")]
    number_of_entries: i32,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize, Into, From)]
pub struct RaceTypeWrapper(RaceType);

derive_rweb_schema!(RaceTypeWrapper, _RaceTypeWrapper);

#[allow(dead_code)]
#[derive(Serialize, Schema)]
enum _RaceTypeWrapper {
    #[serde(rename = "personal")]
    Personal,
    #[serde(rename = "world_record_men")]
    WorldRecordMen,
    #[serde(rename = "world_record_women")]
    WorldRecordWomen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Into, From)]
pub struct RaceResultsWrapper(RaceResults);

derive_rweb_schema!(RaceResultsWrapper, _RaceResultsWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _RaceResultsWrapper {
    #[schema(description = "Race Result ID")]
    id: i32,
    #[schema(description = "Race Type")]
    race_type: RaceTypeWrapper,
    #[schema(description = "Race Date")]
    race_date: Option<NaiveDate>,
    #[schema(description = "Race Name")]
    race_name: Option<StackString>,
    #[schema(description = "Race Distance (m)")]
    race_distance: i32, // distance in meters
    #[schema(description = "Race Duration (s)")]
    race_time: f64,
    #[schema(description = "Race Flag")]
    race_flag: bool,
    #[schema(description = "Race Summary IDs")]
    race_summary_ids: Vec<Option<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Into, From)]
pub struct FitbitActivityTypesWrapper(HashMap<StackString, StackString>);

derive_rweb_schema!(FitbitActivityTypesWrapper, _FitbitActivityTypesWrapper);

#[allow(dead_code)]
#[derive(Schema)]
struct _FitbitActivityTypesWrapper(HashMap<String, StackString>);

#[cfg(test)]
mod test {
    use rweb_helper::derive_rweb_test;

    use crate::{
        FitbitActivityWrapper, FitbitBodyWeightFatUpdateOutputWrapper, FitbitBodyWeightFatWrapper,
        FitbitHeartRateWrapper, FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper,
        GarminConnectUserDailySummaryWrapper, RaceResultsWrapper, RaceTypeWrapper,
        ScaleMeasurementWrapper, StravaActivityWrapper, _FitbitActivityWrapper,
        _FitbitBodyWeightFatUpdateOutputWrapper, _FitbitBodyWeightFatWrapper,
        _FitbitHeartRateWrapper, _FitbitStatisticsSummaryWrapper, _GarminConnectActivityWrapper,
        _GarminConnectUserDailySummaryWrapper, _RaceResultsWrapper, _RaceTypeWrapper,
        _ScaleMeasurementWrapper, _StravaActivityWrapper,
    };

    #[test]
    fn test_types() {
        derive_rweb_test!(FitbitHeartRateWrapper, _FitbitHeartRateWrapper);
        derive_rweb_test!(StravaActivityWrapper, _StravaActivityWrapper);
        derive_rweb_test!(FitbitBodyWeightFatWrapper, _FitbitBodyWeightFatWrapper);
        derive_rweb_test!(ScaleMeasurementWrapper, _ScaleMeasurementWrapper);
        derive_rweb_test!(
            FitbitBodyWeightFatUpdateOutputWrapper,
            _FitbitBodyWeightFatUpdateOutputWrapper
        );
        derive_rweb_test!(FitbitActivityWrapper, _FitbitActivityWrapper);
        derive_rweb_test!(GarminConnectActivityWrapper, _GarminConnectActivityWrapper);
        derive_rweb_test!(
            GarminConnectUserDailySummaryWrapper,
            _GarminConnectUserDailySummaryWrapper
        );
        derive_rweb_test!(
            FitbitStatisticsSummaryWrapper,
            _FitbitStatisticsSummaryWrapper
        );
        derive_rweb_test!(RaceTypeWrapper, _RaceTypeWrapper);
        derive_rweb_test!(RaceResultsWrapper, _RaceResultsWrapper);
    }
}

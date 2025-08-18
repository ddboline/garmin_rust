// #![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::unused_async)]
#![allow(clippy::unsafe_derive_deserialize)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::needless_for_each)]

pub mod errors;
pub mod garmin_elements;
pub mod garmin_file_report_html;
pub mod garmin_requests;
pub mod garmin_rust_app;
pub mod garmin_rust_routes;
pub mod logged_user;
pub mod sport_types_wrapper;

use derive_more::{From, Into};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::collections::HashMap;
use time::{Date, OffsetDateTime};
use utoipa::{
    openapi::{ObjectBuilder, Type},
    PartialSchema, ToSchema,
};
use utoipa_helper::derive_utoipa_schema;
use uuid::Uuid;

use fitbit_lib::{
    fitbit_heartrate::{FitbitBodyWeightFat, FitbitHeartRate},
    fitbit_statistics_summary::FitbitStatisticsSummary,
    scale_measurement::ScaleMeasurement,
};
use garmin_lib::strava_timezone::StravaTimeZone;
use garmin_models::{
    fitbit_activity::FitbitActivity, garmin_connect_activity::GarminConnectActivity,
    strava_activity::StravaActivity,
};
use race_result_analysis::{race_results::RaceResults, race_type::RaceType};

use crate::sport_types_wrapper::SportTypesWrapper;

#[derive(Into, From, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
pub struct StravaTimeZoneWrapper(StravaTimeZone);

impl PartialSchema for StravaTimeZoneWrapper {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        ObjectBuilder::new()
            .format(Some(utoipa::openapi::SchemaFormat::Custom(
                "timezone".into(),
            )))
            .schema_type(Type::String)
            .build()
            .into()
    }
}

impl ToSchema for StravaTimeZoneWrapper {
    fn name() -> std::borrow::Cow<'static, str> {
        "timezone".into()
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Into, From, Eq)]
pub struct FitbitHeartRateWrapper(FitbitHeartRate);

derive_utoipa_schema!(FitbitHeartRateWrapper, _FitbitHeartRateWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = FitbitHeartRate)]
// FitbitHeartrate
struct _FitbitHeartRateWrapper {
    // DateTime
    datetime: OffsetDateTime,
    // Heartrate Value (bpm)
    value: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Into, From)]
pub struct StravaActivityWrapper(StravaActivity);

derive_utoipa_schema!(StravaActivityWrapper, _StravaActivityWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = StravaActivity)]
// StravaActivity
struct _StravaActivityWrapper {
    // Activity Name
    #[schema(inline)]
    name: StackString,
    // Start Date
    start_date: OffsetDateTime,
    // Activity ID
    id: i64,
    // Distance (m)
    distance: Option<f64>,
    // Moving Time (s)
    moving_time: Option<i64>,
    // Elapsed Time (s)
    elapsed_time: i64,
    // Total Elevation Gain (m)
    total_elevation_gain: Option<f64>,
    // Maximum Elevation
    elev_high: Option<f64>,
    // Minimum Elevation
    elev_low: Option<f64>,
    // Activity Type
    activity_type: SportTypesWrapper,
    // Time Zone
    timezone: StravaTimeZoneWrapper,
}

#[derive(Serialize, Deserialize, Debug, Into, From)]
pub struct FitbitBodyWeightFatWrapper(FitbitBodyWeightFat);

derive_utoipa_schema!(FitbitBodyWeightFatWrapper, _FitbitBodyWeightFatWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = FitbitBodyWeightFat)]
// FitbitBodyWeightFat
struct _FitbitBodyWeightFatWrapper {
    // DateTime
    datetime: OffsetDateTime,
    // Weight (lbs)
    weight: f64,
    // Fat %
    fat: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq, Into, From)]
pub struct ScaleMeasurementWrapper(ScaleMeasurement);

derive_utoipa_schema!(ScaleMeasurementWrapper, _ScaleMeasurementWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = ScaleMeasurement)]
// ScaleMeasurement
struct _ScaleMeasurementWrapper {
    // Scale Measurement ID
    id: Uuid,
    // DateTime
    datetime: OffsetDateTime,
    // Mass (lbs)
    mass: f64,
    // Fat %
    fat_pct: f64,
    // Water %
    water_pct: f64,
    // Muscle %
    muscle_pct: f64,
    // Bone %
    bone_pct: f64,
    // Connect Primary Key
    connect_primary_key: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Into, From)]
pub struct FitbitActivityWrapper(FitbitActivity);

derive_utoipa_schema!(FitbitActivityWrapper, _FitbitActivityWrapper);

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
// FitbitActivity
#[schema(as = FitbitActivity)]
struct _FitbitActivityWrapper {
    // Log Type
    #[schema(inline)]
    log_type: StackString,
    // Start Datetime
    start_time: OffsetDateTime,
    // TCX Link
    #[schema(inline)]
    tcx_link: Option<StackString>,
    // Activity Type ID
    activity_type_id: Option<i64>,
    // Activity Name
    #[schema(inline)]
    activity_name: Option<StackString>,
    // Duration (ms)
    duration: i64,
    // Distance (mi)
    distance: Option<f64>,
    // Distance Unit
    #[schema(inline)]
    distance_unit: Option<StackString>,
    // Number of Steps
    steps: Option<i64>,
    // Log ID
    log_id: i64,
}

#[derive(Serialize, Deserialize, Debug, Into, From)]
pub struct GarminConnectActivityWrapper(GarminConnectActivity);

derive_utoipa_schema!(GarminConnectActivityWrapper, _GarminConnectActivityWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
// GarminConnectActivity
#[schema(as = GarminConnectActivity)]
struct _GarminConnectActivityWrapper {
    // Activity ID
    activity_id: i64,
    // Activity Name
    #[schema(inline)]
    activity_name: Option<StackString>,
    // Description
    #[schema(inline)]
    description: Option<StackString>,
    // Start Time UTC
    start_time_gmt: OffsetDateTime,
    // Distance (m)
    distance: Option<f64>,
    // Duration (s)
    duration: f64,
    // Elapsed Duration (s)
    elapsed_duration: Option<f64>,
    // Moving Duration (s)
    moving_duration: Option<f64>,
    // Number of Steps
    steps: Option<i64>,
    // Calories (kCal)
    calories: Option<f64>,
    // Average Heartrate
    average_hr: Option<f64>,
    // Max Heartrate
    max_hr: Option<f64>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Into, From)]
pub struct FitbitStatisticsSummaryWrapper(FitbitStatisticsSummary);

derive_utoipa_schema!(
    FitbitStatisticsSummaryWrapper,
    _FitbitStatisticsSummaryWrapper
);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = FitbitStatisticsSummary)]
// FitbitStatisticsSummary
struct _FitbitStatisticsSummaryWrapper {
    // Date
    date: Date,
    // Minimum Heartrate
    min_heartrate: f64,
    // Maximum Heartrate
    max_heartrate: f64,
    // Mean Heartrate
    mean_heartrate: f64,
    // Median Heartrate
    median_heartrate: f64,
    // Heartrate Standard Deviation
    stdev_heartrate: f64,
    // Number of Entries
    number_of_entries: i32,
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize, Into, From, Eq)]
pub struct RaceTypeWrapper(RaceType);

derive_utoipa_schema!(RaceTypeWrapper, _RaceTypeWrapper);

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
#[schema(as = RaceType)]
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

derive_utoipa_schema!(RaceResultsWrapper, _RaceResultsWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = RaceResults)]
// RaceResults
struct _RaceResultsWrapper {
    // Race Result ID
    id: Uuid,
    // Race Type
    race_type: RaceTypeWrapper,
    // Race Date
    race_date: Option<Date>,
    // Race Name
    #[schema(inline)]
    race_name: Option<StackString>,
    // Race Distance (m)
    race_distance: i32, // distance in meters
    // Race Duration (s)
    race_time: f64,
    // Race Flag
    race_flag: bool,
    // Race Summary IDs
    race_summary_ids: Vec<Option<Uuid>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Into, From, Eq)]
pub struct FitbitActivityTypesWrapper(HashMap<StackString, StackString>);

derive_utoipa_schema!(FitbitActivityTypesWrapper, _FitbitActivityTypesWrapper);

#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(as = FitbitActivityTypes)]
struct _FitbitActivityTypesWrapper(HashMap<String, StackString>);

#[cfg(test)]
mod test {
    use utoipa_helper::derive_utoipa_test;

    use crate::{
        FitbitActivityWrapper, FitbitBodyWeightFatWrapper, FitbitHeartRateWrapper,
        FitbitStatisticsSummaryWrapper, GarminConnectActivityWrapper, RaceResultsWrapper,
        RaceTypeWrapper, ScaleMeasurementWrapper, StravaActivityWrapper, _FitbitActivityWrapper,
        _FitbitBodyWeightFatWrapper, _FitbitHeartRateWrapper, _FitbitStatisticsSummaryWrapper,
        _GarminConnectActivityWrapper, _RaceResultsWrapper, _RaceTypeWrapper,
        _ScaleMeasurementWrapper, _StravaActivityWrapper,
    };

    #[test]
    fn test_types() {
        derive_utoipa_test!(FitbitHeartRateWrapper, _FitbitHeartRateWrapper);
        derive_utoipa_test!(StravaActivityWrapper, _StravaActivityWrapper);
        derive_utoipa_test!(FitbitBodyWeightFatWrapper, _FitbitBodyWeightFatWrapper);
        derive_utoipa_test!(ScaleMeasurementWrapper, _ScaleMeasurementWrapper);
        derive_utoipa_test!(FitbitActivityWrapper, _FitbitActivityWrapper);
        derive_utoipa_test!(GarminConnectActivityWrapper, _GarminConnectActivityWrapper);
        derive_utoipa_test!(
            FitbitStatisticsSummaryWrapper,
            _FitbitStatisticsSummaryWrapper
        );
        derive_utoipa_test!(RaceTypeWrapper, _RaceTypeWrapper);
        derive_utoipa_test!(RaceResultsWrapper, _RaceResultsWrapper);
    }
}

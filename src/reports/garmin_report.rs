extern crate rayon;

use failure::Error;
use std::fs::create_dir_all;

use rayon::prelude::*;

use crate::garmin_file::GarminFile;
use crate::garmin_lap::GarminLap;
use crate::garmin_sync::{get_s3_client, upload_file_acl};
use crate::garmin_util::{
    get_sport_type_string_map, plot_graph, print_h_m_s, titlecase, PlotOpts, MARATHON_DISTANCE_MI,
    METERS_PER_MILE, MONTH_NAMES,
};
use crate::reports::garmin_file_report_txt::get_splits;
use crate::reports::garmin_report_options::GarminReportOptions;
use crate::reports::garmin_templates::{GARMIN_TEMPLATE, MAP_TEMPLATE};

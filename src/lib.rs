#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate log;

#[macro_use]
extern crate postgres;
extern crate postgres_derive;

pub mod garmin_cli;
pub mod garmin_config;
pub mod garmin_correction_lap;
pub mod garmin_file;
pub mod garmin_lap;
pub mod garmin_point;
pub mod garmin_summary;
pub mod garmin_sync;
pub mod parsers;
pub mod reports;
pub mod utils;

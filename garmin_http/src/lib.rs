#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

pub mod errors;
pub mod garmin_requests;
pub mod garmin_rust_app;
pub mod garmin_rust_routes;
pub mod logged_user;

use garmin_lib::common::garmin_config::GarminConfig;

lazy_static! {
    static ref CONFIG: GarminConfig = GarminConfig::get_config(None).unwrap();
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

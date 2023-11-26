pub mod fitbit_activity;
pub mod garmin_correction_lap;
pub mod strava_activity;
pub mod garmin_connect_activity;
pub mod garmin_summary;
pub mod garmin_sync;
pub mod garmin_file;
pub mod garmin_lap;
pub mod garmin_point;


pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

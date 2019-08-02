use std::collections::HashMap;

use fitbit_lib::scale_measurement::ScaleMeasurement;
use fitbit_lib::sheets_client::SheetsClient;
use garmin_lib::common::garmin_config::GarminConfig;
use garmin_lib::common::pgpool::PgPool;

fn main() {
    let config = GarminConfig::get_config(None).unwrap();
    let pool = PgPool::new(&config.pgurl);
    let current_measurements: HashMap<_, _> = ScaleMeasurement::read_from_db(&pool)
        .unwrap()
        .into_iter()
        .map(|meas| (meas.datetime, meas))
        .collect();

    let c = SheetsClient::new(&config, "ddboline@gmail.com");
    let (_, sheets) = c
        .gsheets
        .spreadsheets()
        .get("1MG8so2pFKoOIpt0Vo9pUAtoNk-Y1SnHq9DiEFi-m5Uw")
        .include_grid_data(true)
        .doit()
        .unwrap();
    let sheets = sheets.sheets.unwrap();
    let sheet = &sheets[0];
    let data = sheet.data.as_ref().unwrap();
    let row_data = &data[0].row_data.as_ref().unwrap();
    let measurements: Vec<ScaleMeasurement> = row_data[1..]
        .iter()
        .filter_map(|row| ScaleMeasurement::from_row_data(row).ok())
        .collect();
    println!("{} {} {}", data.len(), row_data.len(), measurements.len());
    for meas in measurements {
        if !current_measurements.contains_key(&meas.datetime) {
            println!("insert {:?}", meas);
            meas.insert_into_db(&pool).unwrap();
        } else {
            println!("exists {:?}", meas);
        }
    }
}

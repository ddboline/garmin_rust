#[macro_use]
extern crate serde_derive;

pub mod fitbit_client;
pub mod fitbit_heartrate;
pub mod scale_measurement;
pub mod sheets_client;
pub mod telegram_bot;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

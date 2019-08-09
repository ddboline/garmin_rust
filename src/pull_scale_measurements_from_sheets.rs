use fitbit_lib::sheets_client::run_sync_sheets;

fn main() {
    env_logger::init();
    run_sync_sheets().unwrap();
}

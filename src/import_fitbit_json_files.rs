use fitbit_lib::fitbit_heartrate::import_fitbit_json_files;

fn main() {
    env_logger::init();
    import_fitbit_json_files("/home/ddboline/Downloads/tmp/DanielBoline/user-site-export").unwrap();
}

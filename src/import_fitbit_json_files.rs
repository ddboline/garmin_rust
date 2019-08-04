use fitbit_lib::fitbit_heartrate::import_fitbit_json_files;

fn main() {
    import_fitbit_json_files("/home/ddboline/Downloads/tmp/DanielBoline/user-site-export").unwrap();
}

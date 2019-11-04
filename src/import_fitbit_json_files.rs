use structopt::StructOpt;

use fitbit_lib::fitbit_heartrate::{JsonImportOpts, import_fitbit_json_files};

fn main() {
    env_logger::init();
    let opts = JsonImportOpts::from_args();
    import_fitbit_json_files(&opts.directory).unwrap();
}

use structopt::StructOpt;

use fitbit_lib::fitbit_heartrate::{import_fitbit_json_files, JsonImportOpts};

fn main() {
    env_logger::init();
    let opts = JsonImportOpts::from_args();
    import_fitbit_json_files(opts.directory.as_str()).unwrap();
}

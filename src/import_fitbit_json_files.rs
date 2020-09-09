#![type_length_limit = "1059389"]

use stack_string::StackString;
use structopt::StructOpt;

use fitbit_lib::fitbit_heartrate::import_fitbit_json_files;

#[derive(StructOpt, Debug, Clone)]
pub struct JsonImportOpts {
    #[structopt(short = "d", long = "directory")]
    pub directory: StackString,
}

fn main() {
    env_logger::init();
    let opts = JsonImportOpts::from_args();
    import_fitbit_json_files(opts.directory.as_str()).unwrap();
}

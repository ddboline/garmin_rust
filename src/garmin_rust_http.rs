#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;

use rocket::response::content;

use garmin_rust::garmin_cli;

#[get("/garmin?<filter>&<history>")]
fn garmin(filter: Option<String>, history: Option<String>) -> content::Html<String> {
    let filter = filter.unwrap_or("sport".to_string());
    let history = history.unwrap_or("sport".to_string());

    let filter_vec: Vec<String> = filter.split(",").map(|x| x.to_string()).collect();

    let (options, constraints) = garmin_cli::process_pattern(&filter_vec);

    content::Html(garmin_cli::run_html(&options, &constraints, &filter, &history).unwrap())
}

fn main() {
    rocket::ignite().mount("/", routes![garmin]).launch();
}

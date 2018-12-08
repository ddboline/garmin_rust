#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;

use rocket::response::content;

use garmin_rust::garmin_cli;

#[get("/garmin")]
fn garmin_default() -> content::Html<String> {
    let filter = format!("year,run");

    garmin(filter)
}

#[get("/garmin?<filter>")]
fn garmin(filter: String) -> content::Html<String> {
    let filter_vec: Vec<String> = filter.split(",").map(|x| x.to_string()).collect();

    let (options, constraints) = garmin_cli::process_pattern(&filter_vec);

    content::Html(garmin_cli::run_html(&options, &constraints).unwrap())
}

fn main() {
    rocket::ignite()
        .mount("/", routes![garmin_default, garmin])
        .launch();
}

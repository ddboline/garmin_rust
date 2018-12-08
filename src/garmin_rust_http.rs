#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[get("/world")]
fn hello() -> &'static str {
    "How's it going"
}

fn main() {
    rocket::ignite().mount("/", routes![index, hello]).launch();
}

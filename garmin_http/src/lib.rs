#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

pub mod errors;
pub mod garmin_requests;
pub mod garmin_rust_app;
pub mod garmin_rust_routes;
pub mod logged_user;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate postgres;
extern crate postgres_derive;

#[macro_use]
extern crate lazy_static;

pub mod common;
pub mod parsers;
pub mod reports;
pub mod utils;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

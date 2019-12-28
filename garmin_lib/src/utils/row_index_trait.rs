use failure::{format_err, Error};
use postgres::row::{Row, RowIndex};
use postgres::types::FromSql;
use std::fmt::Display;

pub trait RowIndexTrait<'a> {
    fn get_idx<I, T>(&'a self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + Display + Copy,
        T: FromSql<'a>;
}

impl<'a> RowIndexTrait<'a> for Row {
    fn get_idx<I, T>(&'a self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + Display + Copy,
        T: FromSql<'a>,
    {
        self.try_get(idx)
            .map_err(|_| format_err!("Invalid index {}", idx))
    }
}

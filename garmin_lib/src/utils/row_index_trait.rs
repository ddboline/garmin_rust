use failure::{err_msg, format_err, Error};
use postgres::rows::{Row, RowIndex};
use postgres::types::FromSql;
use std::fmt::Debug;

pub trait RowIndexTrait {
    fn get_idx<I, T>(&self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + Debug + Copy,
        T: FromSql;
}

impl<'a> RowIndexTrait for Row<'a> {
    fn get_idx<I, T>(&self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + Debug + Copy,
        T: FromSql,
    {
        self.get_opt(idx)
            .ok_or_else(|| format_err!("Invalid index {:?}", idx))
            .and_then(|x| x.map_err(err_msg))
    }
}

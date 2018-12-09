extern crate chrono;
extern crate num;
extern crate serde_json;

use num::traits::Pow;

use std::io::BufRead;
use std::io::BufReader;
use subprocess::{Exec, Redirection};

use chrono::prelude::*;

use failure::{err_msg, Error};
use std::collections::HashMap;


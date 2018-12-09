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

pub fn plot_graph(opts: &PlotOpts) -> Result<String, Error> {
    let command = format!(
        "garmin-plot-graph -n {} -t {} -x {} -y {} -c {} {} {}",
        opts.name,
        format!("{}{}{}", '"', opts.title, '"'),
        format!("{}{}{}", '"', opts.xlabel, '"'),
        format!("{}{}{}", '"', opts.ylabel, '"'),
        format!("{}{}{}", '"', opts.cache_dir, '"'),
        match &opts.marker {
            Some(m) => format!("-m {0}{1}{0}", '"', m),
            None => "".to_string(),
        },
        match opts.do_scatter {
            true => "-s".to_string(),
            false => "".to_string(),
        }
    );

    debug!("{}", command);

    let input = format!("{}\n", serde_json::to_string(&opts.data)?);

    let mut popen = Exec::shell(&command)
        .stdin(Redirection::Pipe)
        .stdout(Redirection::Pipe)
        .popen()?;

    let (result, _) = popen.communicate(Some(&input))?;

    Ok(result.clone().unwrap())
}

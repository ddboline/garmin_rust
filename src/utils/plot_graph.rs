extern crate chrono;
extern crate num;
extern crate serde_json;

use subprocess::{Exec, Redirection};

use failure::{err_msg, Error};

use crate::utils::plot_opts::PlotOpts;

pub fn plot_graph(opts: &PlotOpts) -> Result<String, Error> {
    if opts.data == None {
        return Err(err_msg(format!("No data points {}", opts.name)));
    }
    if let Some(x) = opts.data {
        if x.len() == 0 {
            return Err(err_msg(format!("No data points {}", opts.name)));
        }
    }

    let command = format!(
        "garmin_rust_plot_graph.py -n {} -t {} -x {} -y {} -c {} {} {}",
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

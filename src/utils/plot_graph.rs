extern crate chrono;
extern crate num;
extern crate serde_json;

use std::collections::HashMap;

use failure::{err_msg, Error};

use crate::reports::garmin_templates::{LINEPLOTTEMPLATE, SCATTERPLOTTEMPLATE};
use crate::utils::plot_opts::PlotOpts;

pub fn generate_d3_plot(opts: &PlotOpts) -> Result<String, Error> {
    if opts.data == None {
        return Err(err_msg(format!("No data points {}", opts.name)));
    }
    if let Some(x) = opts.data {
        if x.len() == 0 {
            return Err(err_msg(format!("No data points {}", opts.name)));
        }
    }

    Ok(match opts.do_scatter {
        true => {
            let nbins = 10;
            let xmin = *(&opts
                .data
                .unwrap()
                .iter()
                .map(|(x, _)| x)
                .min_by_key(|&x| (*x * 1000.) as i64)
                .unwrap());
            let xmin = xmin - 0.01 * xmin.abs();
            let xmax = *(&opts
                .data
                .unwrap()
                .iter()
                .map(|(x, _)| x)
                .max_by_key(|&x| (*x * 1000.) as i64)
                .unwrap());
            let xmax = xmax + 0.01 * xmax.abs();
            let xstep = (xmax - xmin) / (nbins as f64);
            let ymin = *(&opts
                .data
                .unwrap()
                .iter()
                .map(|(_, y)| y)
                .min_by_key(|&x| (*x * 1000.) as i64)
                .unwrap());
            let ymin = ymin - 0.01 * ymin.abs();
            let ymax = *(&opts
                .data
                .unwrap()
                .iter()
                .map(|(_, y)| y)
                .max_by_key(|&x| (*x * 1000.) as i64)
                .unwrap());
            let ymax = ymax + 0.01 * ymax.abs();
            let ystep = (ymax - ymin) / (nbins as f64);

            let mut bins: HashMap<(u64, u64), u64> = HashMap::new();
            for xbin in 0..nbins {
                for ybin in 0..nbins {
                    bins.insert((xbin, ybin), 0);
                }
            }

            for (x, y) in opts.data.unwrap() {
                let xindex = ((x - xmin) / xstep) as u64;
                let yindex = ((y - ymin) / ystep) as u64;
                match bins.get_mut(&(xindex, yindex)) {
                    Some(x) => *x += 1,
                    None => println!(
                        "missing {} {} {} {} {} {} {} {}",
                        xindex, yindex, x, y, xmin, ymin, xmax, ymax
                    ),
                }
            }

            let data: Vec<_> = bins
                .iter()
                .map(|((xb, yb), c)| (*xb as f64 * xstep + xmin, *yb as f64 * ystep + ymin, c))
                .collect();

            SCATTERPLOTTEMPLATE
                .split("\n")
                .map(|line| {
                    if line.contains("EXAMPLETITLE") {
                        line.replace("EXAMPLETITLE", &opts.title)
                    } else if line.contains("XSTEP") {
                        line.replace("XSTEP", &xstep.to_string())
                    } else if line.contains("YSTEP") {
                        line.replace("YSTEP", &ystep.to_string())
                    } else if line.contains("DATA") {
                        line.replace("DATA", &serde_json::to_string(&data).unwrap())
                    } else if line.contains("XLABEL") {
                        line.replace("XLABEL", &opts.xlabel)
                    } else if line.contains("YLABEL") {
                        line.replace("YLABEL", &opts.ylabel)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        false => LINEPLOTTEMPLATE
            .split("\n")
            .map(|line| {
                if line.contains("EXAMPLETITLE") {
                    line.replace("EXAMPLETITLE", &opts.title)
                } else if line.contains("XAXIS") {
                    line.replace("XAXIS", &opts.xlabel)
                } else if line.contains("YAXIS") {
                    line.replace("YAXIS", &opts.ylabel)
                } else if line.contains("DATA") {
                    line.replace("DATA", &serde_json::to_string(&opts.data).unwrap())
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

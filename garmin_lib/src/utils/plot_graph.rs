use anyhow::{format_err, Error};
use log::debug;
use stack_string::StackString;
use std::collections::HashMap;

use crate::{
    reports::garmin_templates::{LINEPLOTTEMPLATE, SCATTERPLOTTEMPLATE},
    utils::plot_opts::PlotOpts,
};

#[allow(clippy::similar_names)]
pub fn generate_d3_plot(opts: &PlotOpts) -> Result<StackString, Error> {
    let err_str = format!("No data points {}", opts.name);

    let data = match opts.data.as_ref() {
        Some(x) => {
            if x.is_empty() {
                return Err(format_err!(err_str));
            } else {
                x
            }
        }
        None => return Err(format_err!(err_str)),
    };

    let body = if opts.do_scatter {
        let nbins = 10;
        let xmin = data
            .iter()
            .map(|(x, _)| x)
            .min_by_key(|&x| (*x * 1000.) as i64)
            .cloned()
            .unwrap_or(0.0);
        let xmin = xmin - 0.01 * xmin.abs();
        let xmax = data
            .iter()
            .map(|(x, _)| x)
            .max_by_key(|&x| (*x * 1000.) as i64)
            .cloned()
            .unwrap_or(0.0);
        let xmax = xmax + 0.01 * xmax.abs();
        let xstep = (xmax - xmin) / (nbins as f64);
        let ymin = data
            .iter()
            .map(|(_, y)| y)
            .min_by_key(|&x| (*x * 1000.) as i64)
            .cloned()
            .unwrap_or(0.0);
        let ymin = ymin - 0.01 * ymin.abs();
        let ymax = data
            .iter()
            .map(|(_, y)| y)
            .max_by_key(|&x| (*x * 1000.) as i64)
            .cloned()
            .unwrap_or(0.0);
        let ymax = ymax + 0.01 * ymax.abs();
        let ystep = (ymax - ymin) / (nbins as f64);

        let mut bins: HashMap<(u64, u64), u64> = HashMap::new();
        for xbin in 0..nbins {
            for ybin in 0..nbins {
                bins.insert((xbin, ybin), 0);
            }
        }

        for (x, y) in data.iter() {
            let xindex = ((x - xmin) / xstep) as u64;
            let yindex = ((y - ymin) / ystep) as u64;
            if let Some(x) = bins.get_mut(&(xindex, yindex)) {
                *x += 1;
            } else {
                debug!(
                    "missing {} {} {} {} {} {} {} {}",
                    xindex, yindex, x, y, xmin, ymin, xmax, ymax
                );
            }
        }

        let data: Vec<_> = bins
            .iter()
            .map(|((xb, yb), c)| (*xb as f64 * xstep + xmin, *yb as f64 * ystep + ymin, c))
            .collect();

        let js_str = SCATTERPLOTTEMPLATE
            .split('\n')
            .map(|line| {
                if line.contains("EXAMPLETITLE") {
                    line.replace("EXAMPLETITLE", &opts.title)
                } else if line.contains("XSTEP") {
                    line.replace("XSTEP", &xstep.to_string())
                } else if line.contains("YSTEP") {
                    line.replace("YSTEP", &ystep.to_string())
                } else if line.contains("DATA") {
                    line.replace(
                        "DATA",
                        &serde_json::to_string(&data).unwrap_or_else(|_| "".to_string()),
                    )
                } else if line.contains("XLABEL") {
                    line.replace("XLABEL", &opts.xlabel)
                } else if line.contains("YLABEL") {
                    line.replace("YLABEL", &opts.ylabel)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("<script>\n{}\n</script>", js_str).into()
    } else {
        let js_str = LINEPLOTTEMPLATE
            .split('\n')
            .map(|line| {
                if line.contains("EXAMPLETITLE") {
                    line.replace("EXAMPLETITLE", &opts.title)
                } else if line.contains("XAXIS") {
                    line.replace("XAXIS", &opts.xlabel)
                } else if line.contains("YAXIS") {
                    line.replace("YAXIS", &opts.ylabel)
                } else if line.contains("DATA") {
                    line.replace(
                        "DATA",
                        &serde_json::to_string(&data).unwrap_or_else(|_| "".to_string()),
                    )
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("<script>\n{}\n</script>", js_str).into()
    };
    Ok(body)
}

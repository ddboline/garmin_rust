use log::debug;
use serde::Serialize;
use std::collections::HashMap;

use crate::plot_opts::{DataPoint, PlotOpts};

#[derive(PartialEq, Debug, Serialize)]
pub struct ScatterDataPoint {
    pub x: f64,
    pub y: f64,
    pub c: u64,
}

#[derive(PartialEq, Debug)]
pub struct ScatterPlotData {
    pub data: Vec<ScatterDataPoint>,
    pub xstep: f64,
    pub ystep: f64,
}

/// # Errors
/// Return error if rendering template fails
#[allow(clippy::similar_names)]
#[must_use]
pub fn generate_plot_data(opts: &PlotOpts, data: &[DataPoint]) -> Option<ScatterPlotData> {
    if opts.do_scatter {
        let nbins = 10;
        let xmin = data
            .iter()
            .map(|d| d.x)
            .min_by_key(|x| (x * 1000.) as i64)
            .unwrap_or(0.0);
        let xmin = xmin - 0.01 * xmin.abs();
        let xmax = data
            .iter()
            .map(|d| d.x)
            .max_by_key(|x| (x * 1000.) as i64)
            .unwrap_or(0.0);
        let xmax = xmax + 0.01 * xmax.abs();
        let xstep = (xmax - xmin) / (nbins as f64);
        let ymin = data
            .iter()
            .map(|d| d.y)
            .min_by_key(|x| (x * 1000.) as i64)
            .unwrap_or(0.0);
        let ymin = ymin - 0.01 * ymin.abs();
        let ymax = data
            .iter()
            .map(|d| d.y)
            .max_by_key(|x| (x * 1000.) as i64)
            .unwrap_or(0.0);
        let ymax = ymax + 0.01 * ymax.abs();
        let ystep = (ymax - ymin) / (nbins as f64);

        let mut bins: HashMap<(u64, u64), u64> = HashMap::new();
        for xbin in 0..nbins {
            for ybin in 0..nbins {
                bins.insert((xbin, ybin), 0);
            }
        }

        for DataPoint { x, y } in data {
            let xindex = ((x - xmin) / xstep) as u64;
            let yindex = ((y - ymin) / ystep) as u64;
            if let Some(x) = bins.get_mut(&(xindex, yindex)) {
                *x += 1;
            } else {
                debug!("missing {xindex} {yindex} {x} {y} {xmin} {ymin} {xmax} {ymax}",);
            }
        }

        let mut data: Vec<ScatterDataPoint> = bins
            .iter()
            .map(|((xb, yb), c)| ScatterDataPoint {
                x: *xb as f64 * xstep + xmin,
                y: *yb as f64 * ystep + ymin,
                c: *c,
            })
            .collect();
        data.shrink_to_fit();

        Some(ScatterPlotData { data, xstep, ystep })
    } else {
        None
    }
}

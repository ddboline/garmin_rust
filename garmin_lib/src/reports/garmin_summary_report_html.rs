use chrono::{Datelike, Utc};
use failure::Error;

use crate::reports::garmin_file_report_html::generate_history_buttons;
use crate::reports::garmin_report_options::{GarminReportAgg, GarminReportOptions};
use crate::reports::garmin_templates::GARMIN_TEMPLATE;
use crate::utils::garmin_util::MONTH_NAMES;

fn generate_url_string(current_line: &str, options: &GarminReportOptions) -> String {
    let now = Utc::now();
    let year = now.year().to_string();
    let month = now.month().to_string();
    let today = now.date().format("%Y-%m-%d").to_string();

    let mut cmd_options = Vec::new();

    if let Some(s) = options.do_sport {
        cmd_options.push(s.to_string());
    }

    if let Some(agg) = &options.agg {
        match agg {
            GarminReportAgg::Year => {
                cmd_options.push("month".to_string());
                let current_year = current_line
                    .trim()
                    .split_whitespace()
                    .nth(0)
                    .unwrap_or(&year);
                cmd_options.push(current_year.to_string());
            }
            GarminReportAgg::Month => {
                cmd_options.push("day".to_string());
                let current_year = current_line
                    .trim()
                    .split_whitespace()
                    .nth(0)
                    .unwrap_or(&year);
                let current_month = current_line
                    .trim()
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or(&month);
                let current_month = MONTH_NAMES
                    .iter()
                    .position(|&x| x == current_month)
                    .unwrap_or(12)
                    + 1;
                if current_month >= 1 && current_month <= 12 {
                    cmd_options.push(format!("{:04}-{:02}", current_year, current_month));
                }
            }
            GarminReportAgg::Week => {
                let isoyear = now.iso_week().year();
                let isoweek = now.iso_week().week();
                cmd_options.push("day".to_string());
                let vals: Vec<_> = current_line.trim().split_whitespace().collect();
                let current_year: i32 = vals[0].parse().unwrap_or(isoyear);
                let current_week: u32 = vals[2].parse().unwrap_or(isoweek);
                cmd_options.push(format!("{:04}w{:02}", current_year, current_week));
            }
            GarminReportAgg::Day => {
                cmd_options.push("file".to_string());
                let current_day = current_line
                    .trim()
                    .split_whitespace()
                    .nth(0)
                    .unwrap_or(&today);
                cmd_options.push(current_day.to_string());
            }
            GarminReportAgg::File => {
                let current_file = current_line.trim().split_whitespace().nth(0).unwrap_or("");
                cmd_options.push(current_file.to_string());
            }
        }
    } else {
        cmd_options.push("year".to_string());
        let current_sport = current_line
            .trim()
            .split_whitespace()
            .nth(0)
            .unwrap_or("running");
        cmd_options.push(current_sport.to_string());
    }

    cmd_options.join(",")
}

pub fn summary_report_html(
    domain: &str,
    retval: &[String],
    options: &GarminReportOptions,
    history: &[String],
) -> Result<String, Error> {
    let htmlostr: Vec<_> = retval
        .iter()
        .map(|ent| {
            let cmd = generate_url_string(&ent, &options);
            format!(
                "<tr><td>{}{}{}{}{}{}</td></tr>",
                r#"<button type="submit" onclick="send_command('filter="#,
                cmd,
                r#"');">"#,
                cmd,
                "</button></td><td>",
                ent.trim()
            )
        })
        .collect();

    let htmlostr = htmlostr.join("\n").replace("\n\n", "<br>\n");

    let mut htmlvec: Vec<String> = Vec::new();

    for line in GARMIN_TEMPLATE.split('\n') {
        if line.contains("INSERTTEXTHERE") {
            htmlvec.push(htmlostr.to_string());
        } else if line.contains("SPORTTITLEDATE") {
            let newtitle = "Garmin Summary";
            htmlvec.push(line.replace("SPORTTITLEDATE", newtitle).to_string());
        } else if line.contains("HISTORYBUTTONS") {
            let history_button = generate_history_buttons(history);
            htmlvec.push(line.replace("HISTORYBUTTONS", &history_button).to_string());
        } else if line.contains("DOMAIN") {
            htmlvec.push(line.replace("DOMAIN", domain));
        } else {
            htmlvec.push(line.to_string());
        }
    }

    Ok(htmlvec.join("\n"))
}

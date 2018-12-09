use failure::Error;
use std::fs::create_dir_all;

use crate::reports::garmin_report_options::GarminReportOptions;
use crate::reports::garmin_templates::GARMIN_TEMPLATE;
use crate::utils::garmin_util::MONTH_NAMES;
use crate::utils::sport_types::get_sport_type_string_map;

fn generate_url_string(current_line: &str, options: &GarminReportOptions) -> String {
    let mut cmd_options = Vec::new();

    let sport_map = get_sport_type_string_map();
    match options.do_sport {
        Some(s) => match sport_map.get(&s) {
            Some(sp) => cmd_options.push(sp.clone()),
            None => (),
        },
        None => (),
    }

    if options.do_year {
        cmd_options.push("month".to_string());
        let current_year = current_line.trim().split_whitespace().nth(0).unwrap();
        cmd_options.push(current_year.to_string());
    } else if options.do_month {
        cmd_options.push("day".to_string());
        let current_year = current_line.trim().split_whitespace().nth(0).unwrap();
        let current_month = current_line.trim().split_whitespace().nth(1).unwrap();
        let current_month = MONTH_NAMES
            .iter()
            .position(|&x| x == current_month)
            .unwrap()
            + 1;
        cmd_options.push(format!("{:04}-{:02}", current_year, current_month));
    } else if options.do_day {
        cmd_options.push("file".to_string());
        let current_day = current_line.trim().split_whitespace().nth(0).unwrap();
        cmd_options.push(current_day.to_string());
    } else if options.do_file {
        let current_file = current_line.trim().split_whitespace().nth(0).unwrap();
        cmd_options.push(current_file.to_string());
    }
    cmd_options.join(",")
}

pub fn summary_report_html(
    retval: &Vec<String>,
    options: &GarminReportOptions,
    cache_dir: &str,
) -> Result<String, Error> {
    let htmlostr: Vec<_> = retval
        .iter()
        .map(|ent| {
            let cmd = generate_url_string(&ent, &options);
            format!(
                "{}{}{}{}{}{}",
                r#"<button type="submit" onclick="send_command('"#,
                cmd,
                r#"');">"#,
                cmd,
                "</button> ",
                ent.trim()
            )
        })
        .collect();

    let htmlostr = htmlostr.join("\n").replace("\n\n", "<br>\n");

    create_dir_all(&format!("{}/html", cache_dir))?;

    let mut htmlvec: Vec<String> = Vec::new();

    for line in GARMIN_TEMPLATE.split("\n") {
        if line.contains("INSERTTEXTHERE") {
            htmlvec.push(format!("{}", htmlostr));
        } else if line.contains("SPORTTITLEDATE") {
            let newtitle = "Garmin Summary";
            htmlvec.push(format!("{}", line.replace("SPORTTITLEDATE", newtitle)));
        } else {
            htmlvec.push(format!("{}", line));
        }
    }

    /*
        create_dir_all(&format!("{}/html/garmin", home_dir))?;
        Exec::shell(format!("rm -rf {}/public_html/garmin/html", home_dir)).join()?;
        Exec::shell(format!(
            "mv {}/html {}/public_html/garmin/",
            cache_dir, home_dir
        )).join()?;
    */
    Ok(htmlvec.join("\n"))
}

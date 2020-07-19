use anyhow::Error;
use stack_string::StackString;

use crate::reports::{
    garmin_file_report_html::generate_history_buttons,
    garmin_summary_report_txt::GarminReportQuery,
    garmin_templates::{GARMIN_TEMPLATE, GARMIN_TEMPLATE_DEMO},
};

pub fn summary_report_html<T>(
    domain: &str,
    report_results: &GarminReportQuery,
    history: &[T],
    is_demo: bool,
) -> Result<StackString, Error>
where
    T: AsRef<str>,
{
    let htmlostr = report_results.get_html_entries()?;

    let htmlostr = htmlostr.join("\n").replace("\n\n", "<br>\n");

    let mut htmlvec: Vec<StackString> = Vec::new();

    let template = if is_demo {
        GARMIN_TEMPLATE_DEMO
    } else {
        GARMIN_TEMPLATE
    };

    for line in template.split('\n') {
        if line.contains("INSERTTEXTHERE") {
            htmlvec.push(r#"<table border="0">"#.into());
            htmlvec.push(htmlostr.clone().into());
        } else if line.contains("SPORTTITLEDATE") {
            let newtitle = "Garmin Summary";
            htmlvec.push(line.replace("SPORTTITLEDATE", newtitle).into());
        } else if line.contains("HISTORYBUTTONS") {
            let history_button = generate_history_buttons(history);
            htmlvec.push(line.replace("HISTORYBUTTONS", &history_button).into());
        } else if line.contains("DOMAIN") {
            htmlvec.push(line.replace("DOMAIN", domain).into());
        } else if line.contains("STRAVAUPLOADBUTTON") {
            htmlvec.push("".into());
        } else if line.contains("SPORTTITLELINK") {
            htmlvec.push("".into());
        } else {
            htmlvec.push(line.into());
        }
    }

    Ok(htmlvec.join("\n").into())
}

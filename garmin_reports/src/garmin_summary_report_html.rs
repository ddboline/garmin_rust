use anyhow::Error;
use maplit::hashmap;
use stack_string::StackString;

use garmin_lib::common::garmin_templates::HBR;

use crate::{
    garmin_file_report_html::generate_history_buttons, garmin_summary_report_txt::GarminReportQuery,
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

    let template = if is_demo {
        "GARMIN_TEMPLATE_DEMO"
    } else {
        "GARMIN_TEMPLATE"
    };
    let insert_text_here = format!(r#"<table border="0">{}</table>"#, htmlostr);
    let history_buttons = generate_history_buttons(history);

    let params = hashmap! {
        "SPORTTITLEDATE" => "Garmin Summary",
        "INSERTTEXTHERE" => &insert_text_here,
        "HISTORYBUTTONS" => &history_buttons,
        "DOMAIN" => domain,
        "STRAVAUPLOADBUTTON" => "",
        "SPORTTITLELINK" => "",
    };
    let body = HBR.render(template, &params)?;
    Ok(body.into())
}

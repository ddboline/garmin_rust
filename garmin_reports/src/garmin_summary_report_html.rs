use anyhow::Error;
use maplit::hashmap;
use stack_string::{format_sstr, StackString};
use std::fmt::Write;

use garmin_lib::common::garmin_templates::{get_buttons, get_scripts, get_style, HBR};

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

    let insert_text_here = format_sstr!(r#"<table border="0">{}</table>"#, htmlostr);
    let history_buttons = generate_history_buttons(history);
    let buttons = get_buttons(is_demo).join("\n");
    let style = get_style(false);

    let params = hashmap! {
        "SPORTTITLEDATE" => "Garmin Summary",
        "INSERTTEXTHERE" => &insert_text_here,
        "HISTORYBUTTONS" => &history_buttons,
        "DOMAIN" => domain,
        "STRAVAUPLOADBUTTON" => "",
        "SPORTTITLELINK" => "",
        "GARMIN_STYLE" => &style,
        "GARMINBUTTONS" => &buttons,
        "GARMIN_SCRIPTS" => get_scripts(is_demo),
    };
    let body = HBR.render("GARMIN_TEMPLATE", &params)?;
    Ok(body.into())
}

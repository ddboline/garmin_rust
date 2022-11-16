use anyhow::Error;
use itertools::Itertools;
use maplit::hashmap;
use stack_string::{format_sstr, StackString};

use garmin_lib::common::garmin_templates::{get_buttons, get_scripts, get_style, HBR};

use garmin_reports::garmin_summary_report_txt::{
    DaySummaryReport, FileSummaryReport, GarminReportQuery, GarminReportTrait, HtmlResult,
    MonthSummaryReport, SportSummaryReport, WeekSummaryReport, YearSummaryReport,
};

use crate::garmin_file_report_html::generate_history_buttons;

pub trait GarminReportHtmlTrait: GarminReportTrait {
    /// # Errors
    /// Returns error if `get_text_entry` entry fails
    fn get_html_entry(&self) -> Result<StackString, Error> {
        let ent = self
            .get_text_entry()?
            .into_iter()
            .map(|(s, u)| {
                u.map_or(s, |u| match u {
                    HtmlResult {
                        text: Some(t),
                        url: Some(u),
                    } => {
                        format_sstr!(r#"<a href="{u}" target="_blank">{t}</a> "#)
                    }
                    HtmlResult {
                        text: Some(t),
                        url: None,
                    } => t,
                    HtmlResult {
                        text: None,
                        url: Some(u),
                    } => {
                        format_sstr!(r#"<a href="{u}" target="_blank">link</a> "#)
                    }
                    _ => "".into(),
                })
            })
            .join("</td><td>");
        let cmd = self.generate_url_string();
        Ok(format_sstr!(
            "<tr><td>{}{}{}{}{}{}</td></tr>",
            r#"<button type="submit" onclick="send_command('filter="#,
            cmd,
            r#"');">"#,
            cmd,
            "</button></td><td>",
            ent.trim()
        ))
    }
}

/// # Errors
/// Return error if `get_html_entry` fails
fn get_html_entries(report: &GarminReportQuery) -> Result<Vec<StackString>, Error> {
    match report {
        GarminReportQuery::Year(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::Month(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::Week(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::Day(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::File(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::Sport(x) => x
            .iter()
            .map(GarminReportHtmlTrait::get_html_entry)
            .collect(),
        GarminReportQuery::Empty => Ok(Vec::new()),
    }
}

/// # Errors
/// Return error if `get_html_entries` fails or template rendering fails
pub fn summary_report_html<T>(
    domain: &str,
    report_results: &GarminReportQuery,
    history: &[T],
    is_demo: bool,
) -> Result<StackString, Error>
where
    T: AsRef<str>,
{
    let htmlostr = get_html_entries(report_results)?;

    let htmlostr = htmlostr.join("\n").replace("\n\n", "<br>\n");

    let insert_text_here = format_sstr!(r#"<table border="0">{htmlostr}</table>"#);
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

impl GarminReportHtmlTrait for FileSummaryReport {}
impl GarminReportHtmlTrait for DaySummaryReport {}
impl GarminReportHtmlTrait for WeekSummaryReport {}
impl GarminReportHtmlTrait for MonthSummaryReport {}
impl GarminReportHtmlTrait for SportSummaryReport {}
impl GarminReportHtmlTrait for YearSummaryReport {}

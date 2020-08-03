use anyhow::Error;
use handlebars::Handlebars;
use lazy_static::lazy_static;

fn get_templates() -> Result<Handlebars<'static>, Error> {
    let mut h = Handlebars::new();
    h.register_template_string(
        "GARMIN_TEMPLATE",
        include_str!("../../../templates/GARMIN_TEMPLATE.html.hbr"),
    )?;
    h.register_template_string(
        "MAP_TEMPLATE",
        include_str!("../../../templates/MAP_TEMPLATE.html.hbr"),
    )?;
    h.register_template_string(
        "LINEPLOTTEMPLATE",
        include_str!("../../../templates/LINEPLOTTEMPLATE.js.hbr"),
    )?;
    h.register_template_string(
        "SCATTERPLOTTEMPLATE",
        include_str!("../../../templates/SCATTERPLOTTEMPLATE.js.hbr"),
    )?;
    h.register_template_string(
        "TIMESERIESTEMPLATE",
        include_str!("../../../templates/TIMESERIESTEMPLATE.js.hbr"),
    )?;
    h.register_template_string(
        "SCATTERPLOTWITHLINES",
        include_str!("../../../templates/SCATTERPLOTWITHLINES.js.hbr"),
    )?;
    h.register_template_string(
        "PLOT_TEMPLATE",
        include_str!("../../../templates/PLOT_TEMPLATE.html.hbr"),
    )?;
    h.register_template_string(
        "GARMIN_TEMPLATE_DEMO",
        include_str!("../../../templates/GARMIN_TEMPLATE_DEMO.html.hbr"),
    )?;
    h.register_template_string(
        "MAP_TEMPLATE_DEMO",
        include_str!("../../../templates/MAP_TEMPLATE_DEMO.html.hbr"),
    )?;
    h.register_template_string(
        "PLOT_TEMPLATE_DEMO",
        include_str!("../../../templates/PLOT_TEMPLATE_DEMO.html.hbr"),
    )?;

    Ok(h)
}

lazy_static! {
    pub static ref HBR: Handlebars<'static> = get_templates().expect("Failed to parse templates");
}

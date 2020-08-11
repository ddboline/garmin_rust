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
    h.register_template_string("GOOGLE_MAP_SCRIPT", google_map_script())?;
    Ok(h)
}

lazy_static! {
    pub static ref HBR: Handlebars<'static> = get_templates().expect("Failed to parse templates");
}

pub fn get_scripts(demo: bool) -> &'static str {
    if demo {
        include_str!("../../../templates/GARMIN_SCRIPTS_DEMO.js.hbr")
    } else {
        include_str!("../../../templates/GARMIN_SCRIPTS.js.hbr")
    }
}

pub fn get_buttons(demo: bool) -> Vec<&'static str> {
    let mut buttons = Vec::new();
    if !demo {
        buttons.extend_from_slice(&[
            r#"<button type="submit" onclick="garmin_sync();">sync with S3</button>"#,
            r#"<button type="submit" onclick="garmin_connect_sync();">sync with Garmin Connect</button>"#,
            r#"<button type="submit" onclick="garminConnectUserSummary();">Connect User Summary</button>"#,
            r#"<button type="submit" onclick="fitbit_tcx_sync();">sync with Fitbit TCX</button>"#,
            r#"<button type="submit" onclick="stravaAuth();">read auth Strava</button>"#,
            r#"<button type="submit" onclick="fitbitAuth();">Fitbit Auth</button>"#,
            r#"<button type="submit" onclick="heartrateSync();">Scale sync</button>"#,
            r#"<br>"#,
            r#"<button type="submit" onclick="stravaAthlete();">Strava Athlete</button>"#,
            r#"<button type="submit" onclick="strava_sync();">sync with Strava</button>"#,
            r#"<button type="submit" onclick="fitbitProfile();">Fitbit Profile</button>"#,
            r#"<button type="submit" onclick="fitbitSync();">Fitbit Sync</button>"#,
            r#"<br>"#,]);
    }
    buttons.extend_from_slice(&[
        r#"<button type="submit" onclick="scale_measurement_plots(0);">Scale Plots</button>"#,
        r#"<button type="submit" onclick="heartrate_stat_plot(0);">Hear Rate Stats</button>"#,
        r#"<button type="submit" onclick="heartrate_plot();">Hear Rate Plots</button>"#,
        r#"<button type="submit" onclick="race_result_plot_personal();">Race Result Plot</button>"#,
        r#"<button name="garminconnectoutput" id="garminconnectoutput"> &nbsp; </button>"#,
        r#"<button type="submit" onclick="send_command('filter=latest');"> latest </button>"#,
        r#"<button type="submit" onclick="send_command('filter=sport');"> sport </button>"#,
    ]);
    if !demo {
        buttons.extend_from_slice(&[
            r#"<form action="/garmin/upload_file" method="post" enctype="multipart/form-data">"#,
            r#"    <input type="file" name="filename">"#,
            r#"    <input type="submit">"#,
            r#"</form>"#,
        ]);
    }
    buttons
}

pub fn get_style() -> &'static str {
    r#"
<style>
    html, body, #map-canvas {
    height: 80%;
    width: 80%;
    }
body { font: 12px Arial;}
path {
    stroke: steelblue;
    stroke-width: 2;
    fill: none;
}
.axis path,
.axis line {
    fill: none;
    stroke-width: 1;
    shape-rendering: crispEdges;

    stroke: #000;
}
.label {
font-weight: bold;
}

.tile {
shape-rendering: crispEdges;
}

</style>
    "#
}

fn google_map_script() -> &'static str {
    r#"
<script type="text/javascript" src="https://maps.googleapis.com/maps/api/js?key={{MAPSAPIKEY}}">
</script>
<script type="text/javascript">
    function initialize() {
    let mapOptions = {
        center: { lat: {{CENTRALLAT}}, lng: {{CENTRALLON}}},
        zoom: {{ZOOMVALUE}} ,
        mapTypeId: google.maps.MapTypeId.SATELLITE
    };
    let map = new google.maps.Map(document.getElementById('garmin_text_box'),
        mapOptions);
    let runningRouteCoordinates = [
        {{{INSERTMAPSEGMENTSHERE}}}
        ];
    let runningRoute = new google.maps.Polyline({
        path: runningRouteCoordinates,
        geodesic: true,
        strokeColor: '#FF0000',
        strokeOpacity: 1.0,
        strokeWeight: 2
    });
    runningRoute.setMap(map);
    };
    google.maps.event.addDomListener(window, 'load', initialize);
</script>   
    "#
}

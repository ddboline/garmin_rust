function send_command( command ) {
    let url = '/garmin/index.html?' + command;
    location.replace(url);
}
function processFormData() {
    let garmin_filter = document.getElementById( 'garmin_filter' );
    send_command( 'filter=' + garmin_filter.value );
}
function processStravaData(filename, activity_type) {
    let strava_title = document.getElementById( 'strava_upload' );
    let url = '/garmin/strava/upload';
    let data = JSON.stringify(
        {
            "filename": filename,
            "title": strava_title.value,
            "activity_type": activity_type
        }
    );
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        let xmlhttp2 = new XMLHttpRequest();
        xmlhttp2.onload = function() {
            location.reload()
        }
        xmlhttp2.open("POST", xmlhttp.responseText, true);
        xmlhttp2.send(null);
        document.getElementById("garminconnectoutput").innerHTML = "syncing";
    }
    xmlhttp.open( "POST", url , true );
    xmlhttp.setRequestHeader("Content-Type", "application/json");
    xmlhttp.send(data);
    document.getElementById("garminconnectoutput").innerHTML = "uploading";
}
function processStravaUpdate(activity_id, activity_type, start_time) {
    let strava_title = document.getElementById( 'strava_upload' );
    let url = '/garmin/strava/update';
    let data = JSON.stringify(
        {
            "activity_id": activity_id,
            "title": strava_title.value,
            "activity_type": activity_type,
            "start_time": start_time,
        }
    );
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        let xmlhttp2 = new XMLHttpRequest();
        xmlhttp2.onload = function() {
            location.reload()
        }
        xmlhttp2.open("POST", xmlhttp.responseText, true);
        xmlhttp2.send(null);
        document.getElementById("garminconnectoutput").innerHTML = "syncing";
    }
    xmlhttp.open( "POST", url , true );
    xmlhttp.setRequestHeader("Content-Type", "application/json");
    xmlhttp.send(data);
    document.getElementById("garminconnectoutput").innerHTML = "updating";
}
function garmin_sync() {
    let url = '/garmin/garmin_sync';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function garmin_connect_sync() {
    let url = '/garmin/garmin_connect_sync';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function fitbit_tcx_sync() {
    let url = '/garmin/fitbit/fitbit_tcx_sync';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function stravaAuth() {
    let url = "/garmin/strava/auth";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        let win = window.open(xmlhttp.responseText, '_blank');
        win.focus()
    }
    xmlhttp.open( "GET", url, true );
    xmlhttp.send(null);
}
function stravaAthlete() {
    let url = "/garmin/strava/athlete";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function strava_sync() {
    let url = '/garmin/strava_sync';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function fitbitAuth() {
    let url = '/garmin/fitbit/auth';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        let win = window.open(xmlhttp.responseText, '_blank');
        win.focus()
    }
    xmlhttp.open("GET", url, true);
    xmlhttp.send(null);
}
function fitbitProfile() {
    let url = "/garmin/fitbit/profile";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function fitbitSync() {
    let url = '/garmin/fitbit/bodyweight_sync';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function scale_measurement_plots(offset, start_date=null, end_date=null) {
    if(document.getElementById("start_date_selector_scale")) {
        if(document.getElementById("start_date_selector_scale").value) {
            start_date = document.getElementById("start_date_selector_scale").value;
        }
    }
    if(document.getElementById("end_date_selector_scale")) {
        if(document.getElementById("end_date_selector_scale").value) {
            end_date = document.getElementById("end_date_selector_scale").value;
        }
    }
    let url = '/garmin/fitbit/plots?offset=' + offset;
    if(start_date) {
        url = url + "&start_date=" + start_date;
    }
    if(end_date) {
        url = url + "&end_date=" + end_date;
    }
    location.replace(url)
}
function heartrate_plot() {
    let url = '/garmin/fitbit/heartrate_plots';
    location.replace(url)
}
function heartrate_stat_plot(offset, start_date=null, end_date=null) {
    if(document.getElementById("start_date_selector_stat")) {
        if(document.getElementById("start_date_selector_stat").value) {
            start_date = document.getElementById("start_date_selector_stat").value;
        }
    }
    if(document.getElementById("end_date_selector_stat")) {
        if(document.getElementById("end_date_selector_stat").value) {
            end_date = document.getElementById("end_date_selector_stat").value;
        }
    }
    let url = '/garmin/fitbit/heartrate_statistics_plots?offset=' + offset;
    if(start_date) {
        url = url + "&start_date=" + start_date;
    }
    if(end_date) {
        url = url + "&end_date=" + end_date;
    }
    location.replace(url)
}
function heartrateSync() {
    let url = '/sync/sync_garmin';
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function addGarminCorrectionSport(begin_datetime) {
    let sport = document.getElementById( 'sport_select' );
    let url = '/garmin/add_garmin_correction';
    let data = JSON.stringify(
        {
            "start_time": begin_datetime,
            "lap_number": 0,
            "sport": sport.value
        }
    );
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
    }
    xmlhttp.open( "POST", url , true );
    xmlhttp.setRequestHeader("Content-Type", "application/json");
    xmlhttp.send(data);
    document.getElementById("garminconnectoutput").innerHTML = "updating";
}
function race_result_plot_personal() {
    let url = "/garmin/race_result_plot?race_type=personal"
    location.replace(url)
}
function race_result_plot_world_record_men() {
    let url = "/garmin/race_result_plot?race_type=world_record_men"
    location.replace(url)
}
function race_result_plot_world_record_women() {
    let url = "/garmin/race_result_plot?race_type=world_record_women"
    location.replace(url)
}
function flipRaceResultFlag(id) {
    let url = '/garmin/race_result_flag?id=' + id;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("race_flag_" + id).innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "updating";
}
function garminConnectUserSummary() {
    let url = "/garmin/garmin_connect_user_summary";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = JSON.stringify(JSON.parse(xmlhttp.responseText),null,2);
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "updating";
}
function heartrate_plot_date(start_date, end_date) {
    let url = '/garmin/fitbit/heartrate_plots?start_date=' + start_date + "&end_date=" + end_date;
    location.replace(url)
}
function heartrate_plot_button(start_date, end_date, button_date) {
    if(document.getElementById("start_date_selector_heart")) {
        if(document.getElementById("start_date_selector_heart").value) {
            start_date = document.getElementById("start_date_selector_heart").value;
        }
    }
    if(document.getElementById("end_date_selector_heart")) {
        if(document.getElementById("end_date_selector_heart").value) {
            end_date = document.getElementById("end_date_selector_heart").value;
        }
    }
    let url = '/garmin/fitbit/heartrate_plots?start_date=' + start_date + '&end_date=' + end_date + '&button_date=' + button_date;
    console.log(url);
    location.replace(url)
}
function heartrate_plot_button_single(date, button_date) {
    let url = '/garmin/fitbit/heartrate_plots?start_date=' + date + '&end_date=' + date + '&button_date=' + button_date;
    console.log(url);
    location.replace(url)
}
function heartrate_sync(date) {
    let url = '/garmin/fitbit/sync?date=' + date;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function createStravaActivity(filename) {
    let url = '/garmin/strava/create?filename=' + filename;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.onload = function() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.open("POST", url, true);
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "processing";
}
function raceResultImport(filename) {
    let url = '/garmin/race_result_import?filename=' + filename;
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("GET", url, true);
    xmlhttp.onload = function nothing() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("garmin_text_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "syncing";
}
function scaleMeasurementManualInput() {
    let url = "/garmin/scale_measurements/manual";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("scale_measurement_box").innerHTML = xmlhttp.responseText;
    }
    let weight_in_lbs = parseFloat(document.getElementById('weight_in_lbs').value);
    let body_fat_percent = parseFloat(document.getElementById('body_fat_percent').value);
    let muscle_mass_lbs = parseFloat(document.getElementById('muscle_mass_lbs').value);
    let body_water_percent = parseFloat(document.getElementById('body_water_percent').value);
    let bone_mass_lbs = parseFloat(document.getElementById('bone_mass_lbs').value);

    let data = JSON.stringify(
        {
            "weight_in_lbs": weight_in_lbs,
            "body_fat_percent": body_fat_percent,
            "muscle_mass_lbs": muscle_mass_lbs,
            "body_water_percent": body_water_percent,
            "bone_mass_lbs": bone_mass_lbs
        }
    );
    xmlhttp.setRequestHeader("Content-Type", "application/json");
    xmlhttp.send(data);
    document.getElementById("garminconnectoutput").innerHTML = "processing";
}
function manualScaleMeasurement() {
    let url = "/garmin/scale_measurements/manual/input";
    let xmlhttp = new XMLHttpRequest();
    xmlhttp.open("POST", url, true);
    xmlhttp.onload = function() {
        document.getElementById("garminconnectoutput").innerHTML = "done";
        document.getElementById("scale_measurement_box").innerHTML = xmlhttp.responseText;
    }
    xmlhttp.send(null);
    document.getElementById("garminconnectoutput").innerHTML = "processing";
}
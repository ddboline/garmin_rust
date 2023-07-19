function send_command( command ) {
    let url = '/garmin/demo.html?' + command;
    location.replace(url);
}
function processFormData() {
    let garmin_filter = document.getElementById( 'garmin_filter' );
    send_command( 'filter=' + garmin_filter.value );
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
    let url = '/garmin/fitbit/plots_demo?offset=' + offset;
    if(start_date) {
        url = url + "&start_date=" + start_date;
    }
    if(end_date) {
        url = url + "&end_date=" + end_date;
    }
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
    let url = '/garmin/fitbit/heartrate_statistics_plots_demo?offset=' + offset;
    if(start_date) {
        url = url + "&start_date=" + start_date;
    }
    if(end_date) {
        url = url + "&end_date=" + end_date;
    }

    location.replace(url)
}
function heartrate_plot() {
    let url = '/garmin/fitbit/heartrate_plots_demo';
    location.replace(url)
}
function race_result_plot_personal() {
    let url = "/garmin/race_result_plot_demo?race_type=personal"
    location.replace(url)
}
function race_result_plot_world_record_men() {
    let url = "/garmin/race_result_plot_demo?race_type=world_record_men"
    location.replace(url)
}
function race_result_plot_world_record_women() {
    let url = "/garmin/race_result_plot_demo?race_type=world_record_women"
    location.replace(url)
}
function heartrate_plot_date(start_date, end_date) {
    let url = '/garmin/fitbit/heartrate_plots_demo?start_date=' + start_date + "&end_date=" + end_date;
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
    let url = '/garmin/fitbit/heartrate_plots_demo?start_date=' + start_date + '&end_date=' + end_date +
        '&button_date=' + button_date;
    location.replace(url)
}
function heartrate_plot_button_single(date, button_date) {
    let url = '/garmin/fitbit/heartrate_plots_demo?start_date=' + date + '&end_date=' + date + '&button_date=' + button_date;
    console.log(url);
    location.replace(url)
}

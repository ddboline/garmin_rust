function send_command( command ) {
    let ostr = '/garmin/demo.html?' + command;
    location.replace(ostr);
}
function processFormData() {
    let garmin_filter = document.getElementById( 'garmin_filter' );
    send_command( 'filter=' + garmin_filter.value );
}
function scale_measurement_plots(offset, start_date=null, end_date=null) {
    let ostr = '/garmin/fitbit/plots_demo?offset=' + offset;
    if(start_date) {
        ostr = ostr + "&" + start_date;
    }
    if(end_date) {
        ostr = ostr + "&" + end_date;
    }
    location.replace(ostr)
}
function heartrate_stat_plot(offset) {
    let ostr = '/garmin/fitbit/heartrate_statistics_plots_demo?offset=' + offset;
    location.replace(ostr)
}
function heartrate_plot() {
    let ostr = '/garmin/fitbit/heartrate_plots_demo';
    location.replace(ostr)
}
function race_result_plot_personal() {
    let ostr = "/garmin/race_result_plot_demo?race_type=personal"
    location.replace(ostr)
}
function race_result_plot_world_record_men() {
    let ostr = "/garmin/race_result_plot_demo?race_type=world_record_men"
    location.replace(ostr)
}
function race_result_plot_world_record_women() {
    let ostr = "/garmin/race_result_plot_demo?race_type=world_record_women"
    location.replace(ostr)
}
function heartrate_plot_date(start_date, end_date) {
    let ostr = '/garmin/fitbit/heartrate_plots_demo?start_date=' + start_date + "&end_date=" + end_date;
    location.replace(ostr)
}
function heartrate_plot_button(start_date, end_date, button_date) {
    let ostr = '/garmin/fitbit/heartrate_plots_demo?start_date=' + start_date + '&end_date=' + end_date + 
        '&button_date=' + button_date;
    location.replace(ostr)
}

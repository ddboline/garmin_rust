use stack_string::StackString;

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

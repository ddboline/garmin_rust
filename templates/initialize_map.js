function create_init(center_lat, center_lon, zoom_value, runningRouteCoordinates) {
    return function init() {
        let mapOptions = {
            center: { lat: center_lat, lng: center_lon},
            zoom: zoom_value,
            mapTypeId: google.maps.MapTypeId.SATELLITE
        };
        let map = new google.maps.Map(
            document.getElementById('garmin_text_box'), mapOptions
        );
        let runningRoute = new google.maps.Polyline({
            path: runningRouteCoordinates,
            geodesic: true,
            strokeColor: '#FF0000',
            strokeOpacity: 1.0,
            strokeWeight: 2
        });
        runningRoute.setMap(map);
    };
}
function initialize(center_lat, center_lon, zoom_value, runningRouteCoordinates) {
    let init = create_init(center_lat, center_lon, zoom_value, runningRouteCoordinates);
    google.maps.event.addDomListener(window, 'load', init);
}

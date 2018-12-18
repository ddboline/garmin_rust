pub static GARMIN_TEMPLATE: &str = r#"
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN"
"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
<title>SPORTTITLEDATE</title>
<meta name="generator" content="HTML::TextToHTML v2.51"/>
<meta http-equiv="Cache-Control" content="no-store" />
</head>
<body>

<p>
HISTORYBUTTONS
</p>

<p>
<form>
<input type="text" name="cmd" id="garmin_cmd"/>
<input type="button" name="submitGARMIN" value="Submit" onclick="processFormData();"/>
</form>
</p>

<pre>
INSERTTEXTHERE
</pre>

<script language="JavaScript" type="text/javascript">
    function send_command( command ) {
        var ostr = '../garmin?' + command;
        location.replace(ostr);
    }
    function processFormData() {
        var garmin_cmd = document.getElementById( 'garmin_cmd' );
        send_command( 'filter=' + garmin_cmd.value );
    }
</script>

</body>
</html>
"#;

pub static MAP_TEMPLATE: &str = r#"
<!DOCTYPE html>
<html>
  <head>
    <meta name="viewport" content="initial-scale=1.0, user-scalable=no">
    <meta charset="utf-8">
    <meta http-equiv="Cache-Control" content="no-store" />
    <title>SPORTTITLEDATE</title>
    <style>
      html, body, #map-canvas {
        height: 80%;
        width: 80%;
/*         margin: 10px; */
/*         padding: 10px */
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
    <script type="text/javascript"
      src="https://maps.googleapis.com/maps/api/js?key=MAPSAPIKEY">
    </script>
    <script type="text/javascript">
      function initialize() {
        var mapOptions = {
          center: { lat: CENTRALLAT, lng: CENTRALLON},
          zoom: ZOOMVALUE ,
          mapTypeId: google.maps.MapTypeId.SATELLITE
        };
        var map = new google.maps.Map(document.getElementById('map-canvas'),
            mapOptions);
        var runningRouteCoordinates = [
            INSERTMAPSEGMENTSHERE
            ];
        var runningRoute = new google.maps.Polyline({
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
  </head>
  <body>

<p>
HISTORYBUTTONS
</p>

<p>
<form>
<input type="text" name="cmd" id="garmin_cmd"/>
<input type="button" name="submitGARMIN" value="Submit" onclick="processFormData();"/>
</form>
</p>

<h1><center><b>SPORTTITLEDATE</b></center></h1>

<div id="map-canvas"></div>

<div>
    INSERTTABLESHERE
</div>

<!-- load the d3.js library -->
<script src="https://d3js.org/d3.v3.min.js"></script>

<div>
    INSERTOTHERIMAGESHERE
</div>

<script language="JavaScript" type="text/javascript">
    function send_command( command ) {
        var ostr = '../garmin?' + command;
        location.replace(ostr);
    }
    function processFormData() {
        var garmin_cmd = document.getElementById( 'garmin_cmd' );
        send_command( 'filter=' + garmin_cmd.value );
    }
</script>

</body>

</html>
"#;

pub static LINEPLOTTEMPLATE: &str = r#"
<script>

// Set the dimensions of the canvas / graph
var margin = {top: 30, right: 20, bottom: 30, left: 50},
    width = 600 - margin.left - margin.right,
    height = 270 - margin.top - margin.bottom;

// Parse the date / time
var parseDate = d3.time.format("%d-%b-%y").parse;

// Set the ranges
var x = d3.scale.linear().range([0, width]);
var y = d3.scale.linear().range([height, 0]);

// Define the axes
var xAxis = d3.svg.axis().scale(x)
    .orient("bottom").ticks(5);

var yAxis = d3.svg.axis().scale(y)
    .orient("left").ticks(5);

// Define the line
var valueline = d3.svg.line()
    .x(function(d) { return x(d[0]); })
    .y(function(d) { return y(d[1]); });
    
// Adds the svg canvas
var svg = d3.select("body")
    .append("svg")
        .attr("width", width + margin.left + margin.right)
        .attr("height", height + margin.top + margin.bottom)
    .append("g")
        .attr("transform",
              "translate(" + margin.left + "," + margin.top + ")");

svg.append("text")      // text label for chart Title
        .attr("x", width / 2 )
        .attr("y", 0 - (margin.top/2))
        .style("text-anchor", "middle")
		.style("font-size", "16px")
        .style("text-decoration", "underline")
        .text("EXAMPLETITLE");

svg.append("text")      // text label for the x-axis
        .attr("x", width / 2 )
        .attr("y",  height + margin.bottom)
        .style("text-anchor", "middle")
        .text("XAXIS");

svg.append("text")      // text label for the y-axis
        .attr("y",30 - margin.left)
        .attr("x",50 - (height / 2))
        .attr("transform", "rotate(-90)")
        .style("text-anchor", "end")
        .style("font-size", "16px")
        .text("YAXIS");

// Get the data
var data = DATA;

var xmax = d3.max(data, function(d) {return d[0]});
var xmin = d3.min(data, function(d) {return d[0]});
var ymax = d3.max(data, function(d) {return d[1]});
var ymin = d3.min(data, function(d) {return d[1]});

xmax = xmax + 0.1 * Math.abs(xmax);
xmin = xmin - 0.1 * Math.abs(xmin);
ymax = ymax + 0.1 * Math.abs(ymax);
ymin = ymin - 0.1 * Math.abs(ymin);

x.domain([xmin, xmax]);
y.domain([ymin, ymax]);

svg.append("path").attr("class", "line").attr("d", valueline(data));
svg.append("g").attr("class", "xaxis").attr("transform", "translate(0," + height + ")").call(xAxis);
svg.append("g").attr("class", "yaxis").call(yAxis);

</script>
"#;

pub static SCATTERPLOTTEMPLATE: &str = r#"
<script>

var margin = {top: 20, right: 90, bottom: 30, left: 50},
    width = 960 - margin.left - margin.right,
    height = 500 - margin.top - margin.bottom;

var x = d3.scale.linear().range([0, width]),
    y = d3.scale.linear().range([height, 0]),
    z = d3.scale.linear().range(["white", "steelblue"]);

// The size of the buckets in the CSV data file.
// This could be inferred from the data if it weren't sparse.
var xStep = XSTEP,
    yStep = YSTEP;

var svg = d3.select("body").append("svg")
    .attr("width", width + margin.left + margin.right)
    .attr("height", height + margin.top + margin.bottom)
  .append("g")
    .attr("transform", "translate(" + margin.left + "," + margin.top + ")");

var data = DATA;

// Compute the scale domains.
x.domain(d3.extent(data, function(d) { return d[0]; }));
y.domain(d3.extent(data, function(d) { return d[1]; }));
z.domain([0, d3.max(data, function(d) { return d[2]; })]);

// Extend the x- and y-domain to fit the last bucket.
// For example, the y-bucket 3200 corresponds to values [3200, 3300].
x.domain([x.domain()[0], +x.domain()[1] + xStep]);
y.domain([y.domain()[0], y.domain()[1] + yStep]);

// Display the tiles for each non-zero bucket.
// See http://bl.ocks.org/3074470 for an alternative implementation.
svg.selectAll(".tile")
    .data(data)
.enter().append("rect")
    .attr("class", "tile")
    .attr("x", function(d) { return x(d[0]); })
    .attr("y", function(d) { return y(d[1] + yStep); })
    .attr("width", x(xStep) - x(0))
    .attr("height",  y(0) - y(yStep))
    .style("fill", function(d) { return z(d[2]); });

// Add a legend for the color values.
var legend = svg.selectAll(".legend")
    .data(z.ticks(6).slice(1).reverse())
.enter().append("g")
    .attr("class", "legend")
    .attr("transform", function(d, i) { return "translate(" + (width + 20) + "," + (20 + i * 20) + ")"; });

legend.append("rect")
    .attr("width", 20)
    .attr("height", 20)
    .style("fill", z);

legend.append("text")
    .attr("x", 26)
    .attr("y", 10)
    .attr("dy", ".35em")
    .text(String);

svg.append("text")
    .attr("class", "label")
    .attr("x", width + 20)
    .attr("y", 10)
    .attr("dy", ".35em")
    .text("Count");

svg.append("text")      // text label for chart Title
    .attr("x", width / 2 )
    .attr("y", 0 - (margin.top/4))
    .style("text-anchor", "middle")
    .style("font-size", "16px")
    .style("text-decoration", "underline")
    .text("EXAMPLETITLE");

// Add an x-axis with label.
svg.append("g")
    .attr("class", "x axis")
    .attr("transform", "translate(0," + height + ")")
    .call(d3.svg.axis().scale(x).orient("bottom"))
.append("text")
    .attr("class", "label")
    .attr("x", width)
    .attr("y", -6)
    .attr("text-anchor", "end")
    .text("XLABEL");

// Add a y-axis with label.
svg.append("g")
    .attr("class", "y axis")
    .call(d3.svg.axis().scale(y).orient("left"))
.append("text")
    .attr("class", "label")
    .attr("y", 6)
    .attr("dy", ".71em")
    .attr("text-anchor", "end")
    .attr("transform", "rotate(-90)")
    .text("YLABEL");

</script>
"#;

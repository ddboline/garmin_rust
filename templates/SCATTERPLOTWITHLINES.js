// Set the dimensions of the canvas / graph
var margin = {top: 30, right: 20, bottom: 30, left: 50},
    width = 600 - margin.left - margin.right,
    height = 400 - margin.top - margin.bottom;

// Get the data
var data = DATA;
var other_data = OTHERDATA;

var fit_data = FITDATA;
var neg_data = NEGDATA;
var pos_data = POSDATA;

var xmax = d3.max(data, function(d) {return d[0]});
var xmin = d3.min(data, function(d) {return d[0]});
var ymax = d3.max(data, function(d) {return d[1]});
var ymin = d3.min(data, function(d) {return d[1]});

xmax = xmax + 0.1 * Math.abs(xmax);
xmin = xmin - 0.1 * Math.abs(xmin);
ymax = YMAX;
ymin = YMIN;

let xmap = XMAP;

// Set the ranges
var x = d3.scale.log().domain([xmin, xmax]).range([0, width]);
var y = d3.scale.linear().domain([ymin, ymax]).range([height, 0]);

// Define the axes
var xAxis = d3.svg.axis().scale(x)
    .orient("bottom")
    .tickValues(XTICKS)
    .tickFormat(function(d) {return xmap[d];});

var yAxis = d3.svg.axis().scale(y)
    .orient("left")
    .tickValues(YTICKS);

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

svg.append("g")
    .selectAll("dot")
    .data(data)
    .enter()
    .append("circle")
        .attr("cx", function(d) {return x(d[0]);})
        .attr("cy", function(d) {return y(d[1]);})
        .attr("r", 3)
        .style("fill", "blue");

svg.append("g")
    .selectAll("dot")
    .data(other_data)
    .enter()
    .append("circle")
        .attr("cx", function(d) {return x(d[0]);})
        .attr("cy", function(d) {return y(d[1]);})
        .attr("r", 1)
        .style("fill", "green");

var valueline = d3.svg.line()
    .x(function(d) { return x(d[0]); })
    .y(function(d) { return y(d[1]); });

svg.append("path").attr("class", "line").attr("d", valueline(fit_data));
svg.append("path").attr("class", "line")
    .style("stroke-dasharray", ("3, 3")).attr("d", valueline(neg_data));
svg.append("path").attr("class", "line")
    .style("stroke-dasharray", ("3, 3")).attr("d", valueline(pos_data));

svg.append("g").attr("class", "xaxis").attr("transform", "translate(0," + height + ")").call(xAxis);
svg.append("g").attr("class", "yaxis").call(yAxis);
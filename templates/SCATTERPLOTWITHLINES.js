// Set the dimensions of the canvas / graph
var margin = {top: 30, right: 20, bottom: 30, left: 50},
    width = 600 - margin.left - margin.right,
    height = 270 - margin.top - margin.bottom;

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

svg.append("g")
    .attr("fill", "black")
    .attr("stroke", "black")
    .attr("stroke-width", 2)
    .selectAll("circle")
    .data(data)
    .join("circle")
    .attr("cx", d => x(d[0]))
    .attr("cy", d => y(d[1]))
    .attr("r", 2);

svg.append("g").attr("class", "xaxis").attr("transform", "translate(0," + height + ")").call(xAxis);
svg.append("g").attr("class", "yaxis").call(yAxis);

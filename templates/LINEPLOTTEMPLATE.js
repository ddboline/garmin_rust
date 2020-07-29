// Set the dimensions of the canvas / graph
var margin = {top: 30, right: 20, bottom: 30, left: 60},
    width = 600 - margin.left - margin.right,
    height = 270 - margin.top - margin.bottom;

// Set the ranges
var x = d3.scaleLinear().range([0, width]);
var y = d3.scaleLinear().range([height, 0]);

// Define the axes
var xAxis = d3.axisBottom(x).ticks(5);

var yAxis = d3.axisLeft(y).ticks(5);

// Define the line
var valueline = d3.line()
    .x(function(d) { return x(d[0]); })
    .y(function(d) { return y(d[1]); });
    
// Adds the svg canvas
var svg = d3.select("body")
    .append("svg")
        .attr("width", width + margin.left + margin.right)
        .attr("height", height + margin.top + margin.bottom)
    .append("g")
        .attr("transform",
              "translate(" + margin.left + "," + margin.top + ")")
    .on("mousemove touchmove", handleMouseOverData);

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

let rule = svg.append("g")
    .append("line")
      .attr("y1", y(ymin))
      .attr("y2", y(ymax))
      .attr("stroke", "black");

function handleMouseOverData() {
    let d = d3.mouse(this)
    let date = x.invert(d[0]);
    let heartrate = y.invert(d[1]);

    rule.attr("transform", `translate(${d[0]}, 0)`);

    svg.property("value", date).dispatch("input");
    d3.event.preventDefault();

    let data_date = d3.select('#data_date');
    if (data_date) {
        data_date.remove();
    }
    let data_heartrate = d3.select('#data_heartrate');
    if (data_heartrate) {
        data_heartrate.remove();
    }

    svg.append('text')
        .attr("id", 'data_date')
        .attr("x", function() {return x(xmin) + 30;})
        .attr("y", function() {return y(ymax) + 15;})
        .text(function() {return date;});
    svg.append('text')
        .attr("id", 'data_heartrate')
        .attr("x", function() {return x(xmin) + 30;})
        .attr("y", function() {return y(ymax) + 30;})
        .text(function() {return heartrate;});
}

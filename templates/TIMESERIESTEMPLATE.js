// Set the dimensions of the canvas / graph
var margin = {top: 30, right: 20, bottom: 30, left: 60},
    width = 600 - margin.left - margin.right,
    height = 270 - margin.top - margin.bottom;

// Parse the date / time
var parseDateTime = d3.timeParse("%Y-%m-%dT%H:%M:%S%Z");

// Set the ranges
var x = d3.scaleTime().range([0, width]);
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
data.forEach(function(d) {
    d[0] = parseDateTime(d[0]);
});

var ymax = d3.max(data, function(d) {return d[1]});
var ymin = d3.min(data, function(d) {return d[1]});

ymax = ymax + 0.1 * Math.abs(ymax);
ymin = ymin - 0.1 * Math.abs(ymin);

x.domain(d3.extent(data, function(d) {return d[0]; }));
y.domain([ymin, ymax]);

svg.append("path").attr("class", "line").attr("d", valueline(data));

svg.append("g")
      .attr("class", "x axis")
      .attr("transform", "translate(0," + height + ")")
      .call(xAxis)
	    .selectAll(".tick text")
      .call(wrap, 35);

svg.append("g").attr("class", "yaxis").call(yAxis);

function wrap(text, width) {
    text.each(function() {
        var text = d3.select(this),
            words = text.text().split(/\s+/).reverse(),
            word,
            line = [],
            lineNumber = 0,
            lineHeight = 1.1, // ems
            y = text.attr("y"),
            dy = parseFloat(text.attr("dy")),
            tspan = text.text(null).append("tspan").attr("x", 0).attr("y", y).attr("dy", dy + "em");
        while (word = words.pop()) {
        line.push(word);
        tspan.text(line.join(" "));
        if (tspan.node().getComputedTextLength() > width) {
            line.pop();
            tspan.text(line.join(" "));
            line = [word];
            tspan = text.append("tspan").attr("x", 0).attr("y", y).attr("dy", ++lineNumber * lineHeight + dy + "em").text(word);
        }
        }
    });
}

function handleMouseOverData(d, i) {
    svg.append('line')
        .attr("id", "data_line")
        .attr("x1", x(d[0]))
        .attr("y1", y(ymin))
        .attr("x2", x(d[0]))
        .attr("y2", y(ymax))
        .style("stroke-width", 2)
        .style("stroke", "black")
        .style("fill", "none");
    svg.append('text')
        .attr("id", 'data')
        .attr("x", function() {return x(xmin) + 30;})
        .attr("y", function() {return y(ymax) + 15;})
        .text(function() {return d[0];});
    svg.append('text')
        .attr("id", 'data_date')
        .attr("x", function() {return x(xmin) + 30;})
        .attr("y", function() {return y(ymax) + 30;})
        .text(function() {return d[1];});
}

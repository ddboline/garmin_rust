function scatter_plot_with_lines(
    data, other_data, fit_data, neg_data, pos_data,
    xmap, ymin, ymax,
    xticks, yticks,
    title, xaxis, yaxis,
) {
    // Set the dimensions of the canvas / graph
    var margin = {top: 30, right: 20, bottom: 30, left: 50},
        width = 600 - margin.left - margin.right,
        height = 400 - margin.top - margin.bottom;

    var xmax = d3.max(data, function(d) {return d.x});
    var xmin = d3.min(data, function(d) {return d.x});

    xmax = xmax + 0.1 * Math.abs(xmax);
    xmin = xmin - 0.1 * Math.abs(xmin);

    // Set the ranges
    var x = d3.scaleLog().domain([xmin, xmax]).range([0, width]);
    var y = d3.scaleLinear().domain([ymin, ymax]).range([height, 0]);

    // Define the axes
    var xAxis = d3.axisBottom(x)
        .tickValues(xticks)
        .tickFormat(function(d) {return xmap[d];});

    var yAxis = d3.axisLeft(y)
        .tickValues(yticks);

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
            .text(title);

    svg.append("text")      // text label for the x-axis
            .attr("x", width / 2 )
            .attr("y",  height + margin.bottom)
            .style("text-anchor", "middle")
            .text(xaxis);

    svg.append("text")      // text label for the y-axis
            .attr("y",30 - margin.left)
            .attr("x",50 - (height / 2))
            .attr("transform", "rotate(-90)")
            .style("text-anchor", "end")
            .style("font-size", "16px")
            .text(yaxis);

    svg.append("g")
        .selectAll("dot")
        .data(data)
        .enter()
        .append("circle")
            .attr("cx", function(d) {return x(d.x);})
            .attr("cy", function(d) {return y(d.y);})
            .attr("r", 4)
            .style("fill", "blue")
            .on("mouseover", handleMouseOverData)
            .on("mouseout", handleMouseOutData);

    svg.append("g")
        .selectAll("dot")
        .data(other_data)
        .enter()
        .append("circle")
            .attr("cx", function(d) {return x(d.x);})
            .attr("cy", function(d) {return y(d.y);})
            .attr("r", 3)
            .style("fill", "green")
            .on("mouseover", handleMouseOverOtherData)
            .on("mouseout", handleMouseOutOtherData);

    svg.append("path").attr("class", "line").attr("d", valueline(fit_data));
    svg.append("path").attr("class", "line")
        .style("stroke-dasharray", ("3, 3")).attr("d", valueline(neg_data));
    svg.append("path").attr("class", "line")
        .style("stroke-dasharray", ("3, 3")).attr("d", valueline(pos_data));

    svg.append("g").attr("class", "xaxis").attr("transform", "translate(0," + height + ")").call(xAxis);
    svg.append("g").attr("class", "yaxis").call(yAxis);

    function handleMouseOverData(d, i) {
        svg.append('text')
            .attr("id", 'data' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 15;})
            .text(function() {return data[i].name;});
        svg.append('text')
            .attr("id", 'data_date' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 30;})
            .text(function() {return data[i].date;});
        svg.append('text')
            .attr("id", 'data_time' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 45;})
            .text(function() {return data[i].label;});
    }

    function handleMouseOutData(d, i) {
        d3.select('#data' + i).remove();
        d3.select('#data_date' + i).remove();
        d3.select('#data_time' + i).remove();
    }

    function handleMouseOverOtherData(d, i) {
        svg.append('text')
            .attr("id", 'otherdata' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 15;})
            .text(function() {return other_data[i].name;});
        svg.append('text')
            .attr("id", 'otherdata_date' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 30;})
            .text(function() {return other_data[i].date;});
        svg.append('text')
            .attr("id", 'otherdata_time' + i)
            .attr("x", function() {return x(xmin) + 30;})
            .attr("y", function() {return y(ymax) + 45;})
            .text(function() {return other_data[i].label;});
    }

    function handleMouseOutOtherData(d, i) {
        d3.select('#otherdata' + i).remove();
        d3.select('#otherdata_date' + i).remove();
        d3.select('#otherdata_time' + i).remove();
    }
}
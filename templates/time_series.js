function time_series(data, title, xaxis, yaxis, units) {
    // Set the dimensions of the canvas / graph
    let margin = {top: 30, right: 20, bottom: 30, left: 60};
    let width = 600 - margin.left - margin.right;
    let height = 270 - margin.top - margin.bottom;

    // Parse the date / time
    var parseDateTime = d3.timeParse("%Y-%m-%dT%H:%M:%S%Z");

    // Set the ranges
    let x = d3.scaleTime().range([0, width]);
    let y = d3.scaleLinear().range([height, 0]);

    // Define the axes
    let xAxis = d3.axisBottom(x).ticks(5);

    let yAxis = d3.axisLeft(y).ticks(5);
        
    // Adds the svg canvas
    let svg = d3.select("body")
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

    data.forEach(function(d) {
        d.x = parseDateTime(d.x);
    });

    let xmax = d3.max(data, function(d) {return d.x});
    let xmin = d3.min(data, function(d) {return d.x});
    let ymax = d3.max(data, function(d) {return d.y});
    let ymin = d3.min(data, function(d) {return d.y});

    ymax = ymax + 0.1 * Math.abs(ymax);
    ymin = ymin - 0.1 * Math.abs(ymin);

    x.domain(d3.extent(data, function(d) {return d.x; }));
    y.domain([ymin, ymax]);

    // Define the line
    var valueline = d3.line()
        .x(function(d) { return x(d.x); })
        .y(function(d) { return y(d.y); });

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
            .text(function() {return heartrate.toFixed(1) + " " + units;});
    }
}
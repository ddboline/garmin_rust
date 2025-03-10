use serde::Serialize;
use stack_string::StackString;

#[derive(Serialize, Default, PartialEq, Clone, Copy)]
pub struct DataPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Default)]
pub struct PlotOpts<'a> {
    pub name: StackString,
    pub title: StackString,
    pub data: Option<&'a [DataPoint]>,
    pub do_scatter: bool,
    pub marker: Option<StackString>,
    pub xlabel: StackString,
    pub ylabel: StackString,
}

#[allow(clippy::similar_names)]
impl<'a> PlotOpts<'a> {
    #[must_use]
    pub fn new() -> PlotOpts<'a> {
        PlotOpts {
            name: "".into(),
            title: "".into(),
            data: None,
            do_scatter: false,
            marker: None,
            xlabel: "".into(),
            ylabel: "".into(),
        }
    }

    #[must_use]
    pub fn with_name(mut self, name: &str) -> PlotOpts<'a> {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: &str) -> PlotOpts<'a> {
        self.title = title.into();
        self
    }

    #[must_use]
    pub fn with_data(mut self, data: &'a [DataPoint]) -> PlotOpts<'a> {
        self.data = Some(data);
        self
    }

    #[must_use]
    pub fn with_scatter(mut self) -> PlotOpts<'a> {
        self.do_scatter = true;
        self
    }

    #[must_use]
    pub fn with_marker(mut self, marker: &str) -> PlotOpts<'a> {
        self.marker = Some(marker.into());
        self
    }

    #[must_use]
    pub fn with_labels(mut self, xlabel: &str, ylabel: &str) -> PlotOpts<'a> {
        self.xlabel = xlabel.into();
        self.ylabel = ylabel.into();
        self
    }
}

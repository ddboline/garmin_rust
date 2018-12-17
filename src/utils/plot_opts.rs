#[derive(Serialize, Default)]
pub struct PlotOpts<'a> {
    pub name: String,
    pub title: String,
    pub data: Option<&'a [(f64, f64)]>,
    pub do_scatter: bool,
    pub cache_dir: String,
    pub marker: Option<String>,
    pub xlabel: String,
    pub ylabel: String,
}

impl<'a> PlotOpts<'a> {
    pub fn new() -> PlotOpts<'a> {
        PlotOpts {
            name: "".to_string(),
            title: "".to_string(),
            data: None,
            do_scatter: false,
            cache_dir: "".to_string(),
            marker: None,
            xlabel: "".to_string(),
            ylabel: "".to_string(),
        }
    }

    pub fn with_name(mut self, name: &str) -> PlotOpts<'a> {
        self.name = name.to_string();
        self
    }

    pub fn with_title(mut self, title: &str) -> PlotOpts<'a> {
        self.title = title.to_string();
        self
    }

    pub fn with_data(mut self, data: &'a [(f64, f64)]) -> PlotOpts<'a> {
        self.data = Some(data);
        self
    }

    pub fn with_scatter(mut self) -> PlotOpts<'a> {
        self.do_scatter = true;
        self
    }

    pub fn with_cache_dir(mut self, cache_dir: &str) -> PlotOpts<'a> {
        self.cache_dir = cache_dir.to_string();
        self
    }

    pub fn with_marker(mut self, marker: &str) -> PlotOpts<'a> {
        self.marker = Some(marker.to_string());
        self
    }

    pub fn with_labels(mut self, xlabel: &str, ylabel: &str) -> PlotOpts<'a> {
        self.xlabel = xlabel.to_string();
        self.ylabel = ylabel.to_string();
        self
    }
}

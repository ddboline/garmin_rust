use failure::{err_msg, Error};
use parking_lot::Mutex;
use rand::distributions::{Distribution, Uniform};
use rand::thread_rng;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Response, Url};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use super::garmin_config::GarminConfig;

#[derive(Debug)]
struct GarminConnectClientInner {
    client: Client,
    headers: HeaderMap,
}

pub struct GarminConnect {
    client: Arc<Mutex<GarminConnectClientInner>>,
}

impl Default for GarminConnect {
    fn default() -> Self {
        Self::new()
    }
}

impl GarminConnectClientInner {
    pub fn set_default_headers(&mut self, headers: HashMap<String, String>) -> Result<(), Error> {
        headers
            .into_iter()
            .map(|(k, v)| {
                let name: HeaderName = k.parse()?;
                let val: HeaderValue = v.parse()?;
                self.headers.insert(name, val);
                Ok(())
            })
            .collect()
    }

    pub fn get(&mut self, url: Url, headers: HeaderMap) -> Result<Response, Error> {
        let mut headers: HeaderMap = self.headers.clone();
        for (k, v) in headers.into_iter() {
            println!("{}", k);
            headers.insert(k, v);
        }
        self.client
            .get(url)
            .headers(headers)
            .send()
            .map_err(err_msg)
    }

    pub fn post(
        &mut self,
        url: Url,
        headers: HeaderMap,
        form: HashMap<String, String>,
    ) -> Result<Response, Error> {
        let mut headers = self.headers.clone();
        for (k, v) in headers.iter() {
            let _: Option<HeaderName> = headers.insert(k, v);
        }
        self.client
            .post(url)
            .headers(headers)
            .form(&form)
            .send()
            .map_err(err_msg)
    }
}

impl GarminConnect {
    pub fn new() -> Self {
        GarminConnect {
            client: Arc::new(Mutex::new(GarminConnectClientInner {
                client: Client::builder()
                    .cookie_store(true)
                    .build()
                    .expect("Failed to build client"),
                headers: HeaderMap::new(),
            })),
        }
    }

    pub fn get(&self, url: &Url) -> Result<Response, Error> {
        let mut timeout: f64 = 1.0;
        let mut rng = thread_rng();
        let range = Uniform::from(0..1000);
        loop {
            let mut client = self.client.lock();
            let resp = client.get(url.clone());
            match resp {
                Ok(x) => return Ok(x),
                Err(e) => {
                    sleep(Duration::from_millis((timeout * 1000.0) as u64));
                    timeout *= 4.0 * f64::from(range.sample(&mut rng)) / 1000.0;
                    if timeout >= 64.0 {
                        return Err(err_msg(e));
                    }
                }
            }
        }
    }
}

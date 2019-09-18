use failure::{err_msg, Error};
use parking_lot::Mutex;
use rand::distributions::{Distribution, Uniform};
use rand::thread_rng;
use reqwest::{cookie::Cookie, header::HeaderMap, Client, Response, Url};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug)]
struct GarminConnectClientInner<'a> {
    client: Client,
    headers: HeaderMap,
    cookies: HashMap<String, Cookie<'a>>,
}

pub struct GarminConnect<'a> {
    client: Arc<Mutex<GarminConnectClientInner<'a>>>,
}

impl Default for GarminConnect<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> GarminConnectClientInner<'a> {
    pub fn merge_cookies(&mut self, cookies: Vec<Cookie<'a>>) {
        for cookie in cookies {
            if !self.cookies.contains_key(cookie.name()) {
                self.cookies.insert(cookie.name().into(), cookie);
            }
        }
    }

    pub fn set_headers(&mut self, headers: HashMap<String, String>) -> Result<(), Error> {
        Ok(())
    }

    pub fn get(&mut self, url: Url) -> Result<Response, Error> {
        self.client.get(url).send().map_err(err_msg)
    }
}

impl<'a> GarminConnect<'a> {
    pub fn new() -> Self {
        GarminConnect {
            client: Arc::new(Mutex::new(GarminConnectClientInner {
                client: Client::new(),
                headers: HeaderMap::new(),
                cookies: HashMap::new(),
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

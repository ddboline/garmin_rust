use failure::{err_msg, Error};
use parking_lot::Mutex;
use rand::distributions::{Distribution, Uniform};
use rand::thread_rng;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::blocking::{Response, Client};
use reqwest::{ Url};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug)]
struct ReqwestSessionInner {
    client: Client,
    headers: HeaderMap,
}

pub struct ReqwestSession {
    client: Arc<Mutex<ReqwestSessionInner>>,
}

impl Default for ReqwestSession {
    fn default() -> Self {
        Self::new(true)
    }
}

impl ReqwestSessionInner {
    pub fn get(&mut self, url: Url, mut headers: HeaderMap) -> Result<Response, Error> {
        for (k, v) in &self.headers {
            headers.insert(k, v.into());
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
        mut headers: HeaderMap,
        form: &HashMap<&str, &str>,
    ) -> Result<Response, Error> {
        for (k, v) in &self.headers {
            headers.insert(k, v.into());
        }
        self.client
            .post(url)
            .headers(headers)
            .form(form)
            .send()
            .map_err(err_msg)
    }
}

impl ReqwestSession {
    pub fn new(allow_redirects: bool) -> Self {
        let redirect_policy = if allow_redirects {
            Policy::default()
        } else {
            Policy::none()
        };
        ReqwestSession {
            client: Arc::new(Mutex::new(ReqwestSessionInner {
                client: Client::builder()
                    .cookie_store(true)
                    .redirect(redirect_policy)
                    .build()
                    .expect("Failed to build client"),
                headers: HeaderMap::new(),
            })),
        }
    }

    fn exponential_retry<T, U>(&self, f: T) -> Result<U, Error>
    where
        T: Fn() -> Result<U, Error>,
    {
        let mut timeout: f64 = 1.0;
        let mut rng = thread_rng();
        let range = Uniform::from(0..1000);
        loop {
            let resp = f();
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

    pub fn get(&self, url: &Url, headers: HeaderMap) -> Result<Response, Error> {
        self.exponential_retry(|| self.client.lock().get(url.clone(), headers.clone()))
    }

    pub fn post(
        &self,
        url: &Url,
        headers: HeaderMap,
        form: &HashMap<&str, &str>,
    ) -> Result<Response, Error> {
        self.exponential_retry(|| self.client.lock().post(url.clone(), headers.clone(), form))
    }

    pub fn set_default_headers(&self, headers: HashMap<&str, &str>) -> Result<(), Error> {
        headers
            .into_iter()
            .map(|(k, v)| {
                let name: HeaderName = k.parse()?;
                let val: HeaderValue = v.parse()?;
                self.client.lock().headers.insert(name, val);
                Ok(())
            })
            .collect()
    }
}

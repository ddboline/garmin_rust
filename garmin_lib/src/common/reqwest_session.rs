use anyhow::Error;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
    Client, Response, Url,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

use crate::utils::garmin_util::exponential_retry;

#[derive(Debug)]
struct ReqwestSessionInner {
    client: Client,
    headers: HeaderMap,
}

#[derive(Clone)]
pub struct ReqwestSession {
    client: Arc<Mutex<ReqwestSessionInner>>,
}

impl Default for ReqwestSession {
    fn default() -> Self {
        Self::new(true)
    }
}

impl ReqwestSessionInner {
    pub async fn get(&mut self, url: Url, mut headers: HeaderMap) -> Result<Response, Error> {
        for (k, v) in &self.headers {
            headers.insert(k, v.into());
        }
        self.client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(Into::into)
    }

    pub async fn post(
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
            .await
            .map_err(Into::into)
    }
}

impl ReqwestSession {
    pub fn new(allow_redirects: bool) -> Self {
        let redirect_policy = if allow_redirects {
            Policy::default()
        } else {
            Policy::none()
        };
        Self {
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

    pub async fn get(&self, url: &Url, headers: &HeaderMap) -> Result<Response, Error> {
        exponential_retry(|| {
            let url = url.clone();
            let headers = headers.clone();
            async move { self.client.lock().await.get(url, headers).await }
        })
        .await
    }

    pub async fn post(
        &self,
        url: &Url,
        headers: &HeaderMap,
        form: &HashMap<&str, &str>,
    ) -> Result<Response, Error> {
        exponential_retry(|| {
            let url = url.clone();
            let headers = headers.clone();
            async move {
                self.client
                    .lock()
                    .await
                    .post(url.clone(), headers.clone(), form)
                    .await
            }
        })
        .await
    }

    pub async fn set_default_headers(&self, headers: HashMap<&str, &str>) -> Result<(), Error> {
        for (k, v) in headers {
            let name: HeaderName = k.parse()?;
            let val: HeaderValue = v.parse()?;
            self.client.lock().await.headers.insert(name, val);
        }
        Ok(())
    }
}

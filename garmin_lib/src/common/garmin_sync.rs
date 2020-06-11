use anyhow::Error;
use chrono::DateTime;
use futures::stream::{StreamExt, TryStreamExt};
use log::debug;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, Object as S3Object, PutObjectRequest, S3Client};
use s3_ext::S3Ext;
use std::{collections::{HashSet, HashMap}, fs, path::Path, time::SystemTime};
use sts_profile_auth::get_client_sts;
use std::hash::{Hash, Hasher};
use std::borrow::Borrow;

use crate::utils::{
    garmin_util::{exponential_retry, get_md5sum},
    stack_string::StackString,
};

pub fn get_s3_client() -> S3Client {
    get_client_sts!(S3Client, Region::UsEast1).expect("Failed to obtain client")
}

pub struct GarminSync {
    s3_client: S3Client,
}

#[derive(Debug, Clone, Eq)]
pub struct KeyItem {
    pub key: StackString,
    pub etag: StackString,
    pub timestamp: i64,
    pub size: u64,
}

impl PartialEq for KeyItem {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Hash for KeyItem {
    fn hash<H>(&self, state: &mut H)
    where H: Hasher,
    {self.key.hash(state)}
}

impl Borrow<str> for &KeyItem {
    fn borrow(&self) -> &str {
        self.key.as_str()
    }
}

impl Default for GarminSync {
    fn default() -> Self {
        Self::new()
    }
}

fn process_s3_item(mut item: S3Object) -> Option<KeyItem> {
    item.key.take().and_then(|key| {
        item.e_tag.take().and_then(|etag| {
            item.last_modified.as_ref().and_then(|last_mod| {
                DateTime::parse_from_rfc3339(last_mod)
                    .ok()
                    .map(|lm| KeyItem {
                        key: key.into(),
                        etag: etag.trim_matches('"').into(),
                        timestamp: lm.timestamp(),
                        size: item.size.unwrap_or(0) as u64,
                    })
            })
        })
    })
}

impl GarminSync {
    pub fn new() -> Self {
        Self {
            s3_client: get_s3_client(),
        }
    }

    pub fn from_client(s3client: S3Client) -> Self {
        Self {
            s3_client: s3client,
        }
    }

    pub async fn get_list_of_keys(&self, bucket: &str) -> Result<Vec<KeyItem>, Error> {
        let results: Result<Vec<_>, _> = exponential_retry(|| async move {
            self.s3_client
                .stream_objects(bucket)
                .map(|res| res.map(process_s3_item))
                .try_collect()
                .await
                .map_err(Into::into)
        })
        .await;
        let list_of_keys = results?.into_iter().filter_map(|x| x).collect();
        Ok(list_of_keys)
    }

    pub async fn sync_dir(
        &self,
        title: &str,
        local_dir: &str,
        s3_bucket: &str,
        check_md5sum: bool,
    ) -> Result<Vec<String>, Error> {
        let path = Path::new(local_dir);

        let file_list: Result<Vec<_>, Error> = path
            .read_dir()?
            .filter_map(|dir_line| {
                dir_line.ok().map(|entry| entry.path()).map(|f| {
                    let metadata = fs::metadata(&f)?;
                    let modified = metadata
                        .modified()?
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_secs() as i64;
                    let size = metadata.len();
                    let f: StackString = f.to_string_lossy().as_ref().into();
                    Ok((f, modified, size))
                })
            })
            .collect();
        let file_list = file_list?;
        let file_set: HashMap<StackString, _> = file_list
            .iter()
            .filter_map(|(x, t, s)| x.split('/').last().map(|x| (x.into(), (*t, *s))))
            .collect();

        let key_list = self.get_list_of_keys(s3_bucket).await?;
        let n_keys = key_list.len();

        let key_set: HashSet<&KeyItem> = key_list
            .iter().collect();

        let uploaded: Vec<_> = file_list
            .into_par_iter()
            .filter_map(|(file, tmod, size)| {
                let file_name: StackString = match file.split('/').last() {
                    Some(x) => x.to_string().into(),
                    None => return None,
                };
                let mut do_upload = false;
                if key_set.contains(file_name.as_str()) {
                    let item = key_set.get(file_name.as_str()).unwrap();
                    if tmod != item.timestamp {
                        if check_md5sum {
                            if let Ok(md5) = get_md5sum(&file) {
                                if item.etag != md5 {
                                    debug!(
                                        "upload md5 {} {} {} {} {}",
                                        file_name, item.etag, md5, item.timestamp, tmod
                                    );
                                    do_upload = true;
                                }
                            }
                        } else if size > item.size {
                            debug!(
                                "upload size {} {} {} {} {}",
                                file_name, item.etag, size, item.timestamp, item.size
                            );
                            do_upload = true;
                        }
                    }
                    if tmod != item.timestamp && check_md5sum {}
                } else {
                    do_upload = true;
                }
                if do_upload {
                    debug!("upload file {}", file_name);
                    Some((file, file_name))
                } else {
                    None
                }
            })
            .collect();
        let uploaded_files: Vec<_> = uploaded
            .iter()
            .map(|(_, filename)| filename.clone())
            .collect();
        for (file, filename) in uploaded {
            self.upload_file(&file, &s3_bucket, &filename).await?;
        }
        debug!("uploaded {:?}", uploaded_files);

        let downloaded: Result<Vec<_>, Error> = key_list
            .into_par_iter()
            .filter_map(|item| {
                let res = || {
                    let mut do_download = false;

                    if file_set.contains_key(&item.key) {
                        let (tmod_, size_) = file_set[&item.key];
                        if item.timestamp > tmod_ {
                            if check_md5sum {
                                let file_name = format!("{}/{}", local_dir, item.key);
                                let md5_ = get_md5sum(&file_name)?;
                                if md5_.as_str() != item.etag.as_str() {
                                    debug!(
                                        "download md5 {} {} {} {} {} ",
                                        item.key, md5_, item.etag, item.timestamp, tmod_
                                    );
                                    let file_name = format!("{}/{}", local_dir, item.key);
                                    fs::remove_file(&file_name)?;
                                    do_download = true;
                                }
                            } else if item.size > size_ {
                                let file_name = format!("{}/{}", local_dir, item.key);
                                debug!(
                                    "download size {} {} {} {} {}",
                                    item.key, size_, item.size, item.timestamp, tmod_
                                );
                                fs::remove_file(&file_name)?;
                                do_download = true;
                            }
                        }
                    } else {
                        do_download = true;
                    };

                    if do_download {
                        let file_name = format!("{}/{}", local_dir, item.key);
                        debug!("download {} {}", s3_bucket, item.key);
                        Ok(Some((file_name, item.key)))
                    } else {
                        Ok(None)
                    }
                };
                res().transpose()
            })
            .collect();
        let downloaded = downloaded?;
        let downloaded_files: Vec<_> = downloaded
            .iter()
            .map(|(file_name, _)| file_name.clone())
            .collect();
        for (file_name, key) in downloaded {
            self.download_file(&file_name, &s3_bucket, &key).await?;
        }
        debug!("downloaded {:?}", downloaded_files);

        let msg = format!(
            "{} {} s3_bucketnkeys {} uploaded {} downloaded {}",
            title,
            s3_bucket,
            n_keys,
            uploaded_files.len(),
            downloaded_files.len()
        );

        Ok(vec![msg])
    }

    pub async fn download_file(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<String, Error> {
        exponential_retry(|| async move {
            let etag = self
                .s3_client
                .download_to_file(
                    GetObjectRequest {
                        bucket: s3_bucket.to_string(),
                        key: s3_key.to_string(),
                        ..GetObjectRequest::default()
                    },
                    local_file,
                )
                .await?
                .e_tag
                .unwrap_or_else(|| "".to_string());
            Ok(etag)
        })
        .await
    }

    pub async fn upload_file(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        self.upload_file_acl(local_file, s3_bucket, s3_key, &None)
            .await
    }

    pub async fn upload_file_acl(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
        acl: &Option<String>,
    ) -> Result<(), Error> {
        exponential_retry(|| async move {
            self.s3_client
                .upload_from_file(
                    &local_file,
                    PutObjectRequest {
                        bucket: s3_bucket.to_string(),
                        key: s3_key.to_string(),
                        acl: acl.clone(),
                        ..PutObjectRequest::default()
                    },
                )
                .await
                .map_err(Into::into)
                .map(|_| ())
        })
        .await
    }
}

use anyhow::Error;
use chrono::DateTime;
use futures::stream::{StreamExt, TryStreamExt};
use log::debug;
use rand::{
    distributions::{Alphanumeric, DistString},
    thread_rng,
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, Object as S3Object, PutObjectRequest, S3Client};
use s3_ext::S3Ext;
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    fmt::Write,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::SystemTime,
};
use sts_profile_auth::get_client_sts;
use tokio::task::spawn_blocking;

use crate::utils::garmin_util::{exponential_retry, get_md5sum};

pub fn get_s3_client() -> S3Client {
    get_client_sts!(S3Client, Region::UsEast1).expect("Failed to obtain client")
}

#[derive(Clone)]
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
    where
        H: Hasher,
    {
        self.key.hash(state);
    }
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
        let list_of_keys = results?.into_iter().flatten().collect();
        Ok(list_of_keys)
    }

    pub async fn sync_dir(
        &self,
        title: &str,
        local_dir: &Path,
        s3_bucket: &str,
        check_md5sum: bool,
    ) -> Result<StackString, Error> {
        let file_list: Result<Vec<_>, Error> = local_dir
            .read_dir()?
            .filter_map(|dir_line| {
                dir_line.ok().map(|entry| entry.path()).map(|f| {
                    let metadata = fs::metadata(&f)?;
                    let modified = metadata
                        .modified()?
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_secs() as i64;
                    let size = metadata.len();
                    Ok((f, modified, size))
                })
            })
            .collect();
        let file_list = file_list?;
        let file_set: HashMap<StackString, _> = file_list
            .iter()
            .filter_map(|(f, t, s)| {
                f.file_name()
                    .map(|x| (x.to_string_lossy().as_ref().into(), (*t, *s)))
            })
            .collect();

        let key_list = self.get_list_of_keys(s3_bucket).await?;
        let n_keys = key_list.len();

        let key_set: HashSet<&KeyItem> = key_list.iter().collect();

        let uploaded: Vec<_> = file_list
            .into_par_iter()
            .filter_map(|(file, tmod, size)| {
                let file_name: StackString = file.file_name()?.to_string_lossy().as_ref().into();
                let mut do_upload = false;
                if let Some(item) = key_set.get(file_name.as_str()) {
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
            self.upload_file(&file, s3_bucket, &filename).await?;
        }
        debug!("uploaded {:?}", uploaded_files);

        let downloaded = spawn_blocking({
            let local_dir = local_dir.to_path_buf();
            let s3_bucket: StackString = s3_bucket.into();
            move || get_downloaded(key_list, check_md5sum, &file_set, &local_dir, &s3_bucket)
        })
        .await??;
        let downloaded_files: Vec<_> = downloaded
            .iter()
            .map(|(file_name, _)| file_name.clone())
            .collect();
        for (file_name, key) in downloaded {
            self.download_file(&file_name, s3_bucket, &key).await?;
        }
        debug!("downloaded {:?}", downloaded_files);

        let msg = format_sstr!(
            "{} {} s3_bucketnkeys {} uploaded {} downloaded {}",
            title,
            s3_bucket,
            n_keys,
            uploaded_files.len(),
            downloaded_files.len()
        );

        Ok(msg)
    }

    pub async fn download_file(
        &self,
        local_file: &Path,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<StackString, Error> {
        let tmp_path = {
            let mut rng = thread_rng();
            let rand_str = Alphanumeric.sample_string(&mut rng, 8);
            local_file.with_file_name(format_sstr!(".tmp_{}", rand_str))
        };
        let etag: Result<StackString, Error> = exponential_retry(|| {
            let tmp_path = tmp_path.clone();
            async move {
                let etag = self
                    .s3_client
                    .download_to_file(
                        GetObjectRequest {
                            bucket: s3_bucket.to_string(),
                            key: s3_key.to_string(),
                            ..GetObjectRequest::default()
                        },
                        &tmp_path,
                    )
                    .await?
                    .e_tag
                    .unwrap_or_else(|| "".to_string());
                Ok(etag.into())
            }
        })
        .await;
        tokio::fs::rename(tmp_path, local_file).await?;
        etag
    }

    pub async fn upload_file(
        &self,
        local_file: &Path,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        self.upload_file_acl(local_file, s3_bucket, s3_key).await
    }

    pub async fn upload_file_acl(
        &self,
        local_file: &Path,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        exponential_retry(|| async move {
            self.s3_client
                .upload_from_file(
                    &local_file,
                    PutObjectRequest {
                        bucket: s3_bucket.to_string(),
                        key: s3_key.to_string(),
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

fn get_downloaded(
    key_list: Vec<KeyItem>,
    check_md5sum: bool,
    file_set: &HashMap<StackString, (i64, u64)>,
    local_dir: &Path,
    s3_bucket: &str,
) -> Result<Vec<(PathBuf, StackString)>, Error> {
    key_list
        .into_par_iter()
        .filter_map(|item| {
            let res = || {
                let mut do_download = false;

                if file_set.contains_key(&item.key) {
                    let (tmod_, size_) = file_set[&item.key];
                    if item.timestamp > tmod_ {
                        if check_md5sum {
                            let file_name = local_dir.join(item.key.as_str());
                            let md5_ = get_md5sum(&file_name)?;
                            if md5_.as_str() != item.etag.as_str() {
                                debug!(
                                    "download md5 {} {} {} {} {} ",
                                    item.key, md5_, item.etag, item.timestamp, tmod_
                                );
                                let file_name = local_dir.join(item.key.as_str());
                                fs::remove_file(&file_name)?;
                                do_download = true;
                            }
                        } else if item.size > size_ {
                            let file_name = local_dir.join(item.key.as_str());
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
                    let file_name = local_dir.join(item.key.as_str());
                    debug!("download {} {}", s3_bucket, item.key);
                    Ok(Some((file_name, item.key)))
                } else {
                    Ok(None)
                }
            };
            res().transpose()
        })
        .collect()
}

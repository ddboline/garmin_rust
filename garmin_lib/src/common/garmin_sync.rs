use anyhow::{format_err, Error};
use aws_config::SdkConfig;
use aws_sdk_s3::{
    operation::list_objects::ListObjectsOutput, primitives::ByteStream, types::Object as S3Object,
    Client as S3Client,
};
use log::debug;
use rand::{
    distributions::{Alphanumeric, DistString},
    thread_rng,
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use tokio::{fs::File, task::spawn_blocking};

use crate::utils::garmin_util::{exponential_retry, get_md5sum};

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

fn process_s3_item(mut item: S3Object) -> Option<KeyItem> {
    item.key.take().and_then(|key| {
        item.e_tag.take().and_then(|etag| {
            item.last_modified.as_ref().map(|last_mod| KeyItem {
                key: key.into(),
                etag: etag.trim_matches('"').into(),
                timestamp: last_mod.as_secs_f64() as i64,
                size: item.size as u64,
            })
        })
    })
}

impl GarminSync {
    #[must_use]
    pub fn new(sdk_config: &SdkConfig) -> Self {
        Self {
            s3_client: S3Client::from_conf(sdk_config.into()),
        }
    }

    #[must_use]
    pub fn from_client(s3client: S3Client) -> Self {
        Self {
            s3_client: s3client,
        }
    }

    async fn list_keys(
        &self,
        bucket: &str,
        marker: Option<impl AsRef<str>>,
    ) -> Result<ListObjectsOutput, Error> {
        let mut builder = self.s3_client.list_objects().bucket(bucket);
        if let Some(marker) = marker {
            builder = builder.marker(marker.as_ref());
        }
        builder.send().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if s3 api call fails
    pub async fn get_list_of_keys(&self, bucket: &str) -> Result<Vec<KeyItem>, Error> {
        let results: Result<Vec<_>, _> = exponential_retry(|| async move {
            let mut marker: Option<String> = None;
            let mut list_of_keys = Vec::new();
            loop {
                let mut output = self.list_keys(bucket, marker.as_ref()).await?;
                if let Some(contents) = output.contents.take() {
                    if let Some(last) = contents.last() {
                        if let Some(key) = &last.key {
                            marker.replace(key.into());
                        }
                    }
                    list_of_keys.extend(contents.into_iter().map(process_s3_item));
                }
                if !output.is_truncated {
                    break;
                }
            }
            Ok(list_of_keys)
        })
        .await;
        let list_of_keys = results?.into_iter().flatten().collect();
        Ok(list_of_keys)
    }

    /// # Errors
    /// Return error if s3 api call fails
    pub async fn sync_dir(
        &self,
        title: &str,
        local_dir: &Path,
        s3_bucket: &str,
        check_md5sum: bool,
    ) -> Result<StackString, Error> {
        let allowed_extensions: HashSet<_> = ["fit", "gmn", "gz", "txt", "avro", "parquet"]
            .iter()
            .map(OsStr::new)
            .collect();

        let mut file_list = Vec::new();
        for dir_line in local_dir.read_dir()? {
            let entry = dir_line?;
            let f = entry.path();
            if let Some(ext) = f.extension() {
                if allowed_extensions.contains(&ext) {
                    let metadata = fs::metadata(&f)?;
                    let modified = metadata
                        .modified()?
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_secs() as i64;
                    let size = metadata.len();
                    file_list.push((f, modified, size));
                }
            }
        }
        let file_list = Arc::new(file_list);
        let file_set: HashMap<StackString, _> = file_list
            .iter()
            .filter_map(|(f, t, s)| {
                f.file_name()
                    .map(|x| (x.to_string_lossy().as_ref().into(), (*t, *s)))
            })
            .collect();

        let key_list = Arc::new(self.get_list_of_keys(s3_bucket).await?);
        let n_keys = key_list.len();

        let uploaded = spawn_blocking({
            let file_list = file_list.clone();
            let key_list = key_list.clone();
            move || get_uploaded(&file_list, &key_list, check_md5sum)
        })
        .await?;

        let uploaded_files: Vec<_> = uploaded.iter().map(|(_, filename)| filename).collect();
        for (file, filename) in &uploaded {
            self.upload_file(file, s3_bucket, filename).await?;
        }
        debug!("uploaded {:?}", uploaded_files);

        let downloaded = spawn_blocking({
            let local_dir = local_dir.to_path_buf();
            let s3_bucket: StackString = s3_bucket.into();
            move || get_downloaded(&key_list, check_md5sum, &file_set, &local_dir, &s3_bucket)
        })
        .await??;
        let downloaded_files: Vec<_> = downloaded.iter().map(|(file_name, _)| file_name).collect();
        for (file_name, key) in &downloaded {
            self.download_file(file_name, s3_bucket, key).await?;
        }
        debug!("downloaded {:?}", downloaded_files);

        let msg = format_sstr!(
            "{title} {s3_bucket} s3_bucketnkeys {n_keys} uploaded {u} downloaded {d}",
            u = uploaded_files.len(),
            d = downloaded_files.len()
        );

        Ok(msg)
    }

    /// # Errors
    /// Return error if s3 api call fails
    pub async fn download_file(
        &self,
        local_file: &Path,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<StackString, Error> {
        let tmp_path = {
            let mut rng = thread_rng();
            let rand_str = Alphanumeric.sample_string(&mut rng, 8);
            local_file.with_file_name(format_sstr!(".tmp_{rand_str}"))
        };
        let etag: Result<StackString, Error> = exponential_retry(|| {
            let tmp_path = tmp_path.clone();
            async move {
                let resp = self
                    .s3_client
                    .get_object()
                    .bucket(s3_bucket)
                    .key(s3_key)
                    .send()
                    .await?;
                let etag: StackString = resp.e_tag().ok_or_else(|| format_err!("No etag"))?.into();
                tokio::io::copy(
                    &mut resp.body.into_async_read(),
                    &mut File::create(tmp_path).await?,
                )
                .await?;
                Ok(etag)
            }
        })
        .await;
        tokio::fs::rename(tmp_path, local_file).await?;
        etag
    }

    /// # Errors
    /// Return error if s3 api call fails
    pub async fn upload_file(
        &self,
        local_file: &Path,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        exponential_retry(|| async move {
            let body = ByteStream::read_from().path(local_file).build().await?;
            self.s3_client
                .put_object()
                .bucket(s3_bucket)
                .key(s3_key)
                .body(body)
                .send()
                .await
                .map(|_| ())
                .map_err(Into::into)
        })
        .await
    }
}

fn get_uploaded(
    file_list: &[(PathBuf, i64, u64)],
    key_list: &[KeyItem],
    check_md5sum: bool,
) -> Vec<(PathBuf, StackString)> {
    let key_set: HashSet<&KeyItem> = key_list.iter().collect();
    file_list
        .par_iter()
        .filter_map(|(file, tmod, size)| {
            let file_name: StackString = file.file_name()?.to_string_lossy().as_ref().into();
            let mut do_upload = false;
            if let Some(item) = key_set.get(file_name.as_str()) {
                if tmod != &item.timestamp {
                    if check_md5sum {
                        if let Ok(md5) = get_md5sum(file) {
                            if item.etag != md5 {
                                debug!(
                                    "upload md5 {} {} {} {} {}",
                                    file_name, item.etag, md5, item.timestamp, tmod
                                );
                                do_upload = true;
                            }
                        }
                    } else if size > &item.size {
                        debug!(
                            "upload size {} {} {} {} {}",
                            file_name, item.etag, size, item.timestamp, item.size
                        );
                        do_upload = true;
                    }
                }
                if tmod != &item.timestamp && check_md5sum {}
            } else {
                do_upload = true;
            }
            if do_upload {
                debug!("upload file {}", file_name);
                Some((file.clone(), file_name))
            } else {
                None
            }
        })
        .collect()
}

fn get_downloaded(
    key_list: &[KeyItem],
    check_md5sum: bool,
    file_set: &HashMap<StackString, (i64, u64)>,
    local_dir: &Path,
    s3_bucket: &str,
) -> Result<Vec<(PathBuf, StackString)>, Error> {
    key_list
        .par_iter()
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
                                fs::remove_file(file_name)?;
                                do_download = true;
                            }
                        } else if item.size > size_ {
                            let file_name = local_dir.join(item.key.as_str());
                            debug!(
                                "download size {} {} {} {} {}",
                                item.key, size_, item.size, item.timestamp, tmod_
                            );
                            fs::remove_file(file_name)?;
                            do_download = true;
                        }
                    }
                } else {
                    do_download = true;
                };

                if do_download {
                    let file_name = local_dir.join(item.key.as_str());
                    debug!("download {} {}", s3_bucket, item.key);
                    Ok(Some((file_name, item.key.clone())))
                } else {
                    Ok(None)
                }
            };
            res().transpose()
        })
        .collect()
}

use chrono::DateTime;
use failure::{err_msg, Error};
use log::debug;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, ListObjectsV2Request, PutObjectRequest, S3Client, S3};
use s4::S4;
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, Write};
use std::path::Path;
use std::time::SystemTime;

use crate::utils::garmin_util::{exponential_retry, get_md5sum, map_result};

pub fn get_s3_client() -> S3Client {
    S3Client::new(Region::UsEast1)
}

pub struct GarminSync<T: S3> {
    s3_client: T,
}

#[derive(Debug, Clone)]
pub struct KeyItem {
    pub key: String,
    pub etag: String,
    pub timestamp: i64,
}

impl Default for GarminSync<S3Client> {
    fn default() -> GarminSync<S3Client> {
        GarminSync::new()
    }
}

impl GarminSync<S3Client> {
    pub fn new() -> GarminSync<S3Client> {
        GarminSync {
            s3_client: get_s3_client(),
        }
    }

    pub fn from_client(s3client: S3Client) -> GarminSync<S3Client> {
        GarminSync {
            s3_client: s3client,
        }
    }

    pub fn get_list_of_keys(&self, bucket: &str) -> Result<Vec<KeyItem>, Error> {
        let mut continuation_token = None;

        let mut list_of_keys = Vec::new();

        loop {
            let current_list = exponential_retry(|| {
                self.s3_client
                    .list_objects_v2(ListObjectsV2Request {
                        bucket: bucket.to_string(),
                        continuation_token: continuation_token.clone(),
                        delimiter: None,
                        encoding_type: None,
                        fetch_owner: None,
                        max_keys: None,
                        prefix: None,
                        request_payer: None,
                        start_after: None,
                    })
                    .sync()
                    .map_err(err_msg)
            })?;

            continuation_token = current_list.next_continuation_token.clone();

            match current_list.key_count {
                Some(0) => (),
                Some(_) => {
                    current_list.contents.map(|c| {
                        let contents: Vec<_> = c
                            .into_iter()
                            .filter_map(|mut item| {
                                item.key.take().and_then(|key| {
                                    item.e_tag.take().and_then(|etag| {
                                        item.last_modified.as_ref().and_then(|last_mod| {
                                            DateTime::parse_from_rfc3339(last_mod).ok().and_then(
                                                |lm| {
                                                    Some(KeyItem {
                                                        key,
                                                        etag: etag.trim_matches('"').to_string(),
                                                        timestamp: lm.timestamp(),
                                                    })
                                                },
                                            )
                                        })
                                    })
                                })
                            })
                            .collect();
                        list_of_keys.extend_from_slice(&contents)
                    });
                }
                None => (),
            };

            match &continuation_token {
                Some(_) => (),
                None => break,
            };
        }

        Ok(list_of_keys)
    }

    pub fn sync_dir(
        &self,
        local_dir: &str,
        s3_bucket: &str,
        check_md5sum: bool,
    ) -> Result<(), Error> {
        let path = Path::new(local_dir);

        let file_list: Vec<String> = path
            .read_dir()?
            .filter_map(|dir_line| {
                dir_line.ok().and_then(|entry| {
                    entry
                        .path()
                        .to_str()
                        .map(|input_file| input_file.to_string())
                })
            })
            .collect();

        let file_list: Vec<Result<_, Error>> = file_list
            .into_par_iter()
            .map(|f| {
                let modified = fs::metadata(&f)?
                    .modified()?
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_secs() as i64;

                Ok((f.to_string(), modified))
            })
            .collect();

        let file_list: Vec<_> = map_result(file_list)?;

        let file_set: HashMap<_, _> = file_list
            .iter()
            .filter_map(|(x, t)| x.split('/').last().map(|x| (x.to_string(), *t)))
            .collect();

        let key_list = self.get_list_of_keys(s3_bucket)?;
        writeln!(
            stdout().lock(),
            "{} s3_bucketnkeys {}",
            s3_bucket,
            key_list.len()
        )?;
        let key_set: HashMap<_, _> = key_list
            .iter()
            .map(|item| (item.key.to_string(), item.clone()))
            .collect();

        let results: Vec<_> = file_list
            .par_iter()
            .filter_map(|(file, tmod)| {
                let file_name = match file.split('/').last() {
                    Some(x) => x.to_string(),
                    None => return None,
                };

                let mut do_upload = false;

                if key_set.contains_key(&file_name) {
                    let item = &key_set[&file_name];
                    if tmod < &item.timestamp && check_md5sum {
                        if let Ok(md5) = get_md5sum(&file) {
                            println!(
                                "tmod {} timestamp {} md5sum {} {}",
                                *tmod, item.timestamp, md5, item.etag
                            );
                            if item.etag != md5 {
                                debug!(
                                    "upload md5 {} {} {} {} {}",
                                    file_name, item.etag, md5, item.timestamp, tmod
                                );
                                do_upload = true;
                            }
                        }
                    }
                } else {
                    do_upload = true;
                }

                if do_upload {
                    debug!("upload file {}", file_name);

                    Some(self.upload_file(&file, &s3_bucket, &file_name))
                } else {
                    None
                }
            })
            .collect();

        map_result(results)?;

        let results: Vec<_> = key_list
            .par_iter()
            .filter_map(|item| {
                let mut do_download = false;

                if file_set.contains_key(&item.key) {
                    let tmod_ = file_set[&item.key];
                    if item.timestamp > tmod_ && check_md5sum {
                        let file_name = format!("{}/{}", local_dir, item.key);
                        let md5_ = get_md5sum(&file_name).expect("Failed md5sum");
                        if md5_ != item.etag {
                            debug!(
                                "download md5 {} {} {} {} {} ",
                                item.key, md5_, item.etag, item.timestamp, tmod_
                            );
                            let file_name = format!("{}/{}", local_dir, item.key);
                            fs::remove_file(&file_name).expect("Failed to remove existing file");
                            do_download = true;
                        }
                    }
                } else {
                    do_download = true;
                };

                if do_download {
                    let file_name = format!("{}/{}", local_dir, item.key);
                    debug!("download {} {}", s3_bucket, item.key);

                    Some(self.download_file(&file_name, &s3_bucket, &item.key))
                } else {
                    None
                }
            })
            .collect();

        let _: Vec<_> = map_result(results)?;

        Ok(())
    }

    pub fn download_file(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<String, Error> {
        let etag = exponential_retry(|| {
            {
                self.s3_client.download_to_file(
                    GetObjectRequest {
                        bucket: s3_bucket.to_string(),
                        key: s3_key.to_string(),
                        ..Default::default()
                    },
                    local_file,
                )
            }
            .map_err(err_msg)
        })?
        .e_tag
        .unwrap_or_else(|| "".to_string());
        Ok(etag)
    }

    pub fn upload_file(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        self.upload_file_acl(local_file, s3_bucket, s3_key, None)
    }

    pub fn upload_file_acl(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
        acl: Option<String>,
    ) -> Result<(), Error> {
        exponential_retry(|| {
            {
                self.s3_client.upload_from_file(
                    &local_file,
                    PutObjectRequest {
                        bucket: s3_bucket.to_string(),
                        key: s3_key.to_string(),
                        acl: acl.clone(),
                        ..Default::default()
                    },
                )
            }
            .map_err(err_msg)
        })?;
        Ok(())
    }
}

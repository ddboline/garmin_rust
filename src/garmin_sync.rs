extern crate futures;
extern crate rayon;
extern crate rusoto_s3;

use futures::stream::Stream;
use std::io::{Read, Write};

use chrono::prelude::*;
use rayon::prelude::*;

use failure::Error;

use crate::utils::garmin_util::{get_md5sum, map_result_vec};
use rusoto_core::Region;
use rusoto_s3::{
    GetObjectOutput, GetObjectRequest, ListObjectsV2Request, PutObjectOutput, PutObjectRequest,
    S3Client, StreamingBody, S3,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

pub fn get_s3_client() -> S3Client {
    S3Client::new(Region::UsEast1)
}

pub struct GarminSync {
    s3_client: S3Client,
}

impl Default for GarminSync {
    fn default() -> GarminSync {
        GarminSync::new()
    }
}

impl GarminSync {
    pub fn new() -> GarminSync {
        GarminSync {
            s3_client: get_s3_client(),
        }
    }

    pub fn from_client(s3client: S3Client) -> GarminSync {
        GarminSync {
            s3_client: s3client,
        }
    }

    pub fn get_list_of_keys(&self, bucket: &str) -> Result<Vec<(String, String, i64)>, Error> {
        let mut continuation_token = None;

        let mut list_of_keys = Vec::new();

        loop {
            let current_list = self
                .s3_client
                .list_objects_v2(ListObjectsV2Request {
                    bucket: bucket.to_string(),
                    continuation_token,
                    delimiter: None,
                    encoding_type: None,
                    fetch_owner: None,
                    max_keys: None,
                    prefix: None,
                    request_payer: None,
                    start_after: None,
                })
                .sync()?;

            continuation_token = current_list.next_continuation_token.clone();

            match current_list.key_count {
                Some(0) => (),
                Some(_) => {
                    for item in current_list.contents.unwrap() {
                        list_of_keys.push((
                            item.key.unwrap(),
                            item.e_tag.unwrap().trim_matches('"').to_string(),
                            DateTime::parse_from_rfc3339(&item.last_modified.unwrap())?.timestamp(),
                        ))
                    }
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

    pub fn sync_dir(&self, local_dir: &str, s3_bucket: &str) -> Result<(), Error> {
        let path = Path::new(local_dir);

        let file_list: Vec<String> = path
            .read_dir()?
            .filter_map(|dir_line| match dir_line {
                Ok(entry) => {
                    let input_file = entry.path().to_str().unwrap().to_string();
                    Some(input_file)
                }
                Err(_) => None,
            })
            .collect();

        let file_list: Vec<_> = file_list
            .into_par_iter()
            .map(|f| {
                let md5sum = get_md5sum(&f).unwrap();

                let modified = fs::metadata(&f)
                    .unwrap()
                    .modified()
                    .unwrap()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                (f.to_string(), md5sum, modified)
            })
            .collect();

        let file_set: HashMap<_, _> = file_list
            .iter()
            .map(|(x, m, t)| {
                (
                    x.split('/').last().unwrap().to_string(),
                    (m.to_string(), *t),
                )
            })
            .collect();

        let key_list = self.get_list_of_keys(s3_bucket)?;
        let key_set: HashMap<_, _> = key_list
            .iter()
            .map(|(k, m, t)| (k.to_string(), (m.to_string(), *t)))
            .collect();

        let results: Vec<_> = file_list
            .par_iter()
            .filter_map(|(file, md5, tmod)| {
                let file_name = file.split('/').last().unwrap().to_string();

                let do_upload = if key_set.contains_key(&file_name) {
                    let (md5_, tmod__) = key_set[&file_name].clone();
                    let tmod_ = &tmod__;
                    if (&md5_ != md5) & (tmod > tmod_) {
                        debug!(
                            "upload md5 {} {} {} {} {}",
                            file_name, md5_, md5, tmod_, tmod
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    true
                };

                if do_upload {
                    println!("upload file {}", file_name);

                    Some(self.upload_file(&file, &s3_bucket, &file_name))
                } else {
                    None
                }
            })
            .collect();

        map_result_vec(results)?;

        let results: Vec<_> = key_list
            .par_iter()
            .filter_map(|(key, md5, tmod)| {
                let do_download = if file_set.contains_key(key) {
                    let (md5_, tmod_) = file_set[key].clone();
                    if (md5_ != *md5) & (*tmod > tmod_) {
                        debug!("download md5 {} {} {} {} {} ", key, md5_, md5, tmod, tmod_);
                        let file_name = format!("{}/{}", local_dir, key);
                        fs::remove_file(&file_name).expect("Failed to remove existing file");
                        true
                    } else {
                        false
                    }
                } else {
                    true
                };

                if do_download {
                    let file_name = format!("{}/{}", local_dir, key);
                    println!("download {} {}", s3_bucket, key);

                    Some(self.download_file(&file_name, &s3_bucket, &key))
                } else {
                    None
                }
            })
            .collect();

        map_result_vec(results)?;

        Ok(())
    }

    pub fn download_file(
        &self,
        local_file: &str,
        s3_bucket: &str,
        s3_key: &str,
    ) -> Result<(), Error> {
        self.download_to_file(
            GetObjectRequest {
                bucket: s3_bucket.to_string(),
                if_match: None,
                if_modified_since: None,
                if_none_match: None,
                if_unmodified_since: None,
                key: s3_key.to_string(),
                part_number: None,
                range: None,
                request_payer: None,
                response_cache_control: None,
                response_content_disposition: None,
                response_content_encoding: None,
                response_content_language: None,
                response_content_type: None,
                response_expires: None,
                sse_customer_algorithm: None,
                sse_customer_key: None,
                sse_customer_key_md5: None,
                version_id: None,
            },
            local_file,
        )?;
        Ok(())
    }

    fn download_to_file<F>(
        &self,
        source: GetObjectRequest,
        target: F,
    ) -> Result<GetObjectOutput, Error>
    where
        F: AsRef<Path>,
    {
        debug!("downloading to file {:?}", target.as_ref());
        let mut resp = self.s3_client.get_object(source).sync()?;
        let mut body = resp.body.take().expect("no body");
        let mut target = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        GarminSync::copy(&mut body, &mut target)?;
        Ok(resp)
    }

    fn copy<W>(src: &mut StreamingBody, dest: &mut W) -> Result<(), Error>
    where
        W: Write,
    {
        let src = src.take(512 * 1024).wait();
        for chunk in src {
            dest.write_all(chunk?.as_mut_slice())?;
        }
        Ok(())
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
        self.upload_from_file(
            &local_file,
            PutObjectRequest {
                acl,
                body: None,
                bucket: s3_bucket.to_string(),
                cache_control: None,
                content_disposition: None,
                content_encoding: None,
                content_language: None,
                content_length: None,
                content_md5: None,
                content_type: None,
                expires: None,
                grant_full_control: None,
                grant_read: None,
                grant_read_acp: None,
                grant_write_acp: None,
                key: s3_key.to_string(),
                metadata: None,
                request_payer: None,
                sse_customer_algorithm: None,
                sse_customer_key: None,
                sse_customer_key_md5: None,
                ssekms_key_id: None,
                server_side_encryption: None,
                storage_class: None,
                tagging: None,
                website_redirect_location: None,
            },
        )?;
        Ok(())
    }

    #[inline]
    fn upload_from_file<F>(
        &self,
        source: F,
        target: PutObjectRequest,
    ) -> Result<PutObjectOutput, Error>
    where
        F: AsRef<Path>,
    {
        debug!("uploading file {:?}", source.as_ref());
        let mut source = fs::File::open(source)?;
        self.upload(&mut source, target)
    }

    fn upload<R>(
        &self,
        source: &mut R,
        mut target: PutObjectRequest,
    ) -> Result<PutObjectOutput, Error>
    where
        R: Read,
    {
        let mut content = Vec::new();
        source.read_to_end(&mut content)?;
        target.body = Some(content.into());
        self.s3_client
            .put_object(target)
            .sync()
            .map_err(|e| e.into())
    }
}

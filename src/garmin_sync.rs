extern crate futures;
extern crate rayon;
extern crate rusoto_s3;
extern crate s4;

use rayon::prelude::*;

use failure::Error;

use crate::garmin_util::get_md5sum;
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, ListObjectsV2Request, PutObjectRequest, S3, S3Client};
use s4::S4;
use std::collections::HashMap;
use std::path::Path;

pub fn get_s3_client() -> S3Client {
    S3Client::new(Region::UsEast1)
}

pub fn get_list_of_keys(s3_client: &S3Client, bucket: &str) -> Vec<(String, String)> {
    let mut continuation_token = None;

    let mut list_of_keys = Vec::new();

    loop {
        let current_list = s3_client
            .list_objects_v2(ListObjectsV2Request {
                bucket: bucket.to_string(),
                continuation_token: continuation_token,
                delimiter: None,
                encoding_type: None,
                fetch_owner: None,
                max_keys: None,
                prefix: None,
                request_payer: None,
                start_after: None,
            })
            .sync()
            .unwrap();

        continuation_token = current_list.next_continuation_token.clone();

        match current_list.key_count {
            Some(0) => (),
            Some(_) => {
                for bucket in current_list.contents.unwrap() {
                    list_of_keys.push((
                        bucket.key.unwrap(),
                        bucket.e_tag.unwrap().trim_matches('"').to_string(),
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

    list_of_keys
}

pub fn sync_dir(local_dir: &str, s3_bucket: &str, s3_client: &S3Client) {
    let path = Path::new(local_dir);

    let file_list: Vec<String> = match path.read_dir() {
        Ok(it) => it.filter_map(|dir_line| match dir_line {
            Ok(entry) => {
                let input_file = entry.path().to_str().unwrap().to_string();
                Some(input_file)
            }
            Err(_) => None,
        }).collect(),
        Err(err) => {
            println!("{}", err);
            Vec::new()
        }
    };
    let file_list: Vec<(String, String)> = file_list
        .par_iter()
        .map(|f| {
            let md5sum = get_md5sum(&f);
            (f.to_string(), md5sum)
        })
        .collect();
    let file_set: HashMap<_, _> = file_list
        .iter()
        .map(|(x, m)| (x.split("/").last().unwrap().to_string(), m.to_string()))
        .collect();

    let key_list = get_list_of_keys(&s3_client, s3_bucket);
    let key_set: HashMap<_, _> = key_list.iter().cloned().collect();

    for (file, md5) in file_list {
        let file_name = file.split("/").last().unwrap();
        if key_set.contains_key(file_name) {
            let md5_ = key_set.get(file_name).unwrap().clone();
            if md5_ != md5 {
                println!("md5 {} {} {}", file_name, md5_, md5);
            }
            continue;
        };
        println!("file_name {}", file_name);

        upload_file(&file, &s3_bucket, &file_name, &s3_client).unwrap();
    }

    for (key, md5) in key_list {
        if file_set.contains_key(&key) {
            let md5_ = file_set.get(&key).unwrap().clone();
            if md5_ != md5 {
                println!("md5 {} {} {}", key, md5_, md5);
            }
            continue;
        };
        let file_name = format!("{}/{}", local_dir, key);
        println!("key {} {}", s3_bucket, key);

        download_file(&file_name, &s3_bucket, &key, &s3_client).unwrap();
    }
}

pub fn download_file(
    local_file: &str,
    s3_bucket: &str,
    s3_key: &str,
    s3_client: &S3Client,
) -> Result<(), Error> {
    s3_client.download_to_file(
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

pub fn upload_file(
    local_file: &str,
    s3_bucket: &str,
    s3_key: &str,
    s3_client: &S3Client,
) -> Result<(), Error> {
    s3_client.upload_from_file(
        &local_file,
        PutObjectRequest {
            acl: None,
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

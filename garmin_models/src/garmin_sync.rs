use aws_config::SdkConfig;
use aws_sdk_s3::{
    operation::list_objects::ListObjectsOutput, primitives::ByteStream, types::Object as S3Object,
    Client as S3Client,
};
use futures::{Stream, TryStreamExt};
use log::{debug, error};
use postgres_query::{query, query_dyn, Error as PgError, FromSqlRow, Parameter};
use rand::{
    distr::{Alphanumeric, SampleString},
    rng,
};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt::Write,
    fs,
    hash::{Hash, Hasher},
    path::Path,
    time::SystemTime,
};
use tokio::{
    fs::File,
    task::{spawn, spawn_blocking, JoinHandle},
};

use garmin_lib::errors::GarminError as Error;

use garmin_utils::{
    garmin_util::{exponential_retry, get_md5sum},
    pgpool::PgPool,
};

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

impl KeyItem {
    #[must_use]
    fn from_s3_object(mut item: S3Object) -> Option<Self> {
        let key = item.key.take()?.into();
        let etag = item.e_tag.take()?.trim_matches('"').into();
        let timestamp = item.last_modified.as_ref()?.as_secs_f64() as i64;

        Some(Self {
            key,
            etag,
            timestamp,
            size: item.size? as u64,
        })
    }
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

    async fn get_and_process_keys_impl(
        &self,
        bucket: &str,
        pool: &PgPool,
    ) -> Result<(usize, usize), Error> {
        let mut file_map: HashMap<StackString, KeyItemCache> =
            KeyItemCache::get_files(pool, bucket, None, None)
                .await?
                .map_ok(|item| (item.s3_key.clone(), item))
                .try_collect()
                .await?;
        file_map.shrink_to_fit();

        let mut marker: Option<String> = None;
        let mut total_keys = 0;
        let mut updated_keys = 0;
        loop {
            let mut output = self.list_keys(bucket, marker.as_ref()).await?;
            if let Some(contents) = output.contents.take() {
                if let Some(last) = contents.last() {
                    if let Some(key) = last.key() {
                        marker.replace(key.into());
                    }
                }
                total_keys += contents.len();
                debug!(
                    "contents {} marker {marker:?} truncated {:?}",
                    contents.len(),
                    output.is_truncated
                );
                for object in contents {
                    if let Some(key) = KeyItem::from_s3_object(object) {
                        if let Some(mut key_item) = file_map.remove(&key.key) {
                            key_item.s3_etag = Some(key.etag);
                            key_item.s3_size = Some(key.size.try_into()?);
                            key_item.s3_timestamp = Some(key.timestamp);

                            if key_item.s3_etag == key_item.local_etag
                                || key_item.s3_size == key_item.local_size
                            {
                                key_item.do_download = false;
                                key_item.do_upload = false;
                            } else if key_item.s3_size > key_item.local_size {
                                key_item.do_download = true;
                                key_item.do_upload = false;
                                updated_keys += 1;
                            } else if key_item.s3_size < key_item.local_size {
                                key_item.do_download = false;
                                key_item.do_upload = true;
                                updated_keys += 1;
                            }
                            key_item.insert(pool).await?;
                        } else {
                            let mut key_item = KeyItemCache::from_keyitem(key, bucket)?;
                            key_item.do_download = true;
                            key_item.insert(pool).await?;
                        }
                    }
                }
            }
            if output.is_truncated == Some(false) || output.is_truncated.is_none() {
                break;
            }
        }
        for (_, mut missing) in file_map {
            missing.s3_etag = None;
            missing.s3_timestamp = None;
            missing.s3_size = None;
            missing.do_download = false;
            missing.insert(pool).await?;
        }
        Ok((total_keys, updated_keys))
    }

    async fn get_and_process_keys(
        &self,
        bucket: &str,
        pool: &PgPool,
    ) -> Result<(usize, usize), Error> {
        exponential_retry(|| async move { self.get_and_process_keys_impl(bucket, pool).await })
            .await
    }

    async fn process_files(
        &self,
        local_dir: &Path,
        s3_bucket: &str,
        pool: &PgPool,
    ) -> Result<usize, Error> {
        let mut allowed_extensions: HashSet<_> = ["fit", "gmn", "gz", "txt", "avro", "parquet"]
            .iter()
            .map(OsStr::new)
            .collect();
        allowed_extensions.shrink_to_fit();

        let mut file_map: HashMap<StackString, KeyItemCache> =
            KeyItemCache::get_files(pool, s3_bucket, None, None)
                .await?
                .map_ok(|item| (item.s3_key.clone(), item))
                .try_collect()
                .await?;
        file_map.shrink_to_fit();

        let mut tasks = Vec::new();

        for dir_line in local_dir.read_dir()? {
            let entry = dir_line?;
            let f = entry.path();
            let filename: StackString = f
                .file_name()
                .ok_or_else(|| Error::StaticCustomError("cannot extract filename"))?
                .to_string_lossy()
                .into();
            let ext = f
                .extension()
                .ok_or_else(|| Error::StaticCustomError("cannot extract extension"))?;
            if allowed_extensions.contains(&ext) {
                let metadata = fs::metadata(&f)?;
                let modified = metadata
                    .modified()?
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_secs() as i64;
                let size: i64 = metadata.len().try_into()?;
                if let Some(mut existing) = file_map.remove(&filename) {
                    if existing.local_timestamp != Some(modified)
                        || existing.local_size != Some(size)
                    {
                        let pool = pool.clone();
                        let task: JoinHandle<Result<(), Error>> = spawn(async move {
                            let etag = spawn_blocking(move || get_md5sum(&f)).await??;
                            existing.local_etag = Some(etag);
                            existing.local_timestamp = Some(modified);
                            existing.local_size = Some(size);
                            existing.do_upload = true;
                            existing.insert(&pool).await?;
                            Ok(())
                        });
                        tasks.push(task);
                    }
                } else {
                    let pool = pool.clone();
                    let s3_bucket: StackString = s3_bucket.into();
                    let task: JoinHandle<Result<(), Error>> = spawn(async move {
                        let etag = spawn_blocking(move || get_md5sum(&f)).await??;
                        KeyItemCache {
                            s3_key: filename,
                            s3_bucket,
                            local_etag: Some(etag),
                            local_timestamp: Some(modified),
                            local_size: Some(size),
                            do_upload: true,
                            ..KeyItemCache::default()
                        }
                        .insert(&pool)
                        .await?;
                        Ok(())
                    });
                    tasks.push(task);
                }
            } else {
                error!("invalid extension {} for {}", ext.display(), f.display());
            }
        }
        for (_, mut missing) in file_map {
            let missing_path = local_dir.join(&missing.s3_key);
            if missing_path.exists() {
                continue;
            }
            missing.local_etag = None;
            missing.local_timestamp = None;
            missing.local_size = None;
            missing.do_upload = false;
            missing.insert(pool).await?;
        }
        let updates = tasks.len();
        for task in tasks {
            let _ = task.await?;
        }
        Ok(updates)
    }

    /// # Errors
    /// Return error if s3 api call fails
    pub async fn sync_dir(
        &self,
        title: &str,
        local_dir: &Path,
        s3_bucket: &str,
        pool: &PgPool,
    ) -> Result<StackString, Error> {
        let number_updated_files = self.process_files(local_dir, s3_bucket, pool).await?;
        debug!("number_updated_files {number_updated_files}");
        let (total_keys, updated_keys) = self.get_and_process_keys(s3_bucket, pool).await?;
        debug!("total_keys {total_keys} updated_keys {updated_keys}");

        let mut number_uploaded = 0;
        let mut number_downloaded = 0;

        let mut stream =
            Box::pin(KeyItemCache::get_files(pool, s3_bucket, Some(true), None).await?);

        while let Some(mut key_item) = stream.try_next().await? {
            let local_file = local_dir.join(&key_item.s3_key);
            self.download_file(&local_file, s3_bucket, &key_item.s3_key)
                .await?;
            number_downloaded += 1;
            let metadata = fs::metadata(&local_file)?;
            let modified: i64 = metadata
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs()
                .try_into()?;
            let etag = spawn_blocking(move || get_md5sum(&local_file)).await??;
            key_item.local_etag = Some(etag);
            key_item.local_size = Some(metadata.len().try_into()?);
            key_item.local_timestamp = Some(modified);
            key_item.do_download = false;
            if key_item.s3_etag != key_item.local_etag {
                key_item.do_upload = true;
            }
            key_item.insert(pool).await?;
        }

        let mut stream =
            Box::pin(KeyItemCache::get_files(pool, s3_bucket, None, Some(true)).await?);

        while let Some(mut key_item) = stream.try_next().await? {
            let local_file = local_dir.join(&key_item.s3_key);
            if !local_file.exists() {
                key_item.do_upload = false;
                key_item.insert(pool).await?;
                continue;
            }
            let s3_etag = self
                .upload_file(&local_file, s3_bucket, &key_item.s3_key)
                .await?;
            if Some(&s3_etag) != key_item.local_etag.as_ref() {
                return Err(Error::StaticCustomError(
                    "Uploaded etag does not match local",
                ));
            }
            key_item.s3_etag = Some(s3_etag);
            key_item.s3_size = key_item.local_size;
            key_item.s3_timestamp = key_item.local_timestamp;
            number_uploaded += 1;
            key_item.do_upload = false;
            key_item.insert(pool).await?;
        }

        debug!("uploaded {number_uploaded}");

        debug!("downloaded {number_downloaded}");

        let mut msg = format_sstr!("{title} {s3_bucket} s3_bucket nkeys {total_keys}");

        if number_updated_files > 0 {
            write!(&mut msg, " updated {number_updated_files}")?;
        }
        if updated_keys > 0 {
            write!(&mut msg, " updated keys {updated_keys}")?;
        }
        if number_uploaded > 0 {
            write!(&mut msg, " uploaded {number_uploaded}")?;
        }
        if number_downloaded > 0 {
            write!(&mut msg, " downloaded {number_downloaded}")?;
        }

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
            let mut rng = rng();
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
                let etag: StackString = resp
                    .e_tag()
                    .ok_or_else(|| Error::StaticCustomError("No etag"))?
                    .into();
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
    ) -> Result<StackString, Error> {
        exponential_retry(|| async move {
            let body = ByteStream::read_from().path(local_file).build().await?;
            let etag = self
                .s3_client
                .put_object()
                .bucket(s3_bucket)
                .key(s3_key)
                .body(body)
                .send()
                .await?
                .e_tag
                .ok_or_else(|| Error::StaticCustomError("Missing etag"))?
                .trim_matches('"')
                .into();
            Ok(etag)
        })
        .await
    }
}

#[derive(FromSqlRow, Serialize, Deserialize, Debug, Clone, Default)]
pub struct KeyItemCache {
    pub s3_key: StackString,
    pub s3_bucket: StackString,
    pub s3_etag: Option<StackString>,
    pub s3_timestamp: Option<i64>,
    pub s3_size: Option<i64>,
    pub local_etag: Option<StackString>,
    pub local_timestamp: Option<i64>,
    pub local_size: Option<i64>,
    pub do_download: bool,
    pub do_upload: bool,
}

impl KeyItemCache {
    /// # Errors
    /// Return error if size is larger than `i64::MAX`
    pub fn from_keyitem(value: KeyItem, bucket: &str) -> Result<Self, Error> {
        Ok(Self {
            s3_key: value.key,
            s3_bucket: bucket.into(),
            s3_etag: Some(value.etag),
            s3_timestamp: Some(value.timestamp),
            s3_size: Some(value.size.try_into()?),
            ..Self::default()
        })
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_by_key(
        pool: &PgPool,
        s3_key: &str,
        s3_bucket: &str,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT * FROM key_item_cache
                WHERE s3_key = $s3_key
                  AND s3_bucket = $s3_bucket
            "#,
            s3_key = s3_key,
            s3_bucket = s3_bucket,
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn get_files(
        pool: &PgPool,
        s3_bucket: &str,
        do_download: Option<bool>,
        do_upload: Option<bool>,
    ) -> Result<impl Stream<Item = Result<Self, PgError>>, Error> {
        let mut bindings = vec![("s3_bucket", &s3_bucket as Parameter)];
        let mut constraints = vec![format_sstr!("s3_bucket=$s3_bucket")];
        if let Some(do_download) = &do_download {
            constraints.push(format_sstr!("do_download=$do_download"));
            bindings.push(("do_download", do_download as Parameter));
        }
        if let Some(do_upload) = &do_upload {
            constraints.push(format_sstr!("do_upload=$do_upload"));
            bindings.push(("do_upload", do_upload as Parameter));
        }
        let query = format_sstr!(
            "SELECT * FROM key_item_cache WHERE {}",
            constraints.join(" AND ")
        );
        let query = query_dyn!(&query, ..bindings)?;

        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert(&self, pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                INSERT INTO key_item_cache (
                    s3_key,
                    s3_bucket,
                    s3_etag,
                    s3_timestamp,
                    s3_size,
                    local_etag,
                    local_timestamp,
                    local_size,
                    do_download,
                    do_upload
                ) VALUES (
                    $s3_key,
                    $s3_bucket,
                    $s3_etag,
                    $s3_timestamp,
                    $s3_size,
                    $local_etag,
                    $local_timestamp,
                    $local_size,
                    $do_download,
                    $do_upload
                ) ON CONFLICT (s3_key, s3_bucket) DO UPDATE
                    SET s3_etag=$s3_etag,
                        s3_timestamp=$s3_timestamp,
                        s3_size=$s3_size,
                        local_etag=$local_etag,
                        local_timestamp=$local_timestamp,
                        local_size=$local_size,
                        do_download=$do_download,
                        do_upload=$do_upload
            "#,
            s3_key = self.s3_key,
            s3_bucket = self.s3_bucket,
            s3_etag = self.s3_etag,
            s3_timestamp = self.s3_timestamp,
            s3_size = self.s3_size,
            local_etag = self.local_etag,
            local_timestamp = self.local_timestamp,
            local_size = self.local_size,
            do_download = self.do_download,
            do_upload = self.do_upload,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }
}

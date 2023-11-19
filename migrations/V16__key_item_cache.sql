CREATE TABLE key_item_cache (
    id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
    s3_key TEXT NOT NULL,
    s3_bucket TEXT NOT NULL,
    s3_etag TEXT,
    s3_timestamp BIGINT,
    s3_size BIGINT,
    local_etag TEXT,
    local_timestamp BIGINT,
    local_size BIGINT,
    do_download BOOLEAN NOT NULL DEFAULT false,
    do_upload BOOLEAN NOT NULL DEFAULT false,
    UNIQUE(s3_key, s3_bucket)
)
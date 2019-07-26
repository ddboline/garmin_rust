CREATE TABLE IF NOT EXISTS strava_id_cache (
    strava_id text PRIMARY KEY,
    begin_datetime text NOT NULL,
    strava_title text NOT NULL
);
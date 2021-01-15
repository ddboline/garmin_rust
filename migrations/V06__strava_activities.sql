CREATE TABLE strava_activities (
    id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    start_date TIMESTAMP WITH TIME ZONE NOT NULL,
    distance DOUBLE PRECISION,
    moving_time BIGINT,
    elapsed_time BIGINT NOT NULL,
    total_elevation_gain DOUBLE PRECISION,
    elev_high DOUBLE PRECISION,
    elev_low DOUBLE PRECISION,
    activity_type TEXT NOT NULL,
    timezone TEXT NOT NULL
);

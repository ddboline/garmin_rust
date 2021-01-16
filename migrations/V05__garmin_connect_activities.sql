CREATE TABLE garmin_connect_activities (
    activity_id BIGINT PRIMARY KEY,
    activity_name TEXT,
    description TEXT,
    start_time_gmt TIMESTAMP WITH TIME ZONE NOT NULL,
    distance DOUBLE PRECISION,
    duration DOUBLE PRECISION NOT NULL,
    elapsed_duration DOUBLE PRECISION,
    moving_duration DOUBLE PRECISION,
    steps BIGINT,
    calories DOUBLE PRECISION,
    average_hr DOUBLE PRECISION,
    max_hr DOUBLE PRECISION
);

CREATE TABLE fitbit_activities (
    log_id BIGINT PRIMARY KEY,
    log_type TEXT NOT NULL,
    start_time TIMESTAMP WITH TIME ZONE NOT NULL,
    tcx_link TEXT,
    activity_type_id BIGINT,
    activity_name TEXT,
    duration BIGINT NOT NULL,
    distance DOUBLE PRECISION,
    distance_unit TEXT,
    steps BIGINT
);

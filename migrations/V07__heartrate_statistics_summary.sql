CREATE TABLE heartrate_statistics_summary (
    date DATE UNIQUE NOT NULL,
    min_heartrate DOUBLE PRECISION NOT NULL,
    max_heartrate DOUBLE PRECISION NOT NULL,
    mean_heartrate DOUBLE PRECISION NOT NULL,
    median_heartrate DOUBLE PRECISION NOT NULL,
    stdev_heartrate DOUBLE PRECISION NOT NULL,
    number_of_entries INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS race_results (
    id SERIAL PRIMARY KEY,
    race_type TEXT NOT NULL DEFAULT 'personal',
    race_date DATE,
    race_name TEXT,
    race_distance INTEGER NOT NULL,
    race_time DOUBLE PRECISION NOT NULL,
    race_flag BOOLEAN NOT NULL DEFAULT FALSE
);

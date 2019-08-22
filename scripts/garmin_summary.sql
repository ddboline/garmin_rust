CREATE TABLE IF NOT EXISTS garmin_summary (
    filename text NOT NULL PRIMARY KEY,
    begin_datetime TIMESTAMP WITH TIME ZONE NOT NULL,
    sport varchar(12) NOT NULL,
    total_calories integer,
    total_distance double precision,
    total_duration double precision,
    total_hr_dur double precision,
    total_hr_dis double precision,
    number_of_items integer,
    md5sum varchar(32)
);

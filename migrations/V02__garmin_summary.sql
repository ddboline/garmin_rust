CREATE TABLE garmin_summary (
    filename text NOT NULL PRIMARY KEY,
    begin_datetime TIMESTAMP WITH TIME ZONE NOT NULL,
    sport varchar(12) NOT NULL,
    total_calories integer NOT NULL,
    total_distance double precision NOT NULL,
    total_duration double precision NOT NULL,
    total_hr_dur double precision NOT NULL,
    total_hr_dis double precision NOT NULL,
    md5sum varchar(32) NOT NULL
);

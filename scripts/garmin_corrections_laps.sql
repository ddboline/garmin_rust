CREATE SEQUENCE garmin_corrections_laps_id_seq;

CREATE TABLE IF NOT EXISTS garmin_corrections_laps (
    id integer NOT NULL PRIMARY KEY DEFAULT nextval('garmin_corrections_laps_id_seq'::regclass),
    start_time TIMESTAMP WITH TIME ZONE NOT NULL,
    lap_number integer NOT NULL,
    distance double precision,
    duration double precision,
    sport text,
    CONSTRAINT garmin_corrections_laps_unique_key_key UNIQUE (start_time, lap_number)
);

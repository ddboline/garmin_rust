CREATE SEQUENCE garmin_corrections_laps_id_seq;

CREATE TABLE IF NOT EXISTS garmin_corrections_laps (
    id integer NOT NULL PRIMARY KEY DEFAULT nextval('garmin_corrections_laps_id_seq'::regclass),
    start_time text,
    lap_number integer,
    distance double precision,
    duration double precision,
    unique_key text,
    sport text,
    CONSTRAINT garmin_corrections_laps_unique_key_key UNIQUE (unique_key)
);

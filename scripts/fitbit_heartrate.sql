CREATE SEQUENCE fitbit_heartrate_id_seq;

CREATE TABLE IF NOT EXISTS fitbit_heartrate (
    id integer NOT NULL PRIMARY KEY DEFAULT nextval('fitbit_heartrate_id_seq'::regclass),
    datetime TIMESTAMP WITH TIME ZONE NOT NULL,
    bpm double precision,
    confidence integer
);

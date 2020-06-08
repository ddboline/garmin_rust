CREATE SEQUENCE scale_measurements_id_seq;

CREATE TABLE IF NOT EXISTS scale_measurements (
    id integer NOT NULL PRIMARY KEY DEFAULT nextval('scale_measurements_id_seq'::regclass),
    datetime TIMESTAMP WITH TIME ZONE NOT NULL,
    mass double precision NOT NULL,
    fat_pct double precision NOT NULL,
    water_pct double precision NOT NULL,
    muscle_pct double precision NOT NULL,
    bone_pct double precision NOT NULL
);

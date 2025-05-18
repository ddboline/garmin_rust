DELETE FROM scale_measurements
WHERE id in (
    SELECT id FROM (
        SELECT id,
               substring(cast(datetime at time zone 'utc' as text), 1, 19),
               row_number() OVER (
                PARTITION BY substring(cast(datetime at time zone 'utc' as text), 1, 19)
                ORDER BY datetime
               ) AS row_num
        FROM scale_measurements
    ) t
    WHERE t.row_num > 1
);

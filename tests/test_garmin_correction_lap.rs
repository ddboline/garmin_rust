extern crate tempfile;

#[cfg(test)]
mod tests {
    #[test]
    fn test_garmin_correction_lap_new() {
        let gc = garmin_rust::garmin_correction_lap::GarminCorrectionLap::new();

        assert_eq!(gc.id, -1);
        assert_eq!(gc.lap_number, -1);
        assert_eq!(gc.sport, None);
        assert_eq!(gc.distance, None);
        assert_eq!(gc.duration, None);

        let gc = garmin_rust::garmin_correction_lap::GarminCorrectionLap::new()
            .with_id(5)
            .with_lap_number(3)
            .with_sport("running")
            .with_distance(5.3)
            .with_duration(6.2);
        assert_eq!(gc.id, 5);
        assert_eq!(gc.lap_number, 3);
        assert_eq!(gc.sport, Some("running".to_string()));
        assert_eq!(gc.distance, Some(5.3));
        assert_eq!(gc.duration, Some(6.2));
    }

    #[test]
    fn test_corr_list_from_json() {
        let corr_list = garmin_rust::garmin_correction_lap::corr_list_from_json(
            "tests/data/garmin_corrections.json",
        ).unwrap();

        assert_eq!(corr_list.get(0).unwrap().distance, Some(3.10685596118667));

        let corr_val = garmin_rust::garmin_correction_lap::GarminCorrectionLap::new();
        assert_eq!(corr_val.lap_number, -1);
    }

    #[test]
    fn test_corr_list_from_buffer() {
        let json_buffer = r#"
        {
            "2011-07-04T08:58:27Z": {
            "0": 3.10685596118667
            },
            "2013-01-17T16:14:32Z": {
            "0": 0.507143,
            "1": 0.190476
            },
            "2014-08-23T10:17:14Z": {
            "0": [
            6.5,
            4099.0
            ]
            },
            "abcdefg": {"hijk": [0, 1, 2]}
        }
        "#.to_string()
            .into_bytes();

        let corr_list =
            garmin_rust::garmin_correction_lap::corr_list_from_buffer(&json_buffer).unwrap();

        let first = corr_list.get(0).unwrap();
        let second = corr_list.get(1).unwrap();
        let third = corr_list.get(2).unwrap();
        let fourth = corr_list.get(3).unwrap();
        assert_eq!(corr_list.get(4), None);

        assert_eq!(
            first,
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2011-07-04T08:58:27Z".to_string(),
                lap_number: 0,
                sport: None,
                distance: Some(3.10685596118667),
                duration: None
            }
        );
        assert_eq!(
            second,
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2013-01-17T16:14:32Z".to_string(),
                lap_number: 0,
                sport: None,
                distance: Some(0.507143),
                duration: None
            }
        );
        assert_eq!(
            third,
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2013-01-17T16:14:32Z".to_string(),
                lap_number: 1,
                sport: None,
                distance: Some(0.190476),
                duration: None
            }
        );
        assert_eq!(
            fourth,
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2014-08-23T10:17:14Z".to_string(),
                lap_number: 0,
                sport: None,
                distance: Some(6.5),
                duration: Some(4099.0)
            }
        );
    }

    #[test]
    fn test_corr_list_from_buffer_invalid() {
        let json_buffer = r#"["a", "b", "c"]"#.to_string().into_bytes();

        let corr_list =
            garmin_rust::garmin_correction_lap::corr_list_from_buffer(&json_buffer).unwrap();

        assert_eq!(corr_list.len(), 0);
    }

    #[test]
    fn test_dump_read_corr_list() {
        let json_buffer = r#"
        {
            "2011-07-04T08:58:27Z": {
            "0": 3.10685596118667
            },
            "2013-01-17T16:14:32Z": {
            "0": 0.507143,
            "1": 0.190476
            },
            "2014-08-23T10:17:14Z": {
            "0": [
            6.5,
            4099.0
            ]
            }
        }
        "#.to_string()
            .into_bytes();
        let corr_list =
            garmin_rust::garmin_correction_lap::corr_list_from_buffer(&json_buffer).unwrap();

        let tempfile = tempfile::Builder::new().suffix(".avro").tempfile().unwrap();
        let tempfilename = tempfile.path().to_str().unwrap();

        assert_eq!(
            garmin_rust::garmin_correction_lap::dump_corr_list_to_avro(&corr_list, &tempfilename)
                .unwrap(),
            ()
        );
        assert_eq!(
            garmin_rust::garmin_correction_lap::read_corr_list_from_avro(&tempfilename).unwrap(),
            corr_list
        );
    }

    #[test]
    fn test_add_mislabeled_times_to_corr_list() {
        let corr_list = vec![
            garmin_rust::garmin_correction_lap::GarminCorrectionLap::new()
                .with_start_time("2010-11-20T19:55:34Z")
                .with_distance(10.0)
                .with_lap_number(0),
            garmin_rust::garmin_correction_lap::GarminCorrectionLap::new()
                .with_start_time("2010-11-20T19:55:34Z")
                .with_distance(5.0)
                .with_lap_number(1),
        ];

        let corr_list =
            garmin_rust::garmin_correction_lap::add_mislabeled_times_to_corr_list(&corr_list);

        let corr_map = garmin_rust::garmin_correction_lap::get_corr_list_map(&corr_list);

        println!("{:?}", corr_list);

        assert_eq!(corr_list.len(), 26);

        assert_eq!(
            corr_map
                .get(&("2010-11-20T19:55:34Z".to_string(), 0))
                .unwrap(),
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2010-11-20T19:55:34Z".to_string(),
                lap_number: 0,
                sport: Some("biking".to_string()),
                distance: Some(10.0),
                duration: None
            }
        );
        assert_eq!(
            corr_map
                .get(&("2010-11-20T19:55:34Z".to_string(), 1))
                .unwrap(),
            &garmin_rust::garmin_correction_lap::GarminCorrectionLap {
                id: -1,
                start_time: "2010-11-20T19:55:34Z".to_string(),
                lap_number: 1,
                sport: Some("biking".to_string()),
                distance: Some(5.0),
                duration: None
            }
        );
    }
}

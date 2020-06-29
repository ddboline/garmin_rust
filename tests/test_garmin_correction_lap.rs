use anyhow::Error;
use std::io::{stdout, Write};

use garmin_lib::{
    common::garmin_correction_lap::GarminCorrectionLap,
    utils::{iso_8601_datetime::convert_str_to_datetime, sport_types::SportTypes},
};

#[test]
fn test_garmin_correction_lap_new() {
    let gc = GarminCorrectionLap::new();

    assert_eq!(gc.id, -1);
    assert_eq!(gc.lap_number, -1);
    assert_eq!(gc.sport, None);
    assert_eq!(gc.distance, None);
    assert_eq!(gc.duration, None);

    let gc = GarminCorrectionLap::new()
        .with_id(5)
        .with_lap_number(3)
        .with_sport(SportTypes::Running)
        .with_distance(5.3)
        .with_duration(6.2);
    assert_eq!(gc.id, 5);
    assert_eq!(gc.lap_number, 3);
    assert_eq!(gc.sport, Some(SportTypes::Running));
    assert_eq!(gc.distance, Some(5.3));
    assert_eq!(gc.duration, Some(6.2));
}

#[test]
fn test_corr_list_from_json() -> Result<(), Error> {
    let mut corr_list: Vec<_> =
        GarminCorrectionLap::corr_list_from_json("tests/data/garmin_corrections.json")
            .unwrap()
            .into_iter()
            .map(|(_, v)| v)
            .collect();

    corr_list.sort_by_key(|i| (i.start_time, i.lap_number));

    assert_eq!(corr_list.get(0).unwrap().distance, Some(3.10685596118667));

    let corr_val = GarminCorrectionLap::new();
    assert_eq!(corr_val.lap_number, -1);
    Ok(())
}

#[test]
fn test_corr_map_from_buffer() -> Result<(), Error> {
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
        "#
    .to_string()
    .into_bytes();

    let mut corr_list: Vec<_> = GarminCorrectionLap::corr_map_from_buffer(&json_buffer)
        .unwrap()
        .into_iter()
        .map(|(_, v)| v)
        .collect();

    corr_list.sort_by_key(|i| (i.start_time, i.lap_number));

    let first = corr_list.get(0).unwrap();
    let second = corr_list.get(1).unwrap();
    let third = corr_list.get(2).unwrap();
    let fourth = corr_list.get(3).unwrap();
    assert_eq!(corr_list.get(4), None);

    assert_eq!(
        first,
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2011-07-04T08:58:27Z").unwrap(),
            lap_number: 0,
            sport: None,
            distance: Some(3.10685596118667),
            duration: None
        }
    );
    assert_eq!(
        second,
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2013-01-17T16:14:32Z").unwrap(),
            lap_number: 0,
            sport: None,
            distance: Some(0.507143),
            duration: None
        }
    );
    assert_eq!(
        third,
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2013-01-17T16:14:32Z").unwrap(),
            lap_number: 1,
            sport: None,
            distance: Some(0.190476),
            duration: None
        }
    );
    assert_eq!(
        fourth,
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2014-08-23T10:17:14Z").unwrap(),
            lap_number: 0,
            sport: None,
            distance: Some(6.5),
            duration: Some(4099.0)
        }
    );
    Ok(())
}

#[test]
fn test_corr_map_from_buffer_invalid() -> Result<(), Error> {
    let json_buffer = r#"["a", "b", "c"]"#.to_string().into_bytes();

    let corr_map = GarminCorrectionLap::corr_map_from_buffer(&json_buffer).unwrap();

    assert_eq!(corr_map.len(), 0);
    Ok(())
}

#[test]
fn test_add_mislabeled_times_to_corr_list() -> Result<(), Error> {
    let mut corr_map = GarminCorrectionLap::map_from_vec(vec![
        GarminCorrectionLap::new()
            .with_start_time(convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap())
            .with_distance(10.0)
            .with_lap_number(0),
        GarminCorrectionLap::new()
            .with_start_time(convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap())
            .with_distance(5.0)
            .with_lap_number(1),
    ]);

    GarminCorrectionLap::add_mislabeled_times_to_corr_list(&mut corr_map);

    writeln!(stdout(), "{:?}", corr_map).unwrap();

    assert_eq!(corr_map.len(), 26);

    assert_eq!(
        corr_map
            .get(&(convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap(), 0))
            .unwrap(),
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap(),
            lap_number: 0,
            sport: Some(SportTypes::Biking),
            distance: Some(10.0),
            duration: None
        }
    );
    assert_eq!(
        corr_map
            .get(&(convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap(), 1))
            .unwrap(),
        &GarminCorrectionLap {
            id: -1,
            start_time: convert_str_to_datetime("2010-11-20T19:55:34Z").unwrap(),
            lap_number: 1,
            sport: Some(SportTypes::Biking),
            distance: Some(5.0),
            duration: None
        }
    );
    Ok(())
}

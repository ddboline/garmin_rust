#!/usr/bin/python3
import requests
import os


def read_config_env():
    homedir = os.environ.get('HOME', '/tmp')
    with open(f'{homedir}/setup_files/build/garmin_scripts/config.env', 'r') as f:
        for l in f:
            (key, val) = l.strip().split('=')[:2]
            os.environ[key] = val


read_config_env()

garmin_username = os.environ['GARMIN_USERNAME']
garmin_password = os.environ['GARMIN_PASSWORD']

entry_map = {
    'imdb_episodes': 'episodes',
    'imdb_ratings': 'shows',
    'movie_collection': 'collection',
    'movie_queue': 'queue'
}

dry_run = False


def sync_db():
    from_endpoint = 'https://cloud.ddboline.net'
    to_endpoint = 'https://www.ddboline.net'

    cookies0 = requests.post(f'{from_endpoint}/api/auth',
                             json={
                                 'email': garmin_username,
                                 'password': garmin_password
                             }).cookies
    cookies1 = requests.post(f'{to_endpoint}/api/auth',
                             json={
                                 'email': garmin_username,
                                 'password': garmin_password
                             }).cookies

    measurements0 = requests.get(f'{from_endpoint}/garmin/scale_measurements',
                                 cookies=cookies0).json()
    measurements1 = requests.get(f'{to_endpoint}/garmin/scale_measurements',
                                 cookies=cookies1).json()
    measurements0 = {x['datetime']: x for x in measurements0}
    measurements1 = {x['datetime']: x for x in measurements1}

    measurements = [measurements0[x] for x in (set(measurements0) - set(measurements1))]
    if len(measurements) > 0:
        print(measurements)
        requests.post(f'{to_endpoint}/garmin/scale_measurements',
                      cookies=cookies1,
                      json={'measurements': measurements})
    measurements = [measurements1[x] for x in (set(measurements1) - set(measurements0))]
    if len(measurements) > 0:
        print(measurements)
        requests.post(f'{from_endpoint}/garmin/scale_measurements',
                      cookies=cookies0,
                      json={'measurements': measurements})
    return


if __name__ == '__main__':
    sync_db()

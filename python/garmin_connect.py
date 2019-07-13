#!/usr/bin/env python3

import requests
import time
import json
import os
from dateutil.parser import parse
from dateutil.tz import tzutc
from configparser import ConfigParser

HOMEDIR = os.environ['HOME']


def get_session():
    config = ConfigParser()
    config.read(f'{HOMEDIR}/.config/garmin_rust/garmin_connect.ini')
    email = config.get('API', 'connect_email')
    password = config.get('API', 'connect_password')

    _obligatory_headers = {
        "Referer": "https://sync.tapiriik.com"
    }
    _garmin_signin_headers = {
        "origin": "https://sso.garmin.com"
    }

    data = {
        "username": email,
        "password": password,
        "_eventId": "submit",
        "embed": "true",
    }
    params = {
        "service": "https://connect.garmin.com/modern",
        "clientId": "GarminConnect",
        "gauthHost": "https://sso.garmin.com/sso",
        "consumeServiceTicket": "false",
    }

    session = requests.Session()

    preResp = session.get("https://sso.garmin.com/sso/signin", params=params)

    if preResp.status_code != 200:
        raise Exception("SSO prestart error %s %s" % (preResp.status_code, preResp.text))

    ssoResp = session.post("https://sso.garmin.com/sso/signin",
        headers=_garmin_signin_headers, params=params, data=data, allow_redirects=False)
    if ssoResp.status_code != 200 or "temporarily unavailable" in ssoResp.text:
        raise Exception("SSO error %s %s" % (ssoResp.status_code, ssoResp.text))

    if ">sendEvent('FAIL')" in ssoResp.text:
        raise Exception("Invalid login")
    if ">sendEvent('ACCOUNT_LOCKED')" in ssoResp.text:
        raise Exception("Account Locked")

    if "renewPassword" in ssoResp.text:
        raise Exception("Reset password")

    gcRedeemResp = session.get("https://connect.garmin.com/modern", allow_redirects=False)
    if gcRedeemResp.status_code != 302:
        raise Exception("GC redeem-start error %s %s" % (gcRedeemResp.status_code, gcRedeemResp.text))

    url_prefix = "https://connect.garmin.com"

    max_redirect_count = 7
    current_redirect_count = 1
    while True:
        time.sleep(2)
        url = gcRedeemResp.headers["location"]
        # Fix up relative redirects.
        if url.startswith("/"):
            url = url_prefix + url
        url_prefix = "/".join(url.split("/")[:3])
        gcRedeemResp = session.get(url, allow_redirects=False)

        if current_redirect_count >= max_redirect_count and gcRedeemResp.status_code != 200:
            raise Exception("GC redeem %d/%d error %s %s" % (current_redirect_count, max_redirect_count, gcRedeemResp.status_code, gcRedeemResp.text))
        if gcRedeemResp.status_code == 200 or gcRedeemResp.status_code == 404:
            break
        current_redirect_count += 1
        if current_redirect_count > max_redirect_count:
            break

    session.headers.update(_obligatory_headers)

    return session


def get_activities(max_timestamp):
    max_timestamp = parse(max_timestamp)

    session = get_session()

    resp = session.get("https://connect.garmin.com/modern/proxy/activitylist-service/activities/search/activities")
    
    js = resp.json()

    for entry in js:
        activity_id = entry['activityId']
        start_time_gmt = entry['startTimeGMT']
        timestamp = parse(start_time_gmt).replace(tzinfo=tzutc())

        if timestamp <= max_timestamp:
            continue

        fname = f'{HOMEDIR}/Downloads/{activity_id}.zip'

        print(fname)

        resp = session.get(f'https://connect.garmin.com/modern/proxy/download-service/files/activity/{activity_id}')

        with open(fname, 'wb') as f:
            f.write(resp.content)


if __name__ == '__main__':
    get_activities(max_timestamp=os.sys.argv[1])

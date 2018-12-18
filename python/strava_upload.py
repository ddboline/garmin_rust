#!/usr/bin/env python3
from __future__ import (absolute_import, division, print_function, unicode_literals)

import os.path
import gzip
import requests

from stravalib import Client, exc
from tempfile import NamedTemporaryFile

try:
    from ConfigParser import ConfigParser
except ImportError:
    from configparser import ConfigParser

from flask import Flask, request

app = Flask(__name__)

ACTIVITY_TYPES = ('ride', 'run', 'swim', 'workout', 'hike', 'walk', 'nordicski', 'alpineski',
                  'backcountryski', 'iceskate', 'inlineskate', 'kitesurf', 'rollerski', 'windsurf',
                  'snowboard', 'snowshoe')


def get_config():
    cp_ = ConfigParser()
    cp_.read(os.path.expanduser('~/.stravacli'))
    cat = None
    if cp_.has_section('API'):
        cid = cp_.get('API', 'CLIENT_ID')
        cs = cp_.get('API', 'CLIENT_SECRET')
        if 'access_token' in cp_.options('API'):
            cat = cp_.get('API', 'ACCESS_TOKEN')
    return cp_, cid, cs, cat


@app.route('/callback', methods=['GET'])
def strava_auth_callback():
    cp_, cid, cs, cat = get_config()

    code = request.args.get('code')

    if code is None:
        return 'No code received', 200

    client = Client()

    cat = client.exchange_code_for_token(client_id=cid, client_secret=cs, code=code)['access_token']

    print(cid, cs, cat)

    if not cp_.has_section('API'):
        cp_.add_section('API')
    cp_.set('API', 'CLIENT_ID', cid)
    cp_.set('API', 'CLIENT_SECRET', cs)
    cp_.set('API', 'ACCESS_TOKEN', cat)
    cp_.write(open(os.path.expanduser('~/.stravacli'), "w"))

    return '<title>Strava auth code received!</title>This window can be closed.', 200


@app.route('/', methods=['POST'])
def strava_endpoint():
    allowed_exts = {
        '.tcx': lambda v: '<TrainingCenterDatabase' in v[:200],
        '.gpx': lambda v: '<gpx' in v[:200],
        '.fit': lambda v: v[8:12] == '.FIT'
    }

    print(request.json.get('filename'))
    print(request.json.get('title'))
    print(request.json.get('activity_type'))

    filename = request.json['filename']
    title = request.json['title']
    activity_type = request.json['activity_type']

    description = request.json.get('description')
    is_private = request.json.get('private', False)

    assert activity_type in ACTIVITY_TYPES, 'invalid activity'

    cp_, cid, cs, cat = get_config()

    client = Client(cat)

    try:
        client.get_athlete()
    except requests.exceptions.ConnectionError:
        raise
    except Exception:
        client = Client()

        _scope = 'activity:write'
        authorize_url = client.authorization_url(
            client_id=cid, redirect_uri='https://www.ddboline.net/strava/callback', scope=_scope)

        return f'<a href="{authorize_url}" target="_blank">Link</a>', 200

    act = open(filename, 'rb')

    base, ext = os.path.splitext(act.name)
    # autodetect based on extensions
    if ext.lower() == '.gz':
        base, ext = os.path.splitext(base)
        # un-gzip it in order to parse it
        cf_ = act
    else:
        cf_ = NamedTemporaryFile(suffix='.gz')
        gzip.GzipFile(fileobj=cf_, mode='w+b').writelines(act)
    if ext.lower() not in allowed_exts:
        return "Don't know how to handle extension " \
               "{} (allowed are {}).".format(ext, ', '.join(allowed_exts)), 400

    # upload activity
    try:
        cf_.seek(0, 0)
        upstat = client.upload_activity(
            cf_,
            ext[1:] + '.gz',
            title,
            description,
            private=is_private,
            activity_type=activity_type)
        activity = upstat.wait()
        activity_id = activity.id
        duplicate = False
    except exc.ActivityUploadFailed as e:
        words = e.args[0].split()
        print(words)
        if words[-4:-1] == ['duplicate', 'of', 'activity']:
            activity_id = int(words[-1])
            duplicate = True
        else:
            raise

    uri = "http://strava.com/activities/{:d}".format(activity_id)
    # show results
    if duplicate:
        return uri, 200
    else:
        return uri, 200


if __name__ == '__main__':
    app.run(host='0.0.0.0', port=52168)

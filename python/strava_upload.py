#!/usr/bin/env python3
from __future__ import (absolute_import, division, print_function, unicode_literals)

import os.path
import gzip
import requests
import time
import json
import base64

from gevent.pywsgi import WSGIServer

from stravalib import Client, exc
from tempfile import NamedTemporaryFile

try:
    from ConfigParser import ConfigParser
except ImportError:
    from configparser import ConfigParser

from flask import Flask, request, jsonify

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
        if 'write_access_token' in cp_.options('API'):
            cat = cp_.get('API', 'WRITE_ACCESS_TOKEN')
    return cp_, cid, cs, cat


@app.route('/running', methods=['GET'])
def running():
    return 'running', 200


@app.route('/', methods=['POST'])
def strava_endpoint():
    allowed_exts = {
        '.tcx': lambda v: '<TrainingCenterDatabase' in v[:200],
        '.gpx': lambda v: '<gpx' in v[:200],
        '.fit': lambda v: v[8:12] == '.FIT'
    }

    filename = request.json['filename']
    title = request.json['title']
    activity_type = request.json['activity_type']

    description = request.json.get('description')
    is_private = request.json.get('private', False)

    assert activity_type in ACTIVITY_TYPES, 'invalid activity'

    cat = get_config()[-1]

    client = Client(cat)

    if not os.path.exists(filename):
        return "No such file %s" % filename, 400

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
        upstat = client.upload_activity(cf_,
                                        ext[1:] + '.gz',
                                        title,
                                        description,
                                        private=is_private,
                                        activity_type=activity_type)

        timeout = 10
        start = time.time()
        while upstat.activity_id is None:
            upstat.poll()
            time.sleep(1.0)
            if timeout and (time.time() - start) > timeout:
                raise exc.TimeoutExceeded()

        activity_id = upstat.activity_id
    except exc.ActivityUploadFailed as e:
        words = e.args[0].split()
        if words[-4:-1] == ['duplicate', 'of', 'activity']:
            activity_id = int(words[-1])
        else:
            raise

    uri = "http://strava.com/activities/{:d}".format(activity_id)
    # show results
    return uri, 200


if __name__ == '__main__':
    http_server = WSGIServer(('', 52168), app)
    http_server.serve_forever()

    # app.run(host='0.0.0.0', port=52168)

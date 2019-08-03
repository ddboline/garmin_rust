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
        if 'access_token' in cp_.options('API'):
            cat = cp_.get('API', 'ACCESS_TOKEN')
    return cp_, cid, cs, cat


@app.route('/running', methods=['GET'])
def running():
    return 'running', 200


@app.route('/close_window', methods=['GET'])
def close_window():
    return '<title>Close window!</title>This window can be closed.' \
    '<script language="JavaScript" type="text/javascript">window.close()</script>', 200


@app.route('/callback', methods=['GET'])
def strava_auth_callback():
    cp_, cid, cs, cat = get_config()

    code = request.args.get('code')
    state = request.args.get('state')

    if code is None:
        return 'No code received', 200

    client = Client()

    cat = client.exchange_code_for_token(client_id=cid, client_secret=cs, code=code)['access_token']

    if not cp_.has_section('API'):
        cp_.add_section('API')
    cp_.set('API', 'CLIENT_ID', cid)
    cp_.set('API', 'CLIENT_SECRET', cs)
    cp_.set('API', 'ACCESS_TOKEN', cat)
    cp_.write(open(os.path.expanduser('~/.stravacli'), "w"))

    if state:
        js = base64.b64decode(state).decode()

        return """
            <title>Strava auth code received!</title>This window can be closed.
            <script language="JavaScript" type="text/javascript">
            function processStravaData() {
                var ostr = '/strava';
                var data = JSON.stringify(%s);
                var xmlhttp = new XMLHttpRequest();
                xmlhttp.onload = function() {
                    var win = window.open(xmlhttp.responseText, '_blank');
                    win.focus()
                    window.close()
                }
                xmlhttp.open( "POST", ostr , true );
                xmlhttp.setRequestHeader("Content-Type", "application/json");
                xmlhttp.send(data);
            };
            processStravaData();
            </script>""" % js, 200
    else:
        return '<title>Strava auth code received!</title>This window can be closed.' \
            '<script language="JavaScript" type="text/javascript">window.close()</script>', 200


@app.route('/auth/<type>')
def strava_auth(type):
    domain = request.args.get('domain', 'www.ddboline.net')

    _scope = 'activity:write'
    if type == 'read':
        _scope = 'activity:read_all'
    
    _, cid, _, cat = get_config()

    client = Client(cat)

    try:
        client.get_athlete()
        if type == 'read':
            list(client.get_activities(limit=1))
    except requests.exceptions.ConnectionError:
        raise
    except Exception:
        client = Client()

        authorize_url = client.authorization_url(
            client_id=cid,
            redirect_uri=f'https://{domain}/strava/callback',
            scope=_scope,
            state=base64.b64encode(json.dumps(request.json).encode()).decode())

        return authorize_url, 200

    return f'https://{domain}/strava/close_window', 200


@app.route('/activities', methods=['GET'])
def strava_activities():

    start_date = request.args.get('start_date', None)
    end_date = request.args.get('end_date', None)

    cat = get_config()[-1]

    client = Client(cat)

    activities = {
        x.id: {'begin_datetime': x.start_date.isoformat().replace('+00:00', 'Z'), 'title': x.name}
        for x in client.get_activities(before=end_date, after=start_date)
    }

    return jsonify(activities)


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

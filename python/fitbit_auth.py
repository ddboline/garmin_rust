#!/usr/bin/env python3
from __future__ import (absolute_import, division, print_function, unicode_literals)

import os.path
import logging

from gevent.pywsgi import WSGIServer

from fitbit.api import Fitbit
from oauthlib.oauth2.rfc6749.errors import MismatchingStateError, MissingTokenError

from flask import Flask, request

app = Flask(__name__)

fitbit_client = [None]
logging.basicConfig(level=logging.DEBUG)


@app.route('/running', methods=['GET'])
def running():
    return 'running', 200


@app.route('/auth', methods=['GET'])
def fitbit_auth():
    client_id = request.args.get('id')
    client_secret = request.args.get('secret')
    redirect_uri = 'https://www.ddboline.net/fitbit/callback'

    fitbit_client[0] = Fitbit(
        client_id,
        client_secret,
        redirect_uri=redirect_uri,
        timeout=10,
    )

    url, _ = fitbit_client[0].client.authorize_token_url()
    return url, 200


@app.route('/callback', methods=['GET'])
def fitbit_auth_callback():
    code = request.args.get('code')
    # state = request.args.get('state')

    if code is None:
        return 'No code received', 200
    try:
        fitbit_client[0].client.fetch_access_token(code)
    except MissingTokenError:
        return 'Missing access token parameter.</br>Please check that you are using the correct client_secret', 200
    except MismatchingStateError:
        return 'CSRF Warning! Mismatching state', 200

    access_token = fitbit_client[0].client.session.token['access_token']
    refresh_token = fitbit_client[0].client.session.token['refresh_token']
    user_id = fitbit_client[0].client.session.token['user_id']
    with open('%s/.fitbit_tokens' % os.getenv('HOME'), 'w') as fd:
        fd.write('user_id=%s\n' % user_id)
        fd.write('access_token=%s\n' % access_token)
        fd.write('refresh_token=%s\n' % refresh_token)

    return """
        <h1>You are now authorized to access the Fitbit API!</h1>
        <br/><h3>You can close this window</h3>""", 200


if __name__ == '__main__':
    http_server = WSGIServer(('', 53277), app)
    http_server.serve_forever()

    #app.run(host='0.0.0.0', port=53277)

#!/usr/bin/python3

import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as pl
import sys
import json
import argparse
import os


def plot_graph(name=None, title=None, data=None, do_scatter=False, opts={}):
    """ graphics plotting function """

    cache_dir = opts['cache_dir']
    popts = {}
    if 'plotopt' in opts:
        popts = opts['plotopt']
    pl.clf()
    xar, yar = zip(*data)
    xar, yar = [np.array(x) for x in (xar, yar)]
    if do_scatter:
        if popts:
            pl.hist2d(xar, yar, bins=10, **popts)
        else:
            pl.hist2d(xar, yar, bins=10)
        #pl.hexbin(xar, yar, gridsize=30, **popts)
    else:
        pl.plot(xar, yar, **popts)
        xmin, xmax, ymin, ymax = pl.axis()
        xmin, ymin = [z - 0.1 * abs(z) for z in (xmin, ymin)]
        xmax, ymax = [z + 0.1 * abs(z) for z in (xmax, ymax)]
        pl.axis([xmin, xmax, ymin, ymax])
    if 'xlabel' in opts:
        pl.xlabel(opts['xlabel'], horizontalalignment='right')
    if 'ylabel' in opts:
        pl.ylabel(opts['ylabel'], verticalalignment='top')
    pl.title(title)
    os.makedirs('%s/html' % cache_dir, exist_ok=True)
    pl.savefig('%s/html/%s.png' % (cache_dir, name))
    return '%s.png' % name


def main():
    parser = argparse.ArgumentParser('Wrapper around python plotting facilities')
    parser.add_argument('-n', '--name', help='Name for plot')
    parser.add_argument('-t', '--title', help='Title for plot')
    parser.add_argument('-s', '--do_scatter', help='Do Scatter plot', action='store_true')
    parser.add_argument('-x', '--xlabel', help='X-axis label')
    parser.add_argument('-y', '--ylabel', help='Y-axis label')
    parser.add_argument('-c', '--cachedir', help='Cache directory')
    parser.add_argument('-m', '--marker', help='Marker')
    args = parser.parse_args()

    opts = {'xlabel': args.xlabel, 'ylabel': args.ylabel, 'cache_dir': args.cachedir}
    if hasattr(args, 'marker'):
        if getattr(args, 'marker'):
            opts['plotopt'] = {'marker': args.marker}

    for line in sys.stdin:
        data = json.loads(line)
        break

    try:
        result = plot_graph(name=args.name, title=args.title, do_scatter=args.do_scatter, opts=opts, data=data)
    except AttributeError as exc:
        print(args)
        raise

    print(result)


if __name__ == '__main__':
    main()

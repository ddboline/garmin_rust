version := "0.1.3"
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	cp Dockerfile.ubuntu18.04 Dockerfile
	docker build -t build_rust:ubuntu18.04 .
	rm Dockerfile

amazon:
	cp Dockerfile.amazonlinux2018.03 Dockerfile
	docker build -t build_rust:amazonlinux2018.03 .
	rm Dockerfile

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.garmin_rust

package:
	docker run --cidfile $(cidfile) build_rust:ubuntu18.04 /garmin_rust/build_deb_docker.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/garmin_rust/garmin-rust_$(version)-$(release)_amd64.deb .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

binary:
	docker run --cidfile $(cidfile) build_rust:amazonlinux2018.03 /bin/bash
	docker cp `cat $(cidfile)`:/garmin_rust/target/release/garmin_rust .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

install:
	cp target/$(build_type)/garmin_rust_proc target/$(build_type)/garmin_rust_report target/$(build_type)/garmin_rust_http /usr/bin/
	cp python/plot_graph.py /usr/bin/garmin_rust_plot_graph.py

version := "0.1.1"
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"

all:
	cp Dockerfile.ubuntu18.04 Dockerfile
	docker build -t fpg/build_rust:ubuntu18.04 .
	rm Dockerfile

amazon:
	cp Dockerfile.amazonlinux2018.03 Dockerfile
	docker build -t fpg/build_rust:amazonlinux2018.03 .
	rm Dockerfile

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.dump_normalize_avro_json

package:
	docker run --cidfile $(cidfile) fpg/build_rust:ubuntu18.04 /dump_normalize_avro_json/build_deb.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/dump_normalize_avro_json/dump-normalize-avro-json_$(version)-$(release)_amd64.deb .
	docker cp `cat $(cidfile)`:/dump_normalize_avro_json/target/release/dump_normalize_avro_json .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

binary:
	docker run --cidfile $(cidfile) fpg/build_rust:amazonlinux2018.03 /bin/bash
	docker cp `cat $(cidfile)`:/dump_normalize_avro_json/target/release/dump_normalize_avro_json .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

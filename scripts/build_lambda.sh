#!/bin/bash

. ~/.cargo/env

cargo build --release --bin bootstrap

mkdir tmp

cp target/release/bootstrap tmp/
cp -a lambda/bin lambda/lib tmp/

cd tmp/

zip ../rust.zip bootstrap bin/* lib/*

cd ../

rm -rf tmp/


#!/bin/bash

./target/release/reversi-engine --generate-book -f 2 -k 2 -s 16 -b 16_2.new.json
cp 16_2.new.json 14_4.new.json
./target/release/reversi-engine --generate-book -f 4 -k 4 -s 14 -b 14_4.new.json
cp 14_4.new.json 12_6.new.json
./target/release/reversi-engine --generate-book -f 6 -k 6 -s 12 -b 12_6.new.json

#!/bin/bash

source .env
date >> ~/logs/celestial-echo.log
./target/release/celestial-echo >> ~/logs/celestial-echo.log 2>&1

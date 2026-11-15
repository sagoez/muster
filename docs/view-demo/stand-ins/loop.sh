#!/bin/sh
# loop.sh <seconds> <line>: prints <line>, numbered and timed, every <seconds>.
every=$1
shift
n=0
while true; do
  n=$((n + 1))
  echo "$(date +%T) $* #$n"
  sleep "$every"
done

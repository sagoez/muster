#!/bin/sh
# payments.sh: settles a charge every two seconds, and exits 1 once a `crash`
# file appears in its folder.
n=0
while [ ! -e crash ]; do
  n=$((n + 1))
  [ $((n % 4)) -eq 1 ] && echo "$(date +%T) charge $(((n + 3) / 4)) settled"
  sleep 0.5
done
echo "$(date +%T) gateway timed out, giving up"
exit 1

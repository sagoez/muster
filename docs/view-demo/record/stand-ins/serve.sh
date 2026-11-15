#!/bin/sh
# serve.sh <port> [delay]: a static HTTP server on localhost <port>, after <delay> seconds.
sleep "${2:-0}"
exec python3 -m http.server --bind 127.0.0.1 "$1"

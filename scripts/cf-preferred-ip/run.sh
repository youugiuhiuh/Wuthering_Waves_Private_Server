#!/bin/sh
exec sudo -- "$(dirname "$0")/aegis-cf-preferred-ip" "$@"

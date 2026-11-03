#!/bin/sh

set -o errexit -o xtrace

readonly file="$1"

lzip -l -vvv "$file"
lzip -t -vvv "$file"
tar -tvf "$file"

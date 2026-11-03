#!/usr/bin/env bash

set -o errexit -o xtrace -o nounset

readonly root="$1"

confirm() {
  select s in "Yes" "No"; do
    case $s in
    Yes) return ;;
    No) exit ;;
    esac
  done
}

echo "File list:"
eza -l -T "$root"

echo "Delete these files?"
confirm

pushd "$root"
echo "Directory list:"
fd --type d

echo "Delete these directories?"
confirm

chmod u+w .
fd --type d --print0 | xargs -r -0 chmod u+w -v

rm -rvf ./*
popd

chmod u+w "$root"
rmdir "$root"

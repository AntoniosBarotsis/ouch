#!/usr/bin/env bash

set -e

mkdir release

ls -R artifacts -F

cd artifacts

for dir in ouch-*; do
    cp -r artifacts "$dir/completions"
    mkdir "$dir/man"
    mv "$dir"/completions/*.1 "$dir/man"
    cp ../{README.md,LICENSE,CHANGELOG.md} "$dir"

    if [[ "$dir" = *.exe ]]; then
        target=${dir%.exe}
        mv "$dir" "$target"
        zip -r "../release/$target.zip" "$target"
    else
        chmod +x "$dir/ouch"
        tar czf "../release/$dir.tar.gz" "$dir"
    fi
done

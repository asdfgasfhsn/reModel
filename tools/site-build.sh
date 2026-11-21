#!/bin/sh
# Assemble the promo website into SITE_OUT (`make site`; site/README.md).
#
#   site/            the page and its original art, copied as they are
#   assets/posters/  the game posters (original art), into posters/
#   SITE_SHOTS/      the screenshots `make site-shots` made from your ROMs,
#                    into shots/: PNGs become WebP (quality 82, at most
#                    2580 wide), except *-px.png, kept as PNG so native
#                    pixels stay sharp
#
# Nothing here is committed: SITE_OUT is under out/.
set -eu
SITE_SHOTS=${SITE_SHOTS:-out/site-shots}
SITE_OUT=${SITE_OUT:-out/site}

command -v magick >/dev/null || { echo "site-build: needs ImageMagick (magick)"; exit 1; }
[ -d "$SITE_SHOTS" ] || echo "site-build: no $SITE_SHOTS yet (run make site-shots): the page will miss its screenshots"

rm -rf "$SITE_OUT"
mkdir -p "$SITE_OUT/posters" "$SITE_OUT/shots"
# The page and its art; site/README.md documents the site and is not served.
(cd site && find . -type f ! -name README.md) | while read -r f; do
	mkdir -p "$SITE_OUT/$(dirname "$f")"
	cp "site/$f" "$SITE_OUT/$f"
done
cp assets/posters/*.webp "$SITE_OUT/posters/"
# GitHub Pages: serve files as they are (no Jekyll).
: > "$SITE_OUT/.nojekyll"

n=0
if [ -d "$SITE_SHOTS" ]; then
	for f in $(cd "$SITE_SHOTS" && find . -type f -name '*.png' | sort); do
		dir="$SITE_OUT/shots/$(dirname "$f")"
		mkdir -p "$dir"
		case "$f" in
		*-px.png) cp "$SITE_SHOTS/$f" "$dir/" ;;
		*) magick "$SITE_SHOTS/$f" -resize '2580x>' -quality 82 "$dir/$(basename "$f" .png).webp" ;;
		esac
		n=$((n + 1))
	done
fi
echo "site: $SITE_OUT ($n screenshots, $(du -sh "$SITE_OUT" | cut -f1))"

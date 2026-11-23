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
# The pages and their art; site/README.md documents the site and is not served.
# Files named _*.html are parts shared by the pages (head, header, footer): a
# page line `<!-- include _NAME.html -->` is replaced by that file, and the
# parts themselves are not served.
(cd site && find . -type f ! -name README.md ! -name '_*.html') | while read -r f; do
	mkdir -p "$SITE_OUT/$(dirname "$f")"
	case "$f" in
	*.html) awk -v dir=site '
		match($0, /<!-- include _[A-Za-z0-9-]+\.html -->/) {
			name = substr($0, RSTART + 13, RLENGTH - 17)
			while ((getline line < (dir "/" name)) > 0) print line
			close(dir "/" name)
			next
		}
		{ print }' "site/$f" >"$SITE_OUT/$f" ;;
	*) cp "site/$f" "$SITE_OUT/$f" ;;
	esac
done
if grep -l '<!-- include ' "$SITE_OUT"/*.html >/dev/null 2>&1; then
	echo "site-build: an include was left unexpanded:"; grep -n '<!-- include ' "$SITE_OUT"/*.html; exit 1
fi
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

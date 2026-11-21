#!/bin/sh
# Publish the assembled website (`make site-deploy`; site/README.md): the
# whole of SITE_OUT becomes the single commit of the gh-pages branch on
# origin, force-pushed, so old screenshots never build up in its history.
# GitHub Pages serves that branch. The commit is made in a throwaway
# repository inside SITE_OUT (under out/, never committed), with this
# checkout's git name and email. DRY_RUN=1 makes the commit but does not
# push.
set -eu
SITE_OUT=${SITE_OUT:-out/site}
BRANCH=${SITE_BRANCH:-gh-pages}

[ -f "$SITE_OUT/index.html" ] || { echo "site-deploy: no $SITE_OUT/index.html (run make site first)"; exit 1; }
[ -d "$SITE_OUT/shots" ] && [ -n "$(ls -A "$SITE_OUT/shots")" ] || { echo "site-deploy: $SITE_OUT has no screenshots (run make site-shots, then make site)"; exit 1; }

remote=$(git remote get-url origin)
name=$(git config user.name)
email=$(git config user.email)
rev=$(git rev-parse --short HEAD)

rm -rf "$SITE_OUT/.git"
git -C "$SITE_OUT" init -q -b "$BRANCH"
git -C "$SITE_OUT" add -A
git -C "$SITE_OUT" -c user.name="$name" -c user.email="$email" \
	commit -q -m "reModel website, built from $rev"
echo "site-deploy: $(git -C "$SITE_OUT" ls-files | wc -l) files, $(du -sh --exclude=.git "$SITE_OUT" | cut -f1), from $rev"
if [ -n "${DRY_RUN:-}" ]; then
	echo "site-deploy: DRY_RUN, not pushed"
else
	git -C "$SITE_OUT" push -f "$remote" "$BRANCH:$BRANCH"
fi
rm -rf "$SITE_OUT/.git"

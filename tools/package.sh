#!/bin/sh
# Platform packages around `make dist`'s builds (`make app`, `make appimage`;
# docs/usage.md, "Versions and releases"). Both take the player `make dist`
# left in DIST_OUT (it writes each build's folder name to TARGET.name).
#
#   tools/package.sh app        macOS: reModel.app, universal (lipo of the
#                               aarch64 and x86_64 builds), ad-hoc signed,
#                               in a .dmg with the README, licences and an
#                               Applications link
#   tools/package.sh appimage   Linux x86_64: an AppImage of the player
#                               (APPIMAGETOOL, APPIMAGE_RUNTIME: the pinned
#                               appimagetool and runtime the Makefile fetches)
#
# Environment: DIST_OUT (default out/dist), ICON (the 1024x1024 icon,
# default assets/reModel-icon-1024.png), ICON_SMALL (512x512, default
# assets/reModel-icon.png).
set -eu
cd "$(dirname "$0")/.."
: "${DIST_OUT:=out/dist}"
: "${ICON:=assets/reModel-icon-1024.png}"
: "${ICON_SMALL:=assets/reModel-icon.png}"
BUNDLE_ID=io.github.asdfgasfhsn.remodel

die() {
    echo "package: $*" >&2
    exit 1
}

# The folder `make dist` built for target $1.
built() {
    f=$DIST_OUT/$1.name
    [ -f "$f" ] || die "no make dist build for $1 in $DIST_OUT (make dist DIST_TARGET=$1)"
    echo "$DIST_OUT/$(cat "$f")"
}

app() {
    arm=$(built aarch64-apple-darwin)
    x86=$(built x86_64-apple-darwin)
    base=$(basename "$arm")
    name=${base%-aarch64-apple-darwin}-macos-universal
    version=$(cargo pkgid -p remodel-play | sed 's/.*[@#]//')
    stage=$DIST_OUT/$name
    rm -rf "${stage:?}" "$stage.dmg"
    app=$stage/reModel.app
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

    lipo -create -output "$app/Contents/MacOS/remodel-play" "$arm/remodel-play" "$x86/remodel-play"

    iconset=$DIST_OUT/remodel.iconset
    rm -rf "${iconset:?}"
    mkdir -p "$iconset"
    for s in 16 32 128 256 512; do
        sips -z "$s" "$s" "$ICON" --out "$iconset/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z "$d" "$d" "$ICON" --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns -o "$app/Contents/Resources/remodel.icns" "$iconset"
    rm -rf "${iconset:?}"

    cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>reModel</string>
    <key>CFBundleDisplayName</key>
    <string>reModel</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>
    <string>remodel-play</string>
    <key>CFBundleIconFile</key>
    <string>remodel</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.arcade-games</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>reModel is MIT licensed. Game software and data remain the property of their respective owners.</string>
</dict>
</plist>
EOF
    printf 'APPL????' >"$app/Contents/PkgInfo"
    # Ad-hoc: Apple silicon runs nothing unsigned, and the bundle needs a
    # signature of its own. Not notarised (docs/usage.md).
    codesign --force --sign - --timestamp=none "$app"
    codesign --verify --strict "$app"

    sed "s/(aarch64-apple-darwin)/(macOS, Apple silicon and Intel)/" "$arm/README.txt" >"$stage/README.txt"
    cp "$arm"/LICENSE* "$arm/THIRD-PARTY-NOTICES.txt" "$stage/"
    ln -s /Applications "$stage/Applications"
    hdiutil create -quiet -volname reModel -srcfolder "$stage" -ov -format UDZO "$stage.dmg"
    rm -rf "${stage:?}"
    echo "built $stage.dmg ($(du -h "$stage.dmg" | cut -f1))"
}

appimage() {
    : "${APPIMAGETOOL:?the appimagetool to use}"
    : "${APPIMAGE_RUNTIME:?the AppImage runtime to use}"
    lin=$(built x86_64-unknown-linux-gnu)
    base=$(basename "$lin")
    name=${base%-x86_64-unknown-linux-gnu}-x86_64
    dir=$DIST_OUT/$name.AppDir
    rm -rf "${dir:?}" "$DIST_OUT/$name.AppImage"
    mkdir -p "$dir/usr/bin" "$dir/usr/share/doc/remodel"
    cp "$lin/remodel-play" "$dir/usr/bin/"
    cp "$lin/README.txt" "$lin"/LICENSE* "$lin/THIRD-PARTY-NOTICES.txt" "$dir/usr/share/doc/remodel/"
    ln -s usr/bin/remodel-play "$dir/AppRun"
    cp "$ICON_SMALL" "$dir/remodel.png"
    ln -s remodel.png "$dir/.DirIcon"
    # The window's app id and class are `remodel` (crates/play, as
    # `make install-desktop` has it), so the desktop matches it to this.
    printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=reModel' \
        'GenericName=Arcade player' 'Comment=Sega Model 2 arcade games' \
        'Exec=remodel-play' 'Icon=remodel' 'Terminal=false' \
        'Categories=Game;Emulator;' 'StartupWMClass=remodel' 'StartupNotify=true' \
        >"$dir/remodel.desktop"
    # Extract-and-run: no FUSE needed to run the tool itself (CI runners).
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$APPIMAGETOOL" --no-appstream \
        --runtime-file "$APPIMAGE_RUNTIME" "$dir" "$DIST_OUT/$name.AppImage" >"$DIST_OUT/appimagetool.log" 2>&1 ||
        {
            cat "$DIST_OUT/appimagetool.log" >&2
            die "appimagetool failed"
        }
    rm -rf "${dir:?}" "$DIST_OUT/appimagetool.log"
    echo "built $DIST_OUT/$name.AppImage ($(du -h "$DIST_OUT/$name.AppImage" | cut -f1))"
}

case ${1:-} in
    app) app ;;
    appimage) appimage ;;
    *) die "usage: tools/package.sh app | appimage" ;;
esac

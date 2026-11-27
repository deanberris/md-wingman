# dmgbuild settings for md-wingman's disk image: the app on the left, an
# Applications link on the right and an arrow between them, so installing is
# one drag. scripts/make-dmg.sh passes the paths in as defines.
#
# dmgbuild writes the window layout (.DS_Store) itself, without scripting
# Finder, so this works on a headless CI runner.

import os.path

app = defines["app"]  # noqa: F821 (dmgbuild provides `defines`)
app_name = os.path.basename(app)

files = [app]
symlinks = {"Applications": "/Applications"}
# No hide_extensions: it sets Finder info on the app bundle, which breaks
# `codesign --verify --strict`. Finder hides ".app" by default anyway.

format = "UDZO"
filesystem = "HFS+"

background = defines["background"]  # noqa: F821
window_rect = ((200, 160), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False

icon_size = 112
text_size = 13
# Centres of the icons, matching the arrow in background.png.
icon_locations = {
    app_name: (165, 190),
    "Applications": (495, 190),
}

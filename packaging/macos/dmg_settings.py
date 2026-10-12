# dmgbuild settings for the Bars disk image (scripts/package-macos.sh).
# Window size and icon positions match dmg-background.png (768x576 at 1x).
import glob
import os.path

app = defines["app"]  # noqa: F821 (provided by dmgbuild)
appname = os.path.basename(app)

volume_name = "Bars"
format = "UDZO"
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}

icns = glob.glob(os.path.join(app, "Contents", "Resources", "*.icns"))
icon = icns[0] if icns else None

background = defines["background"]  # noqa: F821
window_rect = ((200, 120), (768, 576))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 128
text_size = 13
icon_locations = {
    appname: (236, 354),
    "Applications": (536, 354),
}

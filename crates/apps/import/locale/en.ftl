# Import from Another Desktop.

title = Import from Another Desktop
# $desktop is the desktop the settings came from, such as GNOME.
imported =
    { $count ->
        [one] Imported { $count } { $desktop } setting.
       *[other] Imported { $count } { $desktop } settings.
    }
# $error is the system's own message.
import-failed = Reprise couldn't import the settings: { $error }

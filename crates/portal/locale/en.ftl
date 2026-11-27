# The portal's prompts. An & marks the next letter as the access key; the Access prompts
# that take their button labels from the frontend's convention use an underscore instead.

## Buttons and boxes the prompts share
allow = &Allow
deny = &Deny
remember-choice = &Remember this choice

## Permission prompts (RemoteDesktop, Screenshot, InputCapture, GlobalShortcuts, Background, Wallpaper)
permit-unknown-app = A program
# One line per listed item beyond the first few.
permit-more = and { $count } more
permit-preview-loading = Loading the picture...
permit-preview-cannot-show = This picture cannot be shown, so it cannot be set.
# $devices is keyboard, pointer or both.
permit-remote-title = Remote control
permit-remote-prompt =
    { $devices ->
        [keyboard] { $who } wants to control your keyboard.
        [pointer] { $who } wants to control your mouse.
       *[both] { $who } wants to control your keyboard and mouse.
    }
permit-remote-note-keyboard = It will be able to type as if you did.
permit-remote-note-pointer = It will be able to click as if you did.
permit-remote-note-both = It will be able to type and click as if you did.
permit-screenshot-title = Screenshot
permit-screenshot-prompt = { $who } wants to take a picture of your screen.
permit-screenshot-note = It will get a copy of everything the screen shows.
permit-capture-title = Input capture
permit-capture-prompt =
    { $devices ->
        [keyboard] { $who } wants to capture your keyboard.
        [pointer] { $who } wants to capture your mouse.
       *[both] { $who } wants to capture your keyboard and mouse.
    }
permit-capture-note-both = At a screen edge it takes both. { $keys } ends it.
permit-capture-note-one = At a screen edge it takes it. { $keys } ends it.
permit-shortcuts-title = Keyboard shortcuts
permit-shortcuts-prompt = { $who } wants to use keyboard shortcuts.
permit-shortcuts-note = Change the keys on Reprise Settings' Keyboard page.
permit-background-title = Running in the background
permit-background-prompt = { $who } is running in the background.
permit-background-note = It has no open window. Deny closes it.
permit-wallpaper-title = Desktop background
permit-wallpaper-prompt = { $who } wants to change your desktop background.
permit-wallpaper-note = The picture goes behind the desktop icons.

## Screen and window picker (ScreenCast, Screenshot)
picker-share-title = Share your screen
picker-share-prompt = { $who } wants to see your screen or a window.
picker-share-note = Choose one to share. It will see everything shown on it.
picker-share = &Share
picker-screenshot-title = Screenshot
picker-screenshot-prompt = { $who } wants a picture of a screen or a window.
picker-screenshot-note = Choose one. It will get a copy of what it shows.
picker-take = &Take
picker-sources = Sources
picker-screens = Screens:
picker-windows = Windows:
picker-delay = &Delay:
picker-delay-units = seconds

## Account (GetUserInformation)
account-title = Your account
account-prompt-name = { $who } wants your name.
account-prompt-picture = { $who } wants your name and picture.
account-share = _Share

## DynamicLauncher
launcher-webapp-title = Install web app
launcher-webapp-prompt = { $who } wants to add a web app to the Start menu.
launcher-title = Add to Start menu
launcher-prompt = { $who } wants to add a launcher to the Start menu.
launcher-opens = It opens { $address }.
launcher-uninstall = You can uninstall it from its right-click menu in All Programs.
launcher-install = _Install

## Access prompts that name no labels of their own
access-cancel = _Cancel

## Balloon for a RemoteDesktop session started from a remembered grant
remote-notice-unknown = An application is controlling this computer
remote-notice-named = { $name } is controlling this computer
remote-notice-body = You allowed this before and ticked Remember this choice.
remote-notice-forget = Stop and forget

## Email
email-no-program = No mail program
email-no-program-body = { $who } wanted to write an email. Choose a mail program on the Default Programs page of Reprise Settings.
email-not-opened = The email did not open
email-start-failed = Reprise could not start { $name }: { $error }.

## File chooser
chooser-select-folder = &Select Folder
chooser-open = &Open
chooser-save = &Save
chooser-cannot-save-here = These files cannot be saved here.
chooser-one-name = Type one file name.
chooser-type-name = Type a file name.
chooser-replace =
    { $path } already exists.
    Do you want to replace it?
chooser-all-files = All Files

## GlobalShortcuts, as listed in the permission prompt
shortcut-no-key = no key
shortcut-line = { $what }: { $keys }

## Wallpaper
wallpaper-default-name = Wallpaper

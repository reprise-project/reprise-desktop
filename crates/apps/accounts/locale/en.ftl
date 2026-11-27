# User Accounts: the list, the property sheet and the dialogs.

## Add Account
add-account-intro =
    Type the person's name. The user name is what they
    log in with; it cannot be changed later.
add-account-full-name = &Full name:
add-account-user-name = &User name:
add-account-type = Account type
add-account-standard = &Standard user
add-account-admin = &Administrator
add-account-password-group = Password
add-account-password = &Password:
add-account-confirm = C&onfirm:
add-account-at-login = As&k for a new password at the first login
add-account-create = Create
add-account-type-name = Type a user name.
add-account-name-too-long = The user name is longer than 32 characters.
add-account-name-rules = Use small letters, digits, - and _, starting with a letter.
add-account-name-taken = That user name is taken.
add-account-full-name-chars = A full name cannot hold a colon or a comma.
add-account-type-password = Type a password for the account.
add-account-passwords-differ = The passwords do not match.

## Properties sheet
props-tab-general = General
props-tab-picture = Picture
props-tab-password = Password
props-tab-fingerprints = Fingerprints
props-title = { $name } Properties
props-no-password = This account has no password. Type a new one for it.
props-may-have-no-password = This account already has to pick a new password at its next login, and may have none until then. Type a new one for it.
props-full-name = &Full name:
props-user-name = User name:
props-account-type = Account type
props-standard = &Standard user
props-standard-desc = Can use programs and change their own settings.
props-admin = &Administrator
props-admin-desc = Can change any setting and manage other accounts.
props-picture-intro =
    Pick a picture for the account, or browse for one of your
    own. A picture that is not square is cut to its middle.
props-browse = &Browse...
props-password-intro-none =
    Your account has no password. To set one, type the
    new password twice.
props-password-intro-change =
    To change your password, type the one you use now,
    then the new one twice.
props-current-password = &Current password:
props-new-password = &New password:
props-confirm-new-password = C&onfirm new password:
props-password-intro-other =
    Set a new password for this account. Anything its old
    password unlocked, such as saved keyring passwords,
    stays locked until it is unlocked with the old one.
props-at-next-login = As&k for a new password at the next login
props-need-current = Type your current password first.
props-need-new = Type a new password.
props-new-differ = The new passwords do not match. Type them again.
props-name-chars = A full name cannot hold a colon or a comma.

## Account list
accounts-your-account = Your account
accounts-other-accounts = Other accounts
accounts-intro = Pick an account to change, or add one for someone new.
accounts-add = &Add...
accounts-remove = &Remove
accounts-properties = &Properties
accounts-reading = Reading the accounts...
accounts-caption = { $kind } - { $login }

## Remote Desktop password dialog
rdpassword-wrong-password = The password is incorrect.
rdpassword-too-short = Use 8 or more characters.
rdpassword-same-as-login = Use a password other than your login password.
rdpassword-not-remote-user = Your account is not allowed to log on remotely.
rdpassword-failed = The password could not be changed.
rdpassword-login-password = &Your login password:
rdpassword-new-password = &New Remote Desktop password:
rdpassword-confirm = &Confirm:
rdpassword-title-create = Create Remote Desktop Password
rdpassword-title-change = Change Remote Desktop Password
rdpassword-title-remove = Remove Remote Desktop Password

## Password change through passwd
passwd-has-nul = The password holds a NUL.
passwd-no-hash-method = No password hashing method is available.
passwd-hash-failed = The password could not be hashed.
passwd-cannot-change = The password cannot be changed here.
passwd-current-incorrect = The current password is not correct.
passwd-not-changed = The password was not changed.
passwd-no-terminal = No terminal for passwd: { $why }
passwd-start-failed = passwd could not start: { $why }
passwd-stopped = passwd stopped answering.

## Window and dialogs
accounts-title = User Accounts
accounts-busy = Another change is still being made. Try again once it is done.
accounts-unreadable = The accounts could not be read. Is AccountsService installed?
accounts-account-failed =
    The account { $name }:

    { $why }
accounts-add-title = Add Account
accounts-browse-title = Browse
accounts-browse-open = Open
accounts-browse-filter-pictures = Pictures (*.png;*.jpg;*.jpeg;*.bmp;*.jxl;*.avif)
accounts-browse-filter-all = All Files (*.*)
accounts-not-a-picture = '{ $name }' is not a picture Reprise can read.
accounts-delete-title = Delete Account
accounts-delete-text =
    Delete the account { $name }?
    Their files in { $home } can stay on this computer or go with the account.
accounts-delete-files = &Delete Files
accounts-keep-files = &Keep Files
accounts-delete-fingerprints-title = Delete All Fingerprints
accounts-delete-fingerprints-text = Delete all your fingerprints? You unlock with your password until you add one again.
accounts-delete-all = &Delete All
accounts-not-readable-picture = The file is not a picture Reprise can read.

## Account changes
account-administrator = Administrator
account-standard = Standard user
account-only-admin = This is the only administrator account. Make another account an administrator first.
account-logged-on = { $name } is logged on. Delete the account once they have logged off.
account-password-kept = The new password was set, but the account will not be asked for a new one at the next login.
account-unreadable = The accounts could not be read, so nothing was changed.
account-passwd-stopped = passwd stopped unexpectedly
account-expire-failed = The account { $login } could not be set to ask for a new password.{ $why }
accounts-delete-fingerprint-verb = delete the fingerprint

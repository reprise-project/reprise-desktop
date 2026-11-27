# Printers: the folder, the printer properties and the Add Printer wizard.

## Printers folder: context menu
folder-open = &Open
folder-set-default = Set as De&fault Printer
folder-resume = Resume &Printing
folder-pause = Pa&use Printing
folder-cancel-all = Ca&ncel All Documents
folder-sharing = S&haring...
folder-delete = &Delete
folder-rename = Rena&me
folder-properties = P&roperties
folder-add-printer = &Add Printer
folder-refresh = R&efresh

## Printers folder: task pane
folder-task-open = See what's printing
folder-task-set-default = Set as default printer
folder-task-resume = Resume printing
folder-task-pause = Pause printing
folder-task-cancel-all = Cancel all documents
folder-task-share = Share this printer
folder-task-rename = Rename this printer
folder-task-delete = Delete this printer
folder-task-properties = Set printer properties
folder-task-add = Add a printer
folder-tasks = Printer Tasks
folder-details = Details

## Printers folder: details
folder-cups-not-running = The printing service (CUPS) is not running
folder-cups-install = Install cups and enable cups.socket to manage printers.
folder-default-printer = Default printer
folder-detail-status = Status: { $status }
folder-detail-documents = Documents: { $count }
folder-detail-model = Model: { $model }
folder-detail-location = Location: { $location }
folder-printers = Printers
folder-no-printers = There are no printers yet. Add one to print.
folder-system-folder = System Folder

## Printers folder: rename and columns
folder-name-too-long = A printer name can be at most 127 characters long.
folder-name-characters = A printer name cannot contain any of these characters: / \ # ? ' "
folder-column-name = Name
folder-column-documents = Documents
folder-column-status = Status
folder-column-comments = Comments
folder-column-location = Location
folder-column-model = Model

## Printer properties
props-tab-general = General
props-tab-sharing = Sharing
props-tab-ports = Ports
props-tab-device = Device Settings
props-reading-features = Reading the printer's features...
props-yes = Yes
props-no = No
props-feature-color = Color: { $value }
props-feature-duplex = Double-sided: { $value }
props-feature-paper = Paper: { $size }
props-not-accepting = Not accepting documents
props-conn-ipp = Network printer (IPP)
props-conn-dnssd = Network printer (found on the network)
props-conn-socket = Network printer (raw TCP/IP)
props-conn-lpd = Network printer (LPD)
props-conn-smb = Shared printer on another computer (SMB)
props-conn-usb = USB printer
props-conn-parallel = Parallel port printer
props-conn-serial = Serial port printer
props-conn-pdf = Prints to PDF files
props-conn-file = Prints to a file
props-conn-implicit = Printer shared by another computer
props-conn-other = Other
props-location = &Location:
props-comment = &Comment:
props-model = Model:
props-features = Features
props-test-page = Print &Test Page
props-sharing-about =
    You can share this printer with other
    computers on your network.
props-no-share = Do &not share this printer
props-share = &Share this printer
props-sharing-note =
    Other computers see a shared printer only while
    CUPS shares printers (cupsctl --share-printers).
props-ports-about = Documents print to the printer at this address.
props-uri = &Address (URI):
props-connection = Connection:
props-examples-title = Examples:
props-reading-settings = Reading the printer's settings...
props-device-about =
    Documents print with these settings unless they
    ask for others.
props-paper-size = &Paper size:
props-two-sided = T&wo-sided:
props-color = C&olor:
props-quality = &Quality:
props-not-offered = Not offered by this printer
props-settings-unreadable = The settings could not be read. { $why }

## Add Printer wizard
wizard-searching = Searching for printers...
wizard-none-found = No printers were found
wizard-found =
    { $count ->
        [one] 1 printer was found
       *[other] { NUMBER($count, useGrouping: "false") } printers were found
    }
wizard-found-network = { $found } on the network.
wizard-found-local = { $found }.
wizard-found-radio = A printer &found by this computer:
wizard-search-again = &Search Again
wizard-typed-radio = The printer is not listed. Its &address:
wizard-address-hint = A host name, an IP address or a URI (socket://host:9100)
wizard-choose-title = Choose a printer
wizard-choose-subtitle = Pick a printer Reprise found, or type its address.
wizard-driver-file = Driver file: { $file }
wizard-reading-drivers = Reading the list of drivers...
wizard-no-driver = No driver matches the search.
wizard-search = Sea&rch:
wizard-have-disk = &Have Disk...
wizard-driver-title = Install printer software
wizard-driver-subtitle = Pick the printer's maker and model, or a driver file.
wizard-name = Printer &name:
wizard-location = &Location:
wizard-name-hint = Spaces in the name become underscores.
wizard-default = Set as the &default printer
wizard-test-page = Print a &test page
wizard-name-title = Name your printer
wizard-name-subtitle = Programs show this name when you print.
wizard-adding = Adding the printer...

## CUPS errors
cups-not-running = The printing service (CUPS) is not running.
cups-denied = You are not allowed to change printers. CUPS lets the members of its admin group do this: SystemGroup in /etc/cups/cups-files.conf, wheel on most systems.
cups-needs-helper = { $denied } Install cups-pk-helper to make changes with an administrator's password.
cups-not-authorized = Nothing was changed: this needs an administrator's password.
cups-refused-status = CUPS refused the request (status 0x{ $code }).
cups-unreadable-reply = CUPS sent a reply Reprise cannot read ({ $why }).
cups-test-page-unreadable = The test page could not be read ({ $path }: { $why }).
cups-no-home = There is no home folder to keep the setting in.
cups-default-not-saved = Could not save the default printer: { $why }.
cups-file-unreadable = { $path } could not be read: { $why }.
cups-case-clash = { $new } differs from { $old } only in case, which printer names ignore.
cups-name-taken = There is already a printer called { $name }.
cups-rename-busy = { $old } can be renamed once no documents are waiting or printing on it.
cups-gone = { $old } is no longer there.
cups-rename-default-failed = { $old } is now called { $new }, but { $new } could not be made the default printer.
cups-no-such-document = CUPS has no such document.

## Printer and job states, device kinds, setting choices
printer-not-accepting = Not accepting documents
printer-ready = Ready
printer-printing = Printing
printer-paused = Paused
printer-error-with = Error: { $reason }
printer-error = Error
job-waiting = Waiting
job-paused = Paused
job-printing = Printing
job-stopped = Stopped
job-deleting = Deleting
job-error = Error
job-printed = Printed
device-network-driverless = network, no driver needed
device-network = network
device-local = connected to this computer
device-label = { $name } ({ $how })
paper-size = { $name } ({ $w } x { $h } { $unit })
sides-off = Off
sides-long-edge = Long edge (standard)
sides-short-edge = Short edge (flip up)
color-mono = Black and white
color-color = Color
color-auto = Automatic
color-auto-mono = Automatic black and white
color-process-mono = Black and white (process)
quality-draft = Draft
quality-normal = Normal
quality-high = High

## Print queue
queue-untitled = Untitled
queue-pause = &Pause
queue-resume = &Resume
queue-restart = Res&tart
queue-cancel = &Cancel
queue-column-document = Document Name
queue-column-status = Status
queue-column-owner = Owner
queue-column-pages = Pages
queue-column-size = Size
queue-column-submitted = Submitted
queue-status =
    { $count ->
        [one] 1 document in queue
       *[other] { NUMBER($count, useGrouping: "false") } documents in queue
    }
queue-set-default = Set as De&fault Printer
queue-resume-printing = Resume &Printing
queue-pause-printing = Pa&use Printing
queue-cancel-all = Ca&ncel All Documents
queue-sharing = S&haring...
queue-properties = P&roperties
queue-close = &Close

## Tray
tray-title = Printers
tray-could-not-print = { $name } could not be printed
tray-could-not-print-many = { NUMBER($count, useGrouping: "false") } documents could not be printed
tray-printing = Printing { $name }
tray-pending =
    { $count ->
        [one] 1 document pending for { $user }
       *[other] { NUMBER($count, useGrouping: "false") } documents pending for { $user }
    }

## Notifications and dialogs
notify-no-subscription = CUPS made no subscription.
notify-subscription-refused = CUPS refused the subscription (status 0x{ $status }).
balloon-problem-title = Printer problem
balloon-problem = { $name } has a problem: { $status }.
balloon-problem-message = { $name }: { $message }
balloon-printed-title = Document printed
balloon-printed = { $name } was printed on { $printer }.
balloon-not-printed-title = Document not printed
balloon-not-printed = { $name } could not be printed on { $printer }.
balloon-test-page-title = Print Test Page
balloon-test-page = A test page has been sent to { $printer }.
balloon-rename-title = Rename
rename-title = Rename Printer
rename-text =
    Renaming { $old } to { $new } drops the user name and password from its address. Properties then opens on the Ports tab to type the address again.
    Do you want to rename the printer?
delete-title = Delete Printer
delete-text =
    Are you sure you want to delete the printer { $printer }?
    Documents waiting on it are cancelled.
have-disk-title = Have Disk
have-disk-open = Open
have-disk-filter-ppd = Printer Drivers (*.ppd)
have-disk-filter-all = All Files (*.*)
wizard-window-title = Add Printer Wizard

## Failure titles
fail-pause = Could not pause the printer
fail-resume = Could not resume printing
fail-cancel-all = Could not cancel the documents
fail-set-default = Could not set the default printer
fail-delete = Could not delete the printer
fail-rename = Could not rename the printer
fail-modify = Could not change the printer
fail-test-page = Could not print a test page
fail-cancel-job = Could not cancel the document
fail-pause-job = Could not pause the document
fail-resume-job = Could not resume the document
fail-restart-job = Could not restart the document
fail-add = Could not add the printer
fail-generic = Printers

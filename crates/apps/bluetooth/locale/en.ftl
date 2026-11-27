# Bluetooth Devices. An & marks the next letter as the access key.

app-title = Bluetooth Devices
wizard-title = Add Bluetooth Device Wizard

## The Devices sheet
devices-tab-devices = Devices
devices-tab-options = Options
devices-add = &Add...
devices-remove = &Remove
devices-connect = &Connect
devices-disconnect = &Disconnect
devices-properties = &Properties
devices-no-adapter = No Bluetooth adapter was found.
devices-radio-off = Bluetooth is turned off. Turn it on from the Options tab.

## Device groups in the list
group-audio = Audio
group-input = Input devices
group-phones = Phones
group-computers = Computers
group-other = Other devices

## Device kinds
kind-headphones = Headphones
kind-headset = Headset
kind-speaker = Speaker
kind-gamepad = Game controller
kind-keyboard = Keyboard
kind-mouse = Mouse
kind-phone = Phone
kind-computer = Computer
kind-other = Bluetooth device

## Device status lines; the percent is a plain number
status-connected = Connected
status-disconnected = Not connected
status-connected-battery = Connected, battery { $percent }%
status-disconnected-battery = Not connected, battery { $percent }%
# A device in the tray menu and tooltip: its alias, an em dash, its battery.
battery-row = { $alias } — { $percent }%

## The Options tab
options-radio-group = Bluetooth radio
options-power = &Turn on Bluetooth
options-discovery-group = Discovery
options-discoverable = Allow Bluetooth &devices to find this computer
options-discovery-note =
    While this is on, other devices can see this computer
    and ask to pair with it.
options-battery-group = Battery
options-battery-label = &Warn when a device's battery drops to:
options-battery-units = %
options-battery-note = For connected devices. 0 turns the warning off.
options-files-group = File transfer
options-receive = &Receive files from paired devices
options-receive-note-on =
    You are asked before each file, and accepted files
    are saved in your Downloads folder.
options-receive-note-missing =
    Receiving files needs BlueZ's OBEX service (obexd),
    which is not installed.

## The Properties sheet
props-tab-general = General
props-title = { $name } Properties
props-type = Type:
props-address = Address:
props-status = Status:
props-battery = Battery:
props-battery-value = { $percent }%

## The Add Bluetooth Device wizard
wizard-the-device = The device
wizard-welcome-title = Welcome to the Add Bluetooth Device Wizard
wizard-intro = This wizard adds a Bluetooth device to this computer: headphones, a speaker, a keyboard, a mouse, a game controller or a phone.
wizard-how = Turn the device on and make it discoverable. Its manual says how; many devices have a button to hold down until a light flashes.
wizard-ready = &My device is set up and ready to be found.
wizard-select-title = Select the Bluetooth device that you want to add.
wizard-select-subtitle = If it is not listed, make sure it is on and discoverable, then click Search Again.
wizard-search-again = &Search Again
wizard-searching = Searching for Bluetooth devices...
wizard-none-found = No devices were found.
wizard-search-finished = Search finished.
wizard-adding-title = Adding { $name }
wizard-adding-subtitle = Follow any steps below to finish adding the device.
wizard-connecting = Connecting to { $name }...
wizard-added = { $name } was added and is connected.
wizard-added-ready = It is ready to use.
wizard-added-unconnected = { $name } was added, but it could not connect: { $error }
wizard-added-unconnected-hint = Turn it on and connect it from the Bluetooth Devices window.
wizard-failed = { $name } could not be added.
wizard-failed-hint = Click Back to search again, or Finish to close this wizard.
wizard-finish-title = Completing the Add Bluetooth Device Wizard
wizard-adapter-removed = The Bluetooth adapter was removed.
wizard-radio-off = Bluetooth was turned off.

## The Pairing Request box and the wizard's pairing step
pair-title = Bluetooth Pairing Request
pair-type-pin = Type the PIN for { $name }. If you do not know it, check the device's manual; many devices use 0000.
pair-type-passkey = Type the six-digit passkey that { $name } shows.
pair-show-passkey = Type this passkey on { $name }, then press its { $enter } key:
pair-confirm-passkey = Check that { $name } shows this passkey:
pair-authorize = { $name } wants to pair with this computer.
pair-pin-label = &PIN:
pair-passkey-label = &Passkey:
pair-match = &Yes, the passkeys match
pair-differ = No, they are &different
pair-allow = &Allow it to pair
pair-deny = &Do not allow it
pair-trust = Connect to this device auto&matically

## Removing a device
remove-title = Confirm Device Removal
remove-text =
    Are you sure you want to remove { $alias }?
    To use it again, you will have to add it again.

## The prompt for a file a device sends, and its balloons
offer-title = Bluetooth File Transfer
offer-accept = &Accept
offer-decline = &Decline
offer-sender = { $sender } wants to send you a file.
offer-file = File: { $name }
offer-size = Size: { $size }
offer-destination = Accepted files are saved in your Downloads folder.
received-summary = File received
received-body = { $name } from { $sender } is in Downloads.
received-open = Open
received-show-in-folder = Show in folder
not-received-summary = File not received
not-received-body = { $name } from { $sender } was not received.

## Battery warning balloon
battery-low-summary = Low Bluetooth battery
battery-low-body = { $alias } has { $percent }% battery left.

## The notification area icon
tray-show = Show Bluetooth Devices
tray-add = Add a Bluetooth Device
tray-turn-off = Turn Bluetooth Off
tray-turn-on = Turn Bluetooth On
tray-exit = Exit
tray-no-adapter = No Bluetooth adapter
tray-off = Bluetooth is off
tray-connected =
    { $count ->
        [0] Bluetooth: no devices connected
        [one] Bluetooth: 1 device connected
       *[other] Bluetooth: { $count } devices connected
    }

## Failure boxes
failure-power-on = Could not turn Bluetooth on
failure-power-off = Could not turn Bluetooth off
failure-discovery = Could not change discovery
failure-search = Could not search for devices
failure-connect = Could not connect
failure-disconnect = Could not disconnect
failure-remove = Could not remove the device
failure-change = Could not change the device
failure-airplane-on = Could not turn airplane mode on
failure-airplane-off = Could not turn airplane mode off
failure-other = Bluetooth

## BlueZ errors in plain words
error-pairing-refused = The device did not accept the pairing.
error-pairing-cancelled = Pairing was cancelled.
error-unreachable = The device could not be reached. Make sure it is turned on and in range.
error-key-missing = The device no longer knows this computer. Remove it and add it again.
error-not-ready = Bluetooth is not ready. Make sure it is turned on.
error-busy = Bluetooth is busy with another request. Try again in a moment.
error-already-added = This device has already been added.
error-gone = The device is no longer available.
error-blocked = Bluetooth is turned off by a switch or key on this computer.

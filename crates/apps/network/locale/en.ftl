# Network Connections: the connection list, status, key and VPN dialogs, the hotspot and the tray.

## Create Hotspot
hotspot-title = Create Hotspot
hotspot-intro = Share this computer's connection with other devices over Wi-Fi.
hotspot-ssid = Network &name:
hotspot-password = &Password:
hotspot-security = Se&curity:
hotspot-security-wpa2 = WPA2 Personal
hotspot-security-wpa3 = WPA3 Personal
hotspot-band = &Band:
hotspot-band-automatic = Automatic
hotspot-band-24 = 2.4 GHz
hotspot-band-5 = 5 GHz
hotspot-hint = The password takes 8 to 63 characters.
hotspot-start = &Start
hotspot-warning = Starting the hotspot disconnects this computer from { $joined }. It stays off that network until the hotspot stops.

## Window and dialogs
network-title = Network Connections
network-status-title = Status
network-delete-title = Confirm Connection Delete
network-delete-text = Are you sure you want to delete the connection '{ $name }'?
network-import-title = Import
network-choose-file-title = Choose File
network-open = Open
network-filter-vpn = VPN Files (*.conf;*.ovpn;*.pcf)
network-filter-keys = Certificates and Keys (*.crt;*.pem;*.key)
network-filter-all = All Files (*.*)
network-import-unreadable = { $path } could not be read: { $why }.

## Import plan
import-plugin-missing =
    { $kind ->
        [openvpn] This is an { $label } file, and NetworkManager's { $label } plugin is not installed. Install networkmanager-{ $kind }, then import the file again.
       *[other] This is a { $label } file, and NetworkManager's { $label } plugin is not installed. Install networkmanager-{ $kind }, then import the file again.
    }
import-no-plugin = No VPN plugin for NetworkManager is installed. Install the plugin for your VPN (networkmanager-openvpn, for one), then import the file again.

## Wireless network chooser
chooser-hidden-network = Connect to a hidden network...
chooser-intro =
    Click an item in the list below to connect to a wireless network in range
    or to get more information.
chooser-heading = Choose a wireless network
chooser-connecting = Connecting
chooser-connected = Connected
chooser-automatic = Automatic
chooser-security-open = Unsecured wireless network
chooser-security-wpa2 = Security-enabled wireless network (WPA2)
chooser-security-wpa3 = Security-enabled wireless network (WPA3)
chooser-security-enterprise = Security-enabled wireless network (802.1X)
chooser-security-wep = This network requires configuration
chooser-connect = &Connect
chooser-disconnect = &Disconnect
chooser-tasks = Network Tasks
chooser-refresh = Refresh network list
chooser-set-up = Set up a wireless network for a home or small office
chooser-related = Related Tasks
chooser-learn = Learn about wireless networking
chooser-order = Change the order of preferred networks
chooser-advanced = Change advanced settings

## Network Connections folder
folder-type-lan = LAN or High-Speed Internet
folder-type-vpn = Virtual Private Network
folder-starting-hotspot = Starting hotspot
folder-connected = Connected
folder-disconnected = Disconnected
folder-disable = &Disable
folder-enable = &Enable
folder-status = Stat&us
folder-repair = &Repair
folder-view-wireless = &View Available Wireless Networks
folder-stop-hotspot = Stop &Hotspot
folder-create-hotspot = Create &Hotspot...
folder-disconnect = &Disconnect
folder-connect = &Connect
folder-delete = &Delete
folder-properties = P&roperties
folder-task-wireguard = Create a WireGuard connection
folder-task-import = Import a VPN connection
folder-task-hotspot = Create a hotspot
folder-tasks = Network Tasks
folder-see-also = See Also
folder-troubleshooter = Network Troubleshooter
folder-other-places = Other Places
folder-control-panel = Control Panel
folder-documents = Documents
folder-computer = Computer
folder-details = Details
folder-nm-not-running = NetworkManager is not running
folder-name = Network Connections
folder-system-folder = System Folder
folder-column-name = Name
folder-column-type = Type
folder-column-status = Status
folder-column-device = Device Name

## Connection status
status-tab-general = General
status-tab-support = Support
status-title = { $name } Status
status-elapsed-days =
    { $days ->
        [one] { NUMBER($days, useGrouping: "false") } day { $time }
       *[other] { NUMBER($days, useGrouping: "false") } days { $time }
    }
status-disabled = Disabled
status-cable-unplugged = Network cable unplugged
status-connected = Connected
status-acquiring = Acquiring network address
status-limited = Limited or no connectivity
status-not-connected = Not connected
status-dhcp = Assigned by DHCP
status-manual = Manually Configured
status-note-ok =
    Reprise did not detect problems with this
    connection. If you cannot connect, click
    Repair.
status-note-repair = Click Repair to try this connection again.
status-connection = Connection
status-status = Status:
status-duration = Duration:
status-speed = Speed:
status-signal = Signal Strength:
status-hotspot = Hotspot:
status-activity = Activity
status-sent = Sent
status-received = Received
status-bytes = Bytes:
status-per-second = Per second:
status-properties = &Properties...
status-disable = Disa&ble
status-view-wireless = &View Wireless Networks
status-connection-status = Connection status
status-address-type = Address Type:
status-ip = IP Address:
status-mask = Subnet Mask:
status-gateway = Default Gateway:
status-details = &Details...
status-repair = &Repair

## Tray menu and tooltip
tray-disable = Disable
tray-enable = Enable
tray-status = Status
tray-repair = Repair
tray-stop-hotspot = Stop Hotspot
tray-create-hotspot = Create Hotspot...
tray-view-wireless = View Available Wireless Networks
tray-open-folder = Open Network Connections
tray-exit = Exit
tray-signal-excellent = Excellent
tray-signal-very-good = Very Good
tray-signal-good = Good
tray-signal-low = Low
tray-signal-very-low = Very Low
tray-speed-gbps = { $value } Gbps
tray-speed-mbps = { $value } Mbps
tray-airplane-on = Airplane mode is on
tray-sign-in = Sign in to the network
tray-limited = Limited or no connectivity
tray-nm-not-running = NetworkManager is not running
tray-no-adapters = No network adapters
tray-disabled = Disabled
tray-cable-unplugged = A network cable is unplugged.
tray-speed-line = Speed: { $speed }
tray-status-connected = Status: Connected
tray-wifi-name = { $name } ({ $ssid })
tray-signal-line = Signal Strength: { $word }
tray-not-connected = Not connected
tray-sign-in-body = This network asks you to sign in before it lets you on the internet. Click here to open its sign-in page.
tray-balloon-connected = Connected
tray-balloon-connected-to =
    Connected to: { $ssid }
    Signal Strength: { $word }
tray-balloon-now-connected = { $name } is now connected

## Wireless network key dialog
key-title = Wireless Network Key
key-intro = The network '{ $ssid }' requires a network key (also called a WEP key or WPA key). A network key helps prevent unknown intruders from connecting to this network.
key-type-key = Type the key, and then click Connect.
key-hidden-intro =
    Type the name of a network that does not broadcast it, and choose
    the security it uses.
key-network-key = Network &key:
key-confirm = C&onfirm network key:
key-ssid = Network &name (SSID):
key-security = &Security:
key-security-open = Open
key-security-psk = WPA-PSK
key-security-sae = WPA3-SAE
key-security-enterprise = Enterprise (802.1X)
key-next = &Next >
key-connect = &Connect

## Enterprise network login dialog
eap-title = Enterprise Network Login
eap-intro = The network '{ $ssid }' asks for an 802.1X login. Type the details your network administrator gave you, and then click Connect.
eap-no-ca-warning = Not using a Certificate Authority (CA) certificate can result in connections to insecure, rogue Wi-Fi networks.
eap-domain-needed = A domain is needed so this network's server can be told apart from any other server the CA has certified.
eap-domain-partial = Enter the server's full domain, such as radius.example.org.
eap-method = &Authentication:
eap-anonymous = A&nonymous identity:
eap-ca = CA cer&tificate:
eap-system-ca = Use s&ystem CA certificates
eap-no-ca = No CA certificate is re&quired
eap-domain = &Domain:
eap-inner = &Inner authentication:
eap-user = &Username:
eap-password = &Password:
eap-cert = Us&er certificate:
eap-pkey = Private &key:
eap-pkey-password = Private key pass&word:
eap-connect = &Connect
eap-browse = Browse...

## OpenVPN connection sheet
ovpn-auth-tls = Certificates (TLS)
ovpn-auth-password = Password
ovpn-auth-password-tls = Password with certificates (TLS)
ovpn-auth-static-key = Static key
ovpn-need-name = Type a name for the connection.
ovpn-need-gateway = Type the gateway's address.
ovpn-need-user = Type the user name.
ovpn-need-ca = Choose the CA certificate file.
ovpn-need-cert = Choose your certificate file.
ovpn-need-key = Choose your private key file.
ovpn-need-static-key = Choose the static key file.

## VPN sheets
vpn-tab-wireguard = WireGuard
vpn-tab-peers = Peers
vpn-tab-general = General
vpn-tab-authentication = Authentication
vpn-keep-key-cue = Leave empty to keep the saved key
vpn-agent-cue = Asked each time; Reprise can't ask yet
vpn-keep-password-cue = Leave empty to keep the saved password
vpn-connection-name = Connection &name:
vpn-wg-title-new = New WireGuard Connection
vpn-wg-title = { $name } Properties
vpn-plugin-title = { $name } Properties
vpn-ovpn-title = { $name } Properties

## WireGuard sheet
wg-interface = &Interface:
wg-private-key = Private &key:
wg-generate = &Generate
wg-public-key = Public key:
wg-addresses = &Addresses:
wg-dns = &DNS servers:
wg-listen-port = &Listen port:
wg-port-hint = Blank picks a free port.
wg-note =
    Separate several addresses or servers with commas.
    Search domains may go among the DNS servers.
wg-peer-new = (new peer)
wg-peer-line = Peer { $number }: { $what }
wg-add = A&dd
wg-remove = &Remove
wg-peer-group = Peer
wg-peer-public-key = P&ublic key:
wg-peer-preshared = Pre&shared key:
wg-peer-endpoint = &Endpoint:
wg-peer-allowed = Allowed &IPs:
wg-peer-keepalive = Keep&alive:
wg-keepalive-hint = seconds; blank for none

## Plugin VPN sheet
plugin-type = Type:
plugin-settings = &Settings, one key = value a line:
plugin-note = Passwords and keys are kept as they are.
plugin-need-name = Type a name for the connection.

## OpenVPN sheet
ovpn-gateway = &Gateway:
ovpn-type = &Type:
ovpn-user = &User name:
ovpn-password = &Password:
ovpn-note =
    Separate several gateways with commas.
    Settings not shown here are kept as they are.
ovpn-ca = &CA certificate:
ovpn-cert = &User certificate:
ovpn-key = Private &key:
ovpn-key-password = Key &password:
ovpn-static-group = Static key
ovpn-static-file = Key &file:
ovpn-direction = &Direction:
ovpn-local-ip = &Local IP:
ovpn-remote-ip = &Remote IP:
ovpn-browse = Browse...

## WireGuard file import and checks
wg-parse-line = Line { $number }: { $what }
wg-parse-section = { $line } is not a WireGuard section.
wg-parse-expected = expected Key = Variant.
wg-parse-number = { $what } must be a number.
wg-parse-port = ListenPort must be a port number.
wg-parse-keepalive = PersistentKeepalive is too large.
wg-parse-peer-setting = { $key } is not a peer setting.
wg-parse-before-interface = a setting before [Interface].
wg-parse-interface-setting = { $key } is not an interface setting.
wg-parse-no-interface = The file has no [Interface] section.
wg-check-name = Type a name for the connection.
wg-check-iface = The interface name must be 1 to 15 characters, with no spaces, / or :.
wg-check-private-key = The private key must be a 44-character WireGuard key. Click Generate for a new one.
wg-check-address = { $address } is not an address such as 10.0.0.2/24.
wg-check-dns = { $dns } is not a name server.
wg-check-kept-key = A saved key is kept only while every key is left as it is. Type this key again too, or clear the keys you changed.
wg-check-peer-public-key = Peer { $number }'s public key must be a 44-character WireGuard key.
wg-check-peer-preshared = Peer { $number }'s preshared key must be a 44-character WireGuard key, or empty.
wg-check-peer-endpoint = Peer { $number }'s endpoint must be host:port, such as vpn.example.org:51820.
wg-check-allowed = { $network } is not a network such as 0.0.0.0/0.

## Devices and secret agents
device-wired = Local Area Connection
device-wifi = Wireless Network Connection
device-numbered = { $name } { $number }
hotspot-line = Hotspot: { $summary }
hotspot-no-devices = { $ssid } (no devices)
hotspot-clients =
    { $ssid } ({ $count ->
        [one] 1 device
       *[other] { NUMBER($count, useGrouping: "false") } devices
    })
wifi-needs-agent = This network asks for its key each time it connects, and Reprise can't ask for it yet.
vpn-needs-agent = This connection asks for its password or key each time it connects, and Reprise can't ask for it yet.

## Failures reported by the NetworkManager side
nm-no-active-connection = NetworkManager answered with no active connection.
nm-import-failed = The file could not be imported.
nm-nmcli-failed = nmcli could not be started: { $why }
nm-not-active = That connection is not active.
nm-hotspot-not-running = The hotspot is not running.
nm-no-vpn-plugin = No VPN plugin for NetworkManager is installed.
nm-key-rejected = The key for { $name } was not accepted.
nm-no-saved-secret = { $name } asked for a password or key, and none is saved.
nm-login-rejected = The server for { $name } did not accept the login.
nm-not-found = { $name } could not be found.
nm-no-address = { $name } did not give this computer an address.
nm-could-not-connect = Reprise could not connect to { $name }.

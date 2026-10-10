# A throwaway StatusNotifierItem with an icon and a ToolTip, for checking the
# tray by hand: run it on the session bus, hover the icon, Ctrl+C to remove.
import gi
gi.require_version('Gio', '2.0'); gi.require_version('GLib', '2.0')
from gi.repository import Gio, GLib
XML = """<node><interface name="org.kde.StatusNotifierItem">
<property name="Category" type="s" access="read"/><property name="Id" type="s" access="read"/>
<property name="Title" type="s" access="read"/><property name="Status" type="s" access="read"/>
<property name="IconName" type="s" access="read"/><property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
<method name="Activate"><arg type="i" name="x"/><arg type="i" name="y"/></method>
</interface></node>"""
PROPS = {'Category': GLib.Variant('s','ApplicationStatus'), 'Id': GLib.Variant('s','tt-test'),
  'Title': GLib.Variant('s','Title of the test item'), 'Status': GLib.Variant('s','Active'),
  'IconName': GLib.Variant('s','dialog-information'),
  'ToolTip': GLib.Variant('(sa(iiay)ss)', ('', [], 'Tip from the ToolTip property', ''))}
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
bus.register_object('/StatusNotifierItem', info, lambda *a: None, lambda c,s,p,i,n: PROPS[n], None)
bus.call_sync('org.kde.StatusNotifierWatcher','/StatusNotifierWatcher','org.kde.StatusNotifierWatcher',
  'RegisterStatusNotifierItem', GLib.Variant('(s)', (bus.get_unique_name(),)), None, 0, -1)
GLib.MainLoop().run()

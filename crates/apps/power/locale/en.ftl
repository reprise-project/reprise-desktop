# Power Options: the tray item, notifications and the Properties sheet.

## Window
title = Power Options Properties
error-title = Power Options

## Tabs
tab-schemes = Power Schemes
tab-alarms = Alarms
tab-meter = Power Meter
tab-advanced = Advanced

## Idle times
minutes-never = Never
minutes-one = After 1 min
minutes = After { $count } mins
hours-one = After 1 hour
hours = After { $count } hours

## Actions
action-nothing = Do nothing
action-ask = Ask me what to do
action-screen-off = Turn off the screen
action-lock = Lock computer
action-standby = Stand by
action-hibernate = Hibernate
action-turnoff = Shut down

## Power Schemes tab
schemes-intro-1 = Select the power-saving settings you want to use.
schemes-intro-2 = The second column applies while the computer runs
schemes-intro-3 = on its batteries.
schemes-group = Power schemes
settings-group = Settings
column-ac = Plugged in
column-battery = On batteries
monitor-label = Turn off &monitor:
standby-label = System &stand by:
keep-awake = &Keep Awake: no screen saver, lock or stand by
schemes-note = Screen saver and lock times are in Reprise Settings.
profile-power-saver = Power saver
profile-balanced = Balanced
profile-performance = Performance

## Alarms tab
low-group = Low battery alarm
low-check = Activate &low battery alarm when power level reaches:
low-saver = &Switch to the power saver profile at this level
critical-group = Critical battery alarm
critical-check = Activate &critical battery alarm when power level reaches:
alarm-action = &Action:
saver-note = Install power-profiles-daemon to switch to power saver.

## Power Meter tab
source-label = Current power source:
source-batteries = Batteries
source-ac = AC power
battery-group = Battery
status-label = Status:
charge-label = Charge:
remaining-label = Remaining:
status-no-battery = No battery is present
status-charging = Charging
status-discharging = Discharging
status-full = Fully charged
unknown = Unknown
remaining-to-full = { $time } to full
charge-limit = Limit charging to protect the &battery
charge-limit-stops = Limit charging to protect the &battery (stops at { $end }%)

## Advanced tab
options-group = Options
show-icon = &Always show icon on the taskbar
buttons-group = Power buttons
lid-label = When I close the lid of my portable computer:
power-button-label = When I press the power button on my computer:
sleep-button-label = When I press the sleep button on my computer:

## Tray menu and tooltip
menu-meter = Open Power Meter
menu-properties = Adjust Power Properties
menu-keep-awake = Keep Awake
menu-exit = Exit
tip-awake = Keep Awake is on
tip-ac = On AC power
tip-ac-percent = On AC power ({ $percent }%)
tip-full = Fully charged ({ $percent }%)
tip-charging = Charging ({ $percent }%)
tip-remaining = { $percent }% remaining
tip-remaining-time = { $time } remaining ({ $percent }%)
tip-held = { $state } ({ $percent }%)
limited = Charging limited
limited-to = Charging limited to { $end }%

## Notifications
low-battery-title = Low battery
low-battery-text = { $pct }% of battery power remains. Plug the computer in.
low-battery-saver = { $pct }% of battery power remains. Plug the computer in. The power saver profile is on until then.
critical-battery-title = Critical battery
critical-battery-text =
    { $pct }% of battery power remains. The computer is about to { $action ->
        [standby] stand by
        [hibernate] hibernate
        [turnoff] shut down
       *[other] keep running
    }.
saver-on-title = Power saver on
saver-on-text = The battery is low, so the power saver profile is on until the computer is plugged in.

## Errors
write-failed = { $path } could not be written.
awake-no-bus = Keep Awake cannot reach the session bus: { $error }
awake-failed = Reprise could not keep the computer awake: { $error }
charge-failed =
    The charge limit could not be changed.

    { $error }

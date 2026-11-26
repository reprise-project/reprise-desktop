# Scheduled Tasks. An & marks the next letter as the access key.

name = Scheduled Tasks

## The column headers
column-name = Name
column-schedule = Schedule
column-next = Next Run
column-last = Last Run
column-status = Status

## The buttons
run = &Run Now
stop = &Stop
enable = &Enable
disable = &Disable
properties = &Properties
view-output = View &Output
new-task = &New Task...
delete = De&lete

## A task's shortcut menu
menu-run = &Run Now
menu-stop = &Stop
menu-enable = &Enable
menu-disable = &Disable
menu-output = View &Output
menu-delete = De&lete
menu-properties = &Properties

## The line over the list
status-count = { $count ->
    [one] 1 task
   *[other] { $count } tasks
}
status-none = No scheduled tasks.
# $why is the error the session bus or the service manager gave.
status-unanswered = The service manager did not list the tasks: { $why }

## A task's status
status-ready = Ready
status-running = Running
status-disabled = Disabled
status-failed = Failed
status-stopped = Stopped

## A schedule in words; $time is a clock time, $days weekday names
schedule-daily = Every day at { $time }
schedule-days = Every { $days } at { $time }
schedule-monthly = Day { $day } of every month at { $time }
# $date is a weekday and date in the locale's long form.
schedule-once = Once, { $date } at { $time }
schedule-hourly = Every hour
schedule-hourly-at = Every hour at :{ $minute }
schedule-minutely = Every minute
days-range = { $first } to { $last }
after-boot = { $span } after startup
after-startup = { $span } after logging on
after-start = { $span } after the timer starts
every = Every { $span }
after-finish = { $span } after each run finishes

## Spans of time
span-days = { $n ->
    [one] 1 day
   *[other] { $n } days
}
span-hours = { $n } h
span-minutes = { $n } min
span-seconds = { $n } s

## Runs
when-never = Never
result-not-run = Not run yet
result-success = Succeeded
result-exit = Exited with status { $status }
result-signal = Ended by a signal
result-timeout = Timed out

## What the service manager refused; $unit is a unit name, $why its message
refused-run = { $unit } did not start: { $why }
refused-stop = { $unit } did not stop: { $why }
refused-create = The task was not made: { $why }
refused-delete = { $unit } was not deleted: { $why }
refused-enable = { $unit } was not enabled: { $why }
refused-disable = { $unit } was not disabled: { $why }

## Properties
props-title = { $name } Properties
props-general = General
props-schedule = Schedule:
props-settings = Timer:
props-next = Next run:
props-last = Last run:
props-result = Last result:
props-status = Status:
props-runs = Runs:
props-command = Command:
props-file = Unit file:

## View Output; $name is the task's name
output-title = { $name } Output
output-reading = Reading the journal...
output-none = The journal holds no lines for this task yet.
# $why is what journalctl said.
output-unread = The journal could not be read: { $why }
output-refresh = Re&fresh
output-close = Close

## New Task
new-title = New Task
new-name = &Name:
new-program = &Program:
new-browse = &Browse...
new-args = &Arguments:
new-kind = &Run:
new-day = &Day:
new-date = Da&te:
new-time = T&ime:
new-setting = Timer:
new-create = &Create
new-cancel = Cancel
kind-daily = Daily
kind-weekly = Weekly
kind-monthly = Monthly
kind-once = Once
kind-logon = At log on
new-no-name = Type a name for the task.
new-no-program = Type the program to run, or Browse for it.
# $program is what was typed.
new-no-such-program = Cannot find the program { $program }.
new-quote = An argument opens a quote it never closes, or ends in \.
new-control = The name, program and arguments cannot hold control characters.
new-bad-date = Type the date as 2026-12-31.
new-past = That time has passed.
new-logon-now = The task also runs once now, when it is made.
browse-title = Browse
browse-open = Open
browse-all = All Files (*.*)

## Delete; $name is the task's name, $unit its timer
delete-title = Delete Task
delete-text = Delete the task { $name }? Its timer and service ({ $unit }) are stopped and their files removed.
delete-yes = &Delete
delete-no = Cancel

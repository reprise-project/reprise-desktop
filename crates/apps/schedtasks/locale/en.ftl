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
enable = &Enable
disable = &Disable
properties = P&roperties
new-task = &New Task...

## A task's shortcut menu
menu-run = &Run Now
menu-enable = &Enable
menu-disable = &Disable
menu-properties = P&roperties

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

## A schedule in words; $time is a clock time, $days weekday names
schedule-daily = Every day at { $time }
schedule-days = Every { $days } at { $time }
schedule-monthly = Day { $day } of every month at { $time }
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

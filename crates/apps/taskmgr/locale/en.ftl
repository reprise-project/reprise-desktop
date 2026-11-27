# Task Manager: the three pages, the Performance panes and their dialogs.

## Window
title = Task Manager
warning-title = Task Manager Warning
select-columns-title = Select Columns

## Tabs and columns
tab-applications = Applications
tab-processes = Processes
tab-performance = Performance
column-task = Task
column-status = Status
status-running = Running
status-not-responding = Not Responding

## Menus
menu-file = &File
menu-options = &Options
menu-view = &View
menu-exit = E&xit Task Manager
menu-always-on-top = &Always On Top
menu-refresh = &Refresh Now
menu-update-speed = &Update Speed
menu-cpu-history = &CPU History
menu-select-columns = &Select Columns...
speed-high = &High
speed-normal = &Normal
speed-low = &Low
speed-paused = &Paused
history-one-graph = &One Graph, All CPUs
history-per-cpu = One Graph &Per CPU

## Page buttons and status bar
end-task = &End Task
switch-to = &Switch To
all-users = &Show processes from all users
end-process = &End Process
status-processes = Processes: { $count }
status-cpu = CPU Usage: { $percent }%
status-memory = Memory: { $percent }%

## Processes columns
col-name = Image Name
col-pid = PID
col-user = User Name
col-cpu = CPU
col-memory = Mem Usage
col-threads = Threads
col-disk = Disk
col-gpu = GPU
col-cmdline = Command Line

## Select Columns dialog
intro = Pick the columns the Processes page shows.
pick-name = &Image Name
pick-pid = &PID (Process Identifier)
pick-user = &User Name
pick-cpu = &CPU Usage
pick-memory = &Memory Usage
pick-threads = &Thread Count
pick-disk = &Disk Read and Write
pick-gpu = &GPU Usage
pick-cmdline = Command &Line

## End Process and End Program boxes
end-process-text =
    Ending { $name } (PID { $pid }) can lose the data it holds, and a program that needs it may stop working.

    Are you sure you want to end this process?
end-now-title = End Program - { $name }
end-now-asked =
    { $name } is not responding to the request to end.

    To end it now, click End Now. Anything it has not saved is lost.
end-now-hung =
    { $name } is not responding.

    To end it now, click End Now. Anything it has not saved is lost.
end-now = &End Now

## Performance device list
device-cpu = CPU
device-memory = Memory
device-disk = Disk { $index } ({ $name })
device-net = { $kind } ({ $name })
device-gpu = GPU { $index }
device-sensors = Sensors
gpu-asleep = Asleep
gpu-no-data = No usage data
sensors-none = None
kind-ethernet = Ethernet
kind-wifi = Wi-Fi
kind-other = Network
gpu-named = { $driver } GPU
cpu-fallback = CPU

## Performance panes
installed = { $size } installed
adapter-heading = { $kind } adapter { $name }
scaled-title = { $title } (scale { $scale })
gpu-note-asleep = The GPU is asleep; it is not read, so it stays asleep.
gpu-note-no-data = No usage data.
gpu-note-own-processes = Usage counts only your own processes.
sensor = Sensor
sensor-value = Value
sensors-asleep = Not read while the device sleeps
sensors-heading-none = This computer reports no sensors.
sensors-heading =
    { $count ->
        [one] { $count } sensor chip
       *[other] { $count } sensor chips
    }

## Pane figures
figure-procs = Processes
figure-threads = Threads
figure-programs = Programs
figure-cores = Cores
figure-cpu-temp = Temperature
figure-cpu-fan = Fan
figure-mem-total = Total
figure-mem-avail = Available
figure-mem-cached = Cached
figure-swap-total = Total
figure-swap-used = In use
figure-swap-free = Free
figure-disk-active = Active time
figure-disk-read = Read speed
figure-disk-write = Write speed
figure-disk-size = Capacity
figure-disk-read-total = Read
figure-disk-written = Written
figure-net-down = Receive
figure-net-up = Send
figure-net-kind = Type
figure-net-rx = Received
figure-net-tx = Sent
figure-net-name = Adapter
figure-gpu-busy = Usage
figure-gpu-temp = Temperature
figure-gpu-fan = Fan
figure-vram-total = Total
figure-vram-used = In use
figure-vram-free = Free

## Pane meters and graphs
meter-cpu-meter = CPU Usage
graph-cpu-history = CPU Usage History
meter-mem-meter = Memory
graph-mem-history = Memory Usage History
graph-disk-active-history = Active Time
graph-disk-read-history = Read
graph-disk-write-history = Write
graph-net-down-history = Receive
graph-net-up-history = Send
meter-gpu-meter = GPU Usage
graph-gpu-history = GPU Usage History
meter-vram-meter = Memory
graph-vram-history = Video Memory History

## Pane groups
group-totals = Totals
group-processor = Processor
group-physical = Physical Memory (MB)
group-swap = Swap (MB)
group-activity = Activity
group-disk-totals = Since Startup
group-throughput = Throughput
group-net-totals = Since Connected
group-gpu = GPU
group-vram = Video Memory (MB)

# repriseapp's strings: the About boxes.

## A program's About box
about-version = Version { $version }
# build is the package's build id, kernel the running kernel's release.
about-version-build = Version { $version } (Build { $build } : { $kernel })
about-video = Video:  { $driver }
about-free-space = Free space on /:  { $size }
about-memory-label = Physical memory available to Reprise:

## About Reprise: the label of each group, then its lines
system-reprise = Reprise:
system-os = Operating system:
system-computer = Computer:
system-processor = Processor:
system-graphics = Graphics:
system-memory = Memory:
system-disk = Disk (/):
system-displays =
    { $count ->
        [one] Display:
       *[other] Displays:
    }
system-session = Session:
system-version = Version { $version }
system-version-build = Version { $version } (build { $build })
system-interface = { $interface } interface
system-renderer = { $renderer } renderer
system-kernel-arch = Linux { $kernel } ({ $arch })
system-kernel = Linux { $kernel }
system-memory-available = { $memory } available
system-disk-free = { $total } ({ $free } free)
system-user-login = { $user } ({ $login })
system-uptime = Up { $duration }
cpu-cores =
    { $count ->
        [one] { $count } core
       *[other] { $count } cores
    }
cpu-threads =
    { $count ->
        [one] { $count } thread
       *[other] { $count } threads
    }
cpu-speed = up to { NUMBER($ghz, minimumFractionDigits: 2, maximumFractionDigits: 2) } GHz
uptime-days =
    { $count ->
        [one] { $count } day
       *[other] { $count } days
    }
uptime-hours =
    { $count ->
        [one] { $count } hour
       *[other] { $count } hours
    }
uptime-minutes =
    { $count ->
        [one] { $count } minute
       *[other] { $count } minutes
    }
uptime-under-a-minute = less than a minute
# width and height are plain digits.
display-mode = { $width }x{ $height }
display-refresh = { $mode } at { $hz } Hz
display-scale = { $mode }, { $percent }%
display-named = { $mode } ({ $name })

## Sizes
size-kb = { NUMBER($kb) } KB
size-gb = { NUMBER($gb, minimumFractionDigits: 1, maximumFractionDigits: 1) } GB

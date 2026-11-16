## Dates. long-date is a strftime pattern written in the LC_TIME locale's day and month names;
## $order is the order its short date puts day, month and year in (dmy, mdy, ymd).
long-date =
    { $order ->
        [dmy] %A %-d %B %Y
        [ymd] %A, %x
       *[mdy] %A, %B %-d, %Y
    }
## A file's date in Properties, in place of the weekday.
date-today = Today
date-yesterday = Yesterday

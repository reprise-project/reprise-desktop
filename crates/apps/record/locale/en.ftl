# Screen recorder: the reasons a recording failed or stopped, shown in the notice.

## Start
share-failed = The screen could not be shared: { $error }
screen-not-shared = The screen { $name } is not shared.
encoder-search-failed = Finding an encoder failed.
not-a-file = Descriptor { $fd } is not a file to write the recording into.
no-video-encoder = No { $codec } encoder works here.
no-audio-encoder = No { $codec } encoder works here.

## While recording
no-picture = No picture of the screen came.
capture-ended = PipeWire ended the capture.
microphone-stopped = The microphone stopped working; the rest of the recording has only what played.
screen-changed = The screen changed size or format.
encoder-exhausted = No { $codec } encoder could code the screen.
area-outside = The area is outside the screen.

## Writing the file
write-failed = Cannot write the recording: { $error }
finish-failed = Cannot finish the recording: { $error }

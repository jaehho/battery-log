Now: the daemon, the window, and low-battery notifications (UPower's warning
level, in the daemon) are in; the log carries the history from the
shell/Python sampler and UPower before that.

Next: publish the repo and the AUR package (`battery-log-git`), then switch
dotfiles from `make install` to `paru -S battery-log-git`.

Idea: an "explain this stretch" selection — drag across the draw chart, get the
processes, backlight and profile for exactly that window. The data is already
recorded; only the brush interaction is missing.

Idea: notice recurring wake patterns. Five holes of 3659-3660 s sit in the
rows the Python sampler wrote on 2026-09-06/07 — something was waking the
machine on the hour. Both suspends the daemon has measured since are clean, so
this may already be fixed by the touchpad wakeup change in dotfiles; the app
could name the pattern instead of leaving it to be spotted by eye. Needs more
measured suspends before it is worth writing.

Watch: hardware-sleep residency measured 7% on the one suspend that has a
trustworthy reading. If that holds across more suspends it is the standby
story, and worth chasing in the kernel rather than here.

Watch: process rows are the only part of the log that grows with use rather
than with time (five rows a minute, on battery only). If it ever matters, roll
them up per day past a week.

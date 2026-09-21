# Product

## Register

product

## Users

One person: the author — technical, on Arch + Hyprland, comfortable in a
terminal, running a laptop whose battery life and suspend behaviour are both
suspect. Not a team, not a customer.

Context of use: the machine is almost always doing something else. The window
is opened when a question comes up — why did today cost so much, did last
night's suspend actually sleep, is the pack worse than last month — and closed
again. Sessions are a minute, days apart. Between them a daemon is the only
thing running, and it must cost nothing worth measuring.

## Product Purpose

battery-log records what the battery did, minute by minute, together with
enough context to explain it — screen, CPU, profile, suspends — and shows it
back as a picture you can read in one glance.

It succeeds a shell/Python sampler that wrote a good log and had no way to look
at it: first a terminal summary, then an HTML page that needed a browser, then
a TUI that felt retro and thin. The log was never the problem; the reading was.

Success looks like: open it after a bad battery day and know, without
guessing, which hour, which program, and which setting cost the charge — and
be able to prove that a suspend either slept or lied.

## Brand Personality

Precise, quiet, honest. It is an instrument: dense with numbers, calm in
colour, no celebration and no alarm. The voice states what happened and what
was measured, and says plainly when something is inferred rather than known.

Crucially: honest is not raw. It does the arithmetic (average draw, standby
watts, hardware-sleep residency) so the user does not have to — but it never
smooths a hole in the data into a line that implies it knows.

Emotional goal: the calm of an instrument you trust, that never over-claims.

## Anti-references

- **A phone battery screen:** no "app usage" percentages that hide the method,
  no coaching, no green badge for good behaviour.
- **btop / a TUI dashboard:** the terminal aesthetic is not the goal, and a
  battery is a slow story that wants a day-shaped picture, not a live meter.
- **A SaaS analytics dashboard:** no KPI tiles for their own sake, no chart
  that exists to fill a grid cell, no gradient fills or drop shadows.

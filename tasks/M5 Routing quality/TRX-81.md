---
id: TRX-81
title: A built-in fast profile that routes once
type: improvement
status: in-progress
assignee: claude
priority: medium
labels: [profiles]
estimate: 1
layer: access
---

Since [[TRX-79]] and [[TRX-80]] the default profile routes a board up to six ways and keeps the best, which takes two to three times as long and up to three times the memory. A designer who wants a quick first look needs a profile of their own for routing once, `against_layer_direction = 0`.

- [x] A failing test: `fast` is built in, and is `default` without layer directions (`the_fast_profile_is_the_default_routed_once`)
- [x] Built-in profiles other than `default` start from it, as a file's own profiles do; a file's `default` is applied before its other profiles, which start from it (`overlay`)
- [x] Measured and documented in [[Profiles]]: TokenBoard in 62 s with 356 vias instead of 156 s with 275, Sofle in 29 s with 449 instead of 70 s with 401, CNH in 11 s with 87 instead of 65 s with 66

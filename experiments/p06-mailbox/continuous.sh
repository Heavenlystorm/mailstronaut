#!/usr/bin/env bash
# Sourced only by the private Wayland harness with >100 synthetic conversations.
set -euo pipefail
: "${root:?private profile}"
trap 'printf "Continuous browsing failed at line %s; profile: %s\n" "$LINENO" "$root" >&2' ERR
check_seen
capture continuous-start
assert_empty_reader() {
  [[ $(magick "$root/continuous-start.png" "$root/$1.png" -compose difference -composite -crop 700x640+570+110 -format '%[fx:mean<0.00001]' info:) == 1 ]]
}
# Exercise wheel-triggered continuation independently of keyboard navigation.
swaymsg seat seat0 cursor set 400 700 > /dev/null
wlrctl pointer scroll 15000 0
sleep .5
wlrctl pointer scroll 15000 0
sleep .5
capture continuous-wheel-bottom
check_seen
click 400 730
copy_body continuous-wheel-last 'Synthetic cached letter 105.'
check_seen page-105
# Reset the scope, so the keyboard test also crosses a fresh 100-row boundary.
click 80 350
click 80 156
sleep .4
# Kit owns arrow navigation and virtualization. Crossing the batch boundary is
# passive, and must load the remaining rows without a page button or body open.
keys=()
for _ in {1..98}; do keys+=(-k Down -s 65); done
wtype -M ctrl l -m ctrl -s 300 "${keys[@]}" -s 800
# Give the in-flight continuation time to arrive before crossing the end of
# the first batch, where Kit would otherwise wrap selection to the first item.
keys=()
for _ in {1..10}; do keys+=(-k Down -s 100); done
wtype "${keys[@]}" -s 800
check_seen page-105
capture continuous-past-100
assert_empty_reader continuous-past-100
wtype -k Return -s 500
copy_body continuous-last 'Synthetic cached letter 105.'
check_seen page-105
wtype -M ctrl l -m ctrl -s 200
capture continuous-before-refresh
wtype -M ctrl r -m ctrl -s 700
capture continuous-after-refresh
# Exclude Kit's transient scrollbar fade; compare the actual row content/position.
[[ $(magick "$root/continuous-before-refresh.png" "$root/continuous-after-refresh.png" -compose difference -composite -crop 330x640+205+105 -format '%[fx:mean<0.00001]' info:) == 1 ]]
wtype -k Return -s 400
copy_body continuous-refreshed 'Synthetic cached letter 105.'
check_seen page-105
# Folder/search changes reset the loaded prefix and retire the old body. Neither
# switching scope nor a query result can inherit authority to mark another read.
click 80 350
sleep .3
capture continuous-folder
assert_empty_reader continuous-folder
check_seen page-105
click 80 156
wtype -M ctrl k -m ctrl 'impossible-synthetic-query-9z' -s 500
capture continuous-empty-search
assert_empty_reader continuous-empty-search
check_seen page-105
wtype -M ctrl a -m ctrl 'cached letter 105' -s 400
capture continuous-search
check_seen page-105
wtype -M ctrl a -m ctrl -k BackSpace -s 500
capture continuous-reset
assert_empty_reader continuous-reset
check_seen page-105
wtype -M ctrl -M shift d -m shift -m ctrl -s 300
capture continuous-dark
printf 'CONTINUOUS_BROWSING_OK: %s\n' "$root"

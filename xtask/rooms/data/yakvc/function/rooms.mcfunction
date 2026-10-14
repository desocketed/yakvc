# Runs on every server start. The world stays empty: build what a test needs
# with fill commands, or run `/function yakvc:build` for the prefab listening
# grounds. The area is kept loaded (at most 256 chunks may be) and the fill
# limit raised, so a room is one command.
gamerule max_block_modifications 4000000
# Nothing walks in on a listening test, and it stays day with no rain.
gamerule spawn_mobs false
gamerule spawn_patrols false
gamerule spawn_wandering_traders false
gamerule advance_time false
gamerule advance_weather false
difficulty peaceful
time set noon
weather clear
forceload add -92 -40 200 150

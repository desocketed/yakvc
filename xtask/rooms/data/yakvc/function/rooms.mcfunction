# Runs on every server start. Nobody is online yet, so the area is kept
# loaded (at most 256 chunks may be), and built once its chunks have had a
# moment to load. The fill limit is raised so each room is one command.
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
schedule function yakvc:build 2s

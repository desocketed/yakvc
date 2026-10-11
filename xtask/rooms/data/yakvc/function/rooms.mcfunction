# Runs on every server start. Nobody is online yet, so the area is kept
# loaded, and built once its chunks have had a moment to load.
forceload add -2 4 50 45
schedule function yakvc:build 2s

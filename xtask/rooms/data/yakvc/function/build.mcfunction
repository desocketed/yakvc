# Builds the rooms, in a flat world whose ground is at y -61. Stand on one
# side of a wall or room and have someone talk on the other.

# Clear the area, then build it again from scratch.
fill -2 -60 4 50 -50 45 air
setworldspawn 3 -60 6

# A row of walls along z 12, 7 wide and 5 tall: stand at z 8 and z 16.
fill 0 -60 12 6 -56 12 stone
fill 10 -60 11 16 -56 13 stone
fill 20 -60 12 26 -56 12 white_wool
fill 30 -60 12 36 -56 12 glass
fill 40 -60 12 46 -56 12 oak_leaves[persistent=true]

# A small stone room with a door, to open and close while someone talks inside.
fill 0 -60 20 8 -55 28 stone hollow
setblock 4 -59 20 oak_door[facing=north,half=lower]
setblock 4 -58 20 oak_door[facing=north,half=upper]

# A large stone hall with an open doorway: echoes, and sound coming out of the doorway.
fill 14 -60 20 34 -48 40 stone hollow
fill 23 -59 20 24 -57 20 air

# A pool, to talk underwater.
fill 40 -61 20 46 -64 26 water

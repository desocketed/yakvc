# Builds the listening grounds, in a flat world whose ground is at y -61
# (stand at y -60). Stand on one side of a wall or room and have someone talk
# on the other, or play a test sound and listen to the room around you. A
# sign at each spot says what it is. With the dev fly speed, the whole area is
# a few seconds across.

# Clear the area, then build it again from scratch.
fill -92 -60 -40 200 -20 150 air
setworldspawn 3 -60 6

# ---- Walls, x 0..46 along z 12: one of each material, 7 wide and 5 tall.
# Stand at z 8 and z 16.
fill 0 -60 12 6 -56 12 stone
fill 10 -60 11 16 -56 13 stone
fill 20 -60 12 26 -56 12 white_wool
fill 30 -60 12 36 -56 12 glass
fill 40 -60 12 46 -56 12 oak_leaves[persistent=true]
setblock 3 -60 7 oak_sign[rotation=0]{front_text:{messages:["Walls","stone 1, stone 3,","wool, glass, leaves",""]}}

# ---- A small stone room with a door, 7 by 7 inside, 4 tall.
fill 0 -60 20 8 -55 28 stone hollow
setblock 4 -59 20 oak_door[facing=north,half=lower]
setblock 4 -58 20 oak_door[facing=north,half=upper]
setblock 2 -60 18 oak_sign[rotation=0]{front_text:{messages:["Small stone room","7x7, door","",""]}}

# ---- A large stone hall, 19 by 19 inside and 11 tall, with an open doorway.
fill 14 -60 20 34 -48 40 stone hollow
fill 23 -59 20 24 -57 20 air
setblock 21 -60 18 oak_sign[rotation=0]{front_text:{messages:["Stone hall","19x19x11","open doorway",""]}}

# ---- A pool, to talk underwater.
fill 40 -61 20 46 -64 26 water
setblock 39 -60 18 oak_sign[rotation=0]{front_text:{messages:["Pool","","",""]}}

# ---- A stone cavern, 48 wide, 24 tall and 48 deep inside, with a doorway.
fill 0 -60 50 49 -35 99 stone hollow
fill 24 -59 50 25 -57 50 air
setblock 22 -60 48 oak_sign[rotation=0]{front_text:{messages:["Cavern","48x48x24","",""]}}

# ---- A canyon: two stone cliffs 30 tall and 80 long, 70 apart, under open sky.
# Stand between them at x 99 for an echo from each side.
fill 61 -60 10 63 -31 89 stone
fill 134 -60 10 136 -31 89 stone
setblock 99 -60 8 oak_sign[rotation=0]{front_text:{messages:["Canyon","cliffs 30 tall","70 apart",""]}}

# ---- A wooden house, as players build them: 9 by 7 inside, 3 tall, plank
# walls, glass panes, a door, a pitched roof, and some furniture.
fill -40 -60 20 -30 -56 28 oak_planks hollow
fill -40 -55 20 -30 -55 28 oak_planks
fill -39 -54 21 -31 -54 27 oak_planks
fill -38 -53 22 -32 -53 26 oak_planks
fill -37 -52 23 -33 -52 25 oak_planks
fill -39 -55 21 -31 -55 27 air
fill -38 -54 22 -32 -54 26 air
setblock -35 -59 20 oak_door[facing=north,half=lower]
setblock -35 -58 20 oak_door[facing=north,half=upper]
fill -38 -58 20 -37 -58 20 glass_pane
fill -33 -58 20 -32 -58 20 glass_pane
fill -40 -58 23 -40 -58 25 glass_pane
fill -30 -58 23 -30 -58 25 glass_pane
setblock -39 -59 27 red_bed[facing=south,part=foot]
setblock -39 -59 26 red_bed[facing=south,part=head]
setblock -31 -59 27 crafting_table
setblock -31 -59 26 furnace[facing=west]
setblock -36 -59 27 bookshelf
setblock -34 -59 27 bookshelf
setblock -37 -60 18 oak_sign[rotation=0]{front_text:{messages:["Wooden house","9x7, door, windows","furnished",""]}}

# ---- A village street: three small houses each side of a 5-wide gravel
# road, at z 40..70. Talk inside one and listen from the road or next door.
fill -42 -61 40 -38 -61 70 gravel
fill -50 -60 42 -44 -56 48 cobblestone hollow
fill -50 -60 52 -44 -56 58 cobblestone hollow
fill -50 -60 62 -44 -56 68 cobblestone hollow
fill -36 -60 42 -30 -56 48 spruce_planks hollow
fill -36 -60 52 -30 -56 58 spruce_planks hollow
fill -36 -60 62 -30 -56 68 spruce_planks hollow
setblock -44 -59 45 oak_door[facing=east,half=lower]
setblock -44 -58 45 oak_door[facing=east,half=upper]
setblock -44 -59 55 oak_door[facing=east,half=lower]
setblock -44 -58 55 oak_door[facing=east,half=upper]
setblock -44 -59 65 oak_door[facing=east,half=lower]
setblock -44 -58 65 oak_door[facing=east,half=upper]
setblock -36 -59 45 oak_door[facing=west,half=lower]
setblock -36 -58 45 oak_door[facing=west,half=upper]
setblock -36 -59 55 oak_door[facing=west,half=lower]
setblock -36 -58 55 oak_door[facing=west,half=upper]
setblock -36 -59 65 oak_door[facing=west,half=lower]
setblock -36 -58 65 oak_door[facing=west,half=upper]
setblock -40 -60 38 oak_sign[rotation=0]{front_text:{messages:["Village street","6 houses","5-wide road",""]}}

# ---- A cave: a winding 2-wide, 3-tall tunnel through solid stone, with a
# small chamber at the end. Enter at x -62, z 80 (the south face).
fill -90 -60 80 -55 -40 120 stone
fill -62 -59 80 -61 -57 95 air
fill -75 -59 94 -61 -57 95 air
fill -75 -59 94 -74 -57 110 air
fill -88 -59 109 -74 -57 110 air
fill -88 -59 109 -87 -57 118 air
fill -88 -59 112 -77 -52 118 air
setblock -61 -60 78 oak_sign[rotation=0]{front_text:{messages:["Cave tunnel","2 wide, winding","chamber at the end",""]}}

# ---- A warehouse: 30 by 30 inside, 8 tall, with 3 by 3 stone pillars, so
# reverb has obstacles, and a wide open front.
fill 60 -60 100 91 -51 131 stone hollow
fill 68 -59 100 83 -54 100 air
fill 66 -59 106 68 -52 108 stone
fill 76 -59 106 78 -52 108 stone
fill 86 -59 106 88 -52 108 stone
fill 66 -59 116 68 -52 118 stone
fill 76 -59 116 78 -52 118 stone
fill 86 -59 116 88 -52 118 stone
fill 66 -59 126 68 -52 128 stone
fill 76 -59 126 78 -52 128 stone
fill 86 -59 126 88 -52 128 stone
setblock 74 -60 98 oak_sign[rotation=0]{front_text:{messages:["Warehouse","30x30x8, pillars","open front",""]}}

# ---- A grove: oak trees in a loose grid, for voices through leaves without walls.
fill -2 -60 110 -2 -56 110 oak_log
fill -4 -55 108 0 -53 112 oak_leaves[persistent=true]
fill 6 -60 118 6 -56 118 oak_log
fill 4 -55 116 8 -53 120 oak_leaves[persistent=true]
fill 14 -60 110 14 -56 110 oak_log
fill 12 -55 108 16 -53 112 oak_leaves[persistent=true]
fill 22 -60 118 22 -56 118 oak_log
fill 20 -55 116 24 -53 120 oak_leaves[persistent=true]
fill 30 -60 110 30 -56 110 oak_log
fill 28 -55 108 32 -53 112 oak_leaves[persistent=true]
fill -2 -60 126 -2 -56 126 oak_log
fill -4 -55 124 0 -53 128 oak_leaves[persistent=true]
fill 6 -60 134 6 -56 134 oak_log
fill 4 -55 132 8 -53 136 oak_leaves[persistent=true]
fill 14 -60 126 14 -56 126 oak_log
fill 12 -55 124 16 -53 128 oak_leaves[persistent=true]
fill 22 -60 134 22 -56 134 oak_log
fill 20 -55 132 24 -53 136 oak_leaves[persistent=true]
fill 30 -60 126 30 -56 126 oak_log
fill 28 -55 124 32 -53 128 oak_leaves[persistent=true]
fill 38 -60 118 38 -56 118 oak_log
fill 36 -55 116 40 -53 120 oak_leaves[persistent=true]
fill 38 -60 134 38 -56 134 oak_log
fill 36 -55 132 40 -53 136 oak_leaves[persistent=true]
setblock 14 -60 106 oak_sign[rotation=0]{front_text:{messages:["Grove","oak trees","no walls",""]}}

# ---- A tower shaft: 5 by 5 inside and 36 tall, open at the top. Talk at the
# bottom and listen from above, or the other way round.
fill 100 -60 100 106 -24 106 stone hollow
fill 100 -24 100 106 -24 106 air
fill 103 -59 100 103 -57 100 air
setblock 103 -60 98 oak_sign[rotation=0]{front_text:{messages:["Tower shaft","5x5x36","open top",""]}}

# ---- A hill: a terraced stone mound 18 tall, to talk over and behind.
fill 150 -60 0 200 -59 50 stone
fill 153 -58 3 197 -57 47 stone
fill 156 -56 6 194 -55 44 stone
fill 159 -54 9 191 -53 41 stone
fill 162 -52 12 188 -51 38 stone
fill 165 -50 15 185 -49 35 stone
fill 168 -48 18 182 -47 32 stone
fill 171 -46 21 179 -45 29 stone
fill 174 -44 24 176 -43 26 stone
setblock 148 -60 25 oak_sign[rotation=4]{front_text:{messages:["Hill","18 tall, terraced","talk over it",""]}}

# ---- Open plain: beyond z 140 the flat world goes on.
setblock 20 -60 142 oak_sign[rotation=0]{front_text:{messages:["Open plain","nothing to echo off","flat all the way",""]}}
